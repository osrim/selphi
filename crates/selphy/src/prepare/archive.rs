//! Moving finished sources out of the way, never over an existing file.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::atomic;

/// Moves a finished source into `archive_dir` and returns where it went. An
/// existing file there is never replaced: `photo.jpg` becomes `photo-2.jpg`,
/// then `photo-3.jpg`, and so on.
pub fn archive(source: &Path, archive_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(archive_dir)
        .with_context(|| format!("creating {}", archive_dir.display()))?;
    let target = free_name(archive_dir, source)?;
    match fs::rename(source, &target) {
        Ok(()) => {}
        // A rename cannot cross disks, e.g. src/ on an SD card.
        Err(e) if e.kind() == ErrorKind::CrossesDevices => {
            atomic::copy(source, &target)?;
            fs::remove_file(source).with_context(|| format!("removing {}", source.display()))?;
        }
        Err(e) => {
            return Err(e).with_context(|| format!("moving to {}", target.display()));
        }
    }
    Ok(target)
}

/// The first of `name.ext`, `name-2.ext`, `name-3.ext`, ... that does not
/// exist in `dir`.
pub(super) fn free_name(dir: &Path, source: &Path) -> Result<PathBuf> {
    let stem = source
        .file_stem()
        .with_context(|| format!("{} has no file name", source.display()))?;
    let mut n = 1;
    let target = loop {
        let mut name = stem.to_os_string();
        if n > 1 {
            name.push(format!("-{n}"));
        }
        if let Some(ext) = source.extension() {
            name.push(".");
            name.push(ext);
        }
        let candidate = dir.join(name);
        if !candidate.try_exists()? {
            break candidate;
        }
        n += 1;
    };
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::fresh_dir;

    #[test]
    fn archive_never_replaces_an_existing_file() {
        let dir = fresh_dir("archive");
        let (src, originals) = (dir.join("src"), dir.join("originals"));
        fs::create_dir(&src).unwrap();

        let mut archived = Vec::new();
        for round in 1..=3 {
            let photo = src.join("photo.v2.jpg");
            fs::write(&photo, format!("round {round}")).unwrap();
            archived.push(archive(&photo, &originals).unwrap());
            assert!(!photo.exists(), "the source is moved, not copied");
        }

        let names: Vec<_> = archived.iter().map(|p| p.file_name().unwrap()).collect();
        assert_eq!(names, ["photo.v2.jpg", "photo.v2-2.jpg", "photo.v2-3.jpg"]);
        let first = fs::read_to_string(&archived[0]).unwrap();
        assert_eq!(first, "round 1", "the first archived file is untouched");
    }

    #[test]
    fn archive_numbers_files_without_an_extension() {
        let dir = fresh_dir("archive-no-ext");
        let originals = dir.join("originals");
        fs::create_dir(&originals).unwrap();
        fs::write(originals.join("scan"), b"old").unwrap();
        let scan = dir.join("scan");
        fs::write(&scan, b"new").unwrap();
        assert_eq!(
            archive(&scan, &originals).unwrap(),
            originals.join("scan-2")
        );
    }
}
