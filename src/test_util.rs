//! Helpers shared by the unit tests.

use std::fs;
use std::path::PathBuf;

/// An empty directory under the system temp dir, unique to this test run and
/// `name`. Anything left there by an earlier run is removed first.
pub fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("selphy-test-{}-{name}", std::process::id()));
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
