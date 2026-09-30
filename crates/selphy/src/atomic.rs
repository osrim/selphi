//! File writes that never leave a partial file. The data goes to a hidden
//! temporary file next to the target, which is then renamed over it, so an
//! interrupted write leaves the old file or none, never a truncated one.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Writes `bytes` to `path`, replacing any file there.
pub fn write(path: &Path, bytes: impl AsRef<[u8]>) -> Result<()> {
    let temp = temp_beside(path)?;
    fs::write(&temp, bytes).with_context(|| format!("writing {}", path.display()))?;
    finish(&temp, path)
}

/// Copies `from` to `to`, replacing any file there.
pub fn copy(from: &Path, to: &Path) -> Result<()> {
    let temp = temp_beside(to)?;
    fs::copy(from, &temp).with_context(|| format!("copying to {}", to.display()))?;
    finish(&temp, to)
}

/// `dir/.name.part` for `dir/name`. The leading dot makes `prepare` skip a
/// leftover one.
fn temp_beside(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .with_context(|| format!("{} has no file name", path.display()))?;
    let mut temp = OsString::from(".");
    temp.push(name);
    temp.push(".part");
    Ok(path.with_file_name(temp))
}

fn finish(temp: &Path, path: &Path) -> Result<()> {
    fs::rename(temp, path).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::fresh_dir;

    #[test]
    fn write_replaces_the_file_and_leaves_no_temporary() {
        let dir = fresh_dir("atomic-write");
        let path = dir.join("a.jpg");
        fs::write(&path, b"old").unwrap();
        write(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["a.jpg"]);
    }

    #[test]
    fn copy_keeps_the_source() {
        let dir = fresh_dir("atomic-copy");
        let (from, to) = (dir.join("from"), dir.join("to"));
        fs::write(&from, b"data").unwrap();
        copy(&from, &to).unwrap();
        assert_eq!(fs::read(&to).unwrap(), b"data");
        assert!(from.exists());
    }
}
