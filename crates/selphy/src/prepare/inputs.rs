//! Which files a prepare run reads.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

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

    #[test]
    fn a_missing_directory_is_just_a_missing_file() {
        let paths = [PathBuf::from("/nonexistent/src")];
        assert_eq!(collect_inputs(&paths).unwrap(), paths);
    }
}
