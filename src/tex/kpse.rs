//! File lookup through `kpsewhich`, so that dviavi finds exactly the files
//! the TeX installation (and dvipdfmx) would use.

use std::path::PathBuf;
use std::process::Command;

/// Finds `name` (with extension) in the TeX directory tree.
///
/// Returns `None` when kpsewhich is missing or does not know the file.
/// No `mktex*` generation is requested.
pub fn find(name: &str) -> Option<PathBuf> {
    let out = Command::new("kpsewhich").arg(name).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8(out.stdout).ok()?;
    let path = path.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}
