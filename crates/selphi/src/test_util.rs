//! Helpers shared by the unit tests.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Profile;
use crate::paper::Paper;

/// An empty directory under the system temp dir, named for `name`. Anything
/// left there by an earlier run is removed first, so runs do not pile up
/// folders. Each test uses its own `name`.
pub fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("selphi-test-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A minimal little-endian Exif block holding one tag: Orientation.
pub fn exif_with_orientation(value: u16) -> Vec<u8> {
    let mut exif = b"II*\0".to_vec(); // TIFF header, little-endian
    exif.extend(8u32.to_le_bytes()); // first IFD starts at byte 8
    exif.extend(1u16.to_le_bytes()); // one entry
    exif.extend(0x0112u16.to_le_bytes()); // tag: Orientation
    exif.extend(3u16.to_le_bytes()); // type: SHORT
    exif.extend(1u32.to_le_bytes()); // count
    exif.extend(value.to_le_bytes());
    exif.extend([0, 0]); // value field padding
    exif.extend(0u32.to_le_bytes()); // no next IFD
    exif
}

/// Writes a mid-grey JPEG of `width` x `height` with the given Exif block to
/// `path`, and returns the path.
pub fn write_jpeg(path: &Path, width: u16, height: u16, exif: &[u8]) -> PathBuf {
    let mut encoder = jpeg_encoder::Encoder::new_file(path, 90).unwrap();
    encoder.add_exif_metadata(exif).unwrap();
    let pixels = vec![128u8; usize::from(width) * usize::from(height) * 3];
    encoder
        .encode(&pixels, width, height, jpeg_encoder::ColorType::Rgb)
        .unwrap();
    path.to_path_buf()
}

/// The built-in postcard profile.
pub fn postcard() -> Profile {
    Paper::Postcard.default_profile()
}
