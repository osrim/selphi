//! Which files a prepare run reads, and what each one writes.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// The formats the decoder is built with (see the `image` features in
/// the workspace Cargo.toml).
const EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "tif", "tiff"];

/// The photos to prepare: every file named in `paths`, plus the supported
/// images directly inside every directory named (not recursive). Sorted, and
/// each file listed once.
pub fn collect_inputs(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in paths {
        if !path.is_dir() {
            files.push(path.clone());
            continue;
        }
        let entries = fs::read_dir(path).with_context(|| format!("listing {}", path.display()))?;
        for entry in entries {
            let file = entry?.path();
            if file.is_file() && is_supported(&file) {
                files.push(file);
            }
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// A source and the output it writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The photo to prepare.
    pub source: PathBuf,
    /// The print-ready JPEG it becomes, in the output folder.
    pub output: PathBuf,
}

/// Each of `inputs`, in order, with its output in `out_dir`. Fails, before
/// any photo is prepared, when two or more sources would write the same
/// output, with one line per output. Names that differ only in case are the
/// same output, as they are on the default macOS disk format.
pub fn plan(inputs: &[PathBuf], out_dir: &Path) -> Result<Vec<Planned>> {
    let planned = inputs
        .iter()
        .map(|source| {
            Ok(Planned {
                source: source.clone(),
                output: out_dir.join(output_name(source)?),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut by_output: HashMap<String, Vec<&Planned>> = HashMap::new();
    let mut order = Vec::new();
    for item in &planned {
        let key = item.output.to_string_lossy().to_lowercase();
        let sources = by_output.entry(key.clone()).or_default();
        if sources.is_empty() {
            order.push(key);
        }
        sources.push(item);
    }
    let clashes: Vec<String> = order
        .iter()
        .map(|key| &by_output[key])
        .filter(|sources| sources.len() > 1)
        .map(|sources| clash_line(sources))
        .collect();
    if !clashes.is_empty() {
        bail!("{}", clashes.join("\n"));
    }
    Ok(planned)
}

/// "a/x.jpg and b/x.jpg would both be written to out/x-selphy.jpg", or "a,
/// b and c would all be written to …" for more than two.
fn clash_line(sources: &[&Planned]) -> String {
    let names: Vec<String> = sources
        .iter()
        .map(|item| item.source.display().to_string())
        .collect();
    let (last, rest) = names.split_last().expect("a clash has two or more sources");
    let both = if rest.len() == 1 { "both" } else { "all" };
    format!(
        "{} and {last} would {both} be written to {}",
        rest.join(", "),
        sources[0].output.display()
    )
}

/// `photo.v2.png` becomes `photo.v2-selphy.jpg`: only the last extension is
/// replaced.
fn output_name(source: &Path) -> Result<OsString> {
    let mut name = source
        .file_stem()
        .with_context(|| format!("{} has no file name", source.display()))?
        .to_os_string();
    name.push("-selphy.jpg");
    Ok(name)
}

/// A supported extension, in any case, on a file that is not hidden. Hidden
/// files include the `._name.jpg` metadata files macOS writes on external
/// drives, which are not images.
fn is_supported(path: &Path) -> bool {
    let hidden = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'));
    let known = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            EXTENSIONS
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        });
    known && !hidden
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::fresh_dir;

    #[test]
    fn collects_supported_images_from_a_directory() {
        let dir = fresh_dir("collect");
        for name in [
            "b.jpg",
            "A.JPEG",
            "c.tif",
            "notes.txt",
            ".hidden.jpg",
            "._b.jpg",
        ] {
            fs::write(dir.join(name), b"").unwrap();
        }
        fs::create_dir(dir.join("nested.jpg")).unwrap(); // a directory, not a file

        let names: Vec<String> = collect_inputs(&[dir])
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["A.JPEG", "b.jpg", "c.tif"]);
    }

    #[test]
    fn named_files_are_kept_as_given_and_listed_once() {
        let dir = fresh_dir("collect-named");
        let photo = dir.join("photo.jpg");
        fs::write(&photo, b"").unwrap();
        let odd = dir.join("scan.bmp"); // unsupported extension, but asked for
        let files = collect_inputs(&[photo.clone(), dir.clone(), odd.clone()]).unwrap();
        assert_eq!(files, [photo, odd]);
    }

    fn planned(sources: &[&str]) -> Result<Vec<Planned>> {
        let inputs: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
        plan(&inputs, Path::new("out"))
    }

    #[test]
    fn unique_stems_are_planned_in_input_order() {
        let planned = planned(&["b/z.jpg", "a/photo.v2.png"]).unwrap();
        assert_eq!(
            planned,
            [
                Planned {
                    source: "b/z.jpg".into(),
                    output: "out/z-selphy.jpg".into(),
                },
                Planned {
                    source: "a/photo.v2.png".into(),
                    output: "out/photo.v2-selphy.jpg".into(),
                },
            ]
        );
    }

    #[test]
    fn two_sources_with_one_stem_fail_and_both_are_named() {
        let err = planned(&["a/x.jpg", "b/x.jpg", "c.jpg", "c.png", "C.tif"]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "a/x.jpg and b/x.jpg would both be written to out/x-selphy.jpg\n\
             c.jpg, c.png and C.tif would all be written to out/c-selphy.jpg"
        );
    }

    #[test]
    fn a_missing_directory_is_just_a_missing_file() {
        let paths = [PathBuf::from("/nonexistent/src")];
        assert_eq!(collect_inputs(&paths).unwrap(), paths);
    }
}
