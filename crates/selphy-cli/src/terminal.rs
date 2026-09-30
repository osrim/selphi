//! The terminal the commands talk to: the prompts, the two output streams,
//! and the progress bar. Commands take a [`Terminal`], so that a test can
//! script the answers and read what was written.
//!
//! Which stream a message goes to:
//!
//! - stdout: results and tables. The prepared lines and the summary, the
//!   `config` output, the change tables, "Saved.", "Not saved." and
//!   "Wrote …".
//! - stderr: problems and hints. Failed photos, "N failed", "No images in …",
//!   the "Not a terminal" hint, override warnings and "error: …".
//!   A dry run's `→` lines and "Dry run: nothing written." go to stdout, and
//!   its failed photos to stderr, as in a real run.
//!
//! The exit code:
//!
//! - 0: the command did what was asked. This includes "No images" and an
//!   answer of No to "Save?".
//! - 1: an error, or at least one failed photo.
//! - 2: a usage error, from clap.
//! - 130: cancelled at a prompt with Esc or Ctrl-C, as for a shell interrupt.
//!
//! Commands return errors; only `main` prints them.

use std::fmt;
use std::io::{self, IsTerminal, Write};

use anyhow::{Result, bail};
use indicatif::{ProgressBar, ProgressStyle};
use inquire::validator::Validation;
use inquire::{Confirm, CustomType, InquireError, Select};

use selphy::config::fields::Field;
use selphy::config::{ConfigFile, Loaded, Profile};
use selphy::geometry::{Edge, Orientation, Trim};

/// The answer to a prompt was Esc or Ctrl-C.
#[derive(Debug)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("cancelled; nothing saved")
    }
}

impl std::error::Error for Cancelled {}

/// The questions a command can ask.
pub trait Prompts {
    /// Yes or no.
    fn confirm(&mut self, message: &str, default: bool) -> Result<bool>;
    /// One of `options`, as its index. The cursor starts at `start`.
    fn select(
        &mut self,
        message: &str,
        options: &[String],
        start: usize,
        help: &str,
    ) -> Result<usize>;
    /// A finite number.
    fn number(&mut self, message: &str, default: f64, help: &str) -> Result<f64>;
}

/// The prompts, the two output streams, and whether a person is there to
/// answer.
pub struct Terminal {
    prompts: Box<dyn Prompts>,
    /// stdout: results and tables.
    pub out: Box<dyn Write>,
    /// stderr: problems and hints.
    pub err: Box<dyn Write>,
    /// Whether stdin is a terminal, so that the prompts can be answered.
    pub is_interactive: bool,
    show_progress: bool,
}

impl Terminal {
    /// inquire prompts, stdout and stderr.
    pub fn real() -> Terminal {
        Terminal {
            prompts: Box::new(Inquire),
            out: Box::new(io::stdout()),
            err: Box::new(io::stderr()),
            is_interactive: io::stdin().is_terminal(),
            show_progress: io::stderr().is_terminal(),
        }
    }

    pub fn confirm(&mut self, message: &str, default: bool) -> Result<bool> {
        self.prompts.confirm(message, default)
    }

    pub fn select(
        &mut self,
        message: &str,
        options: &[String],
        start: usize,
        help: &str,
    ) -> Result<usize> {
        self.prompts.select(message, options, start, help)
    }

    pub fn number(&mut self, message: &str, default: f64, help: &str) -> Result<f64> {
        self.prompts.number(message, default, help)
    }

    /// A bar of `len` steps on stderr. It is hidden when stderr is not a
    /// terminal. Print lines through its `suspend`, so that they do not break
    /// up the bar's line.
    pub fn progress(&self, len: usize) -> Result<ProgressBar> {
        if !self.show_progress {
            return Ok(ProgressBar::hidden());
        }
        let bar = ProgressBar::new(len as u64);
        bar.set_style(ProgressStyle::with_template(
            "{bar:30} {pos}/{len}  {elapsed}",
        )?);
        Ok(bar)
    }
}

/// The profile before and after new trims, with the orientation the
/// trims were measured in.
pub struct Change {
    pub before: Profile,
    pub after: Profile,
    pub orientation: Orientation,
}

/// The trims on each edge before and after, with changed ones marked.
pub fn print_changes(term: &mut Terminal, change: &Change) -> Result<()> {
    let Change {
        before,
        after,
        orientation,
    } = change;
    writeln!(
        term.out,
        "\nTrim, mm ({})   before  after",
        orientation.name()
    )?;
    for edge in Edge::ALL {
        let trim = Trim::at(*orientation, edge);
        let (old, new) = (trim.mm(before), trim.mm(after));
        let mark = if old == new { "" } else { "  *" };
        writeln!(term.out, "  {:<18}{old:>6.2}{new:>7.2}{mark}", edge.name())?;
    }
    Ok(())
}

/// Warns on stderr for each trim that the change makes while an env var
/// overrides it, then asks whether to save the new profile, and saves it on
/// yes. With `yes`, it saves without asking. The fit in the file is kept.
pub fn confirm_save(
    term: &mut Terminal,
    loaded: &Loaded,
    change: &Change,
    file: &ConfigFile,
    yes: bool,
) -> Result<()> {
    let Change { before, after, .. } = change;
    for trim in Trim::ALL {
        let field = Field::for_trim(trim);
        if trim.mm(after) != trim.mm(before) && loaded.override_of(field).is_some() {
            writeln!(
                term.err,
                "{} is set; the saved value is not used while it is",
                field.env
            )?;
        }
    }
    if yes || term.confirm(&format!("Save to {}?", file.path().display()), true)? {
        file.save(&loaded.saved.with_profile(after.clone()))?;
        writeln!(term.out, "Saved.")?;
    } else {
        writeln!(term.out, "Not saved.")?;
    }
    Ok(())
}

/// The prompts on a real terminal.
struct Inquire;

const NOT_A_NUMBER: &str = "type a number of mm, such as 1.5 or -0.5";

impl Prompts for Inquire {
    fn confirm(&mut self, message: &str, default: bool) -> Result<bool> {
        answer(Confirm::new(message).with_default(default).prompt())
    }

    fn select(
        &mut self,
        message: &str,
        options: &[String],
        start: usize,
        help: &str,
    ) -> Result<usize> {
        let choice = Select::new(message, options.to_vec())
            .with_help_message(help)
            .with_starting_cursor(start)
            .raw_prompt();
        answer(choice).map(|choice| choice.index)
    }

    fn number(&mut self, message: &str, default: f64, help: &str) -> Result<f64> {
        let mm = CustomType::<f64>::new(message)
            .with_default(default)
            .with_help_message(help)
            .with_error_message(NOT_A_NUMBER)
            .with_validator(|mm: &f64| {
                Ok(if mm.is_finite() {
                    Validation::Valid
                } else {
                    Validation::Invalid(NOT_A_NUMBER.into())
                })
            })
            .prompt();
        answer(mm)
    }
}

/// A prompt's answer, with Esc and Ctrl-C turned into [`Cancelled`] and a
/// missing terminal into a plain error.
fn answer<T>(result: Result<T, InquireError>) -> Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            Err(Cancelled.into())
        }
        Err(InquireError::NotTTY) => bail!("the prompts need a terminal"),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
pub use scripted::Answer;

#[cfg(test)]
mod scripted {
    //! A terminal for tests: the prompts answer from a list, and the streams
    //! are collected.

    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io::{self, Write};
    use std::rc::Rc;

    use anyhow::Result;

    use super::{Cancelled, Prompts, Terminal};

    /// One scripted answer.
    #[derive(Debug, Clone, Copy)]
    pub enum Answer {
        Confirm(bool),
        Select(usize),
        Number(f64),
        /// Esc at whatever prompt comes.
        Cancel,
    }

    /// What a scripted terminal wrote.
    pub struct Written {
        out: Buffer,
        err: Buffer,
    }

    impl Written {
        pub fn out(&self) -> String {
            String::from_utf8(self.out.0.borrow().clone()).unwrap()
        }

        pub fn err(&self) -> String {
            String::from_utf8(self.err.0.borrow().clone()).unwrap()
        }
    }

    #[derive(Clone, Default)]
    struct Buffer(Rc<RefCell<Vec<u8>>>);

    impl Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Answers in order. Fails the test when a prompt has no answer queued,
    /// when the answer is for another kind of prompt, or when answers are
    /// left at the end.
    struct Script(VecDeque<Answer>);

    impl Script {
        fn next(&mut self, message: &str) -> Answer {
            self.0
                .pop_front()
                .unwrap_or_else(|| panic!("no answer queued for {message:?}"))
        }
    }

    impl Drop for Script {
        fn drop(&mut self) {
            if !std::thread::panicking() {
                assert!(self.0.is_empty(), "answers left over: {:?}", self.0);
            }
        }
    }

    impl Prompts for Script {
        fn confirm(&mut self, message: &str, _: bool) -> Result<bool> {
            match self.next(message) {
                Answer::Confirm(yes) => Ok(yes),
                Answer::Cancel => Err(Cancelled.into()),
                other => panic!("{other:?} for the confirm {message:?}"),
            }
        }

        fn select(
            &mut self,
            message: &str,
            options: &[String],
            _: usize,
            _: &str,
        ) -> Result<usize> {
            match self.next(message) {
                Answer::Select(ix) => {
                    assert!(
                        ix < options.len(),
                        "{ix} is past the options of {message:?}"
                    );
                    Ok(ix)
                }
                Answer::Cancel => Err(Cancelled.into()),
                other => panic!("{other:?} for the select {message:?}"),
            }
        }

        fn number(&mut self, message: &str, _: f64, _: &str) -> Result<f64> {
            match self.next(message) {
                Answer::Number(n) => Ok(n),
                Answer::Cancel => Err(Cancelled.into()),
                other => panic!("{other:?} for the number {message:?}"),
            }
        }
    }

    impl Terminal {
        /// A terminal that answers with `answers`, in order, and collects
        /// what is written. It is interactive and shows no progress bar.
        pub fn scripted(answers: impl IntoIterator<Item = Answer>) -> (Terminal, Written) {
            let (out, err) = (Buffer::default(), Buffer::default());
            let terminal = Terminal {
                prompts: Box::new(Script(answers.into_iter().collect())),
                out: Box::new(out.clone()),
                err: Box::new(err.clone()),
                is_interactive: true,
                show_progress: false,
            };
            (terminal, Written { out, err })
        }
    }
}
