//! Filesystem walking that honors `.gitignore`.

use std::path::Path;

use ignore::{DirEntry, WalkBuilder};

pub fn rust_files(root: &Path) -> impl Iterator<Item = DirEntry> {
    WalkBuilder::new(root)
        .standard_filters(true)
        .build()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_type().map(|t| t.is_file()).unwrap_or(false)
                && e.path().extension().and_then(|s| s.to_str()) == Some("rs")
        })
}
