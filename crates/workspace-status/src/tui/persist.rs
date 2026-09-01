//! Shared lock and atomic write for workspace-namespaced persist files.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use fs4::fs_std::FileExt;

/// Reserved workspace map key for version-1 records.
///
/// SHA-256 hex workspace ids cannot equal this string. A version-1 file has
/// a flat `entries` field and no workspace id. Persist writes version 2 and
/// keeps those records under this key. Save of the current workspace inserts
/// only that workspace bucket. It does not replace this key. Load uses the
/// current workspace bucket when that key exists. Otherwise it uses this key.
/// GC of one cwd must not drop this key.
pub(crate) const LEGACY_WORKSPACE_ID: &str = "__legacy__";

fn sibling_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

/// Exclusive lock on a sibling `*.lock` file, merge on-disk bytes, then
/// atomic-write (`*.tmp` + flush + rename).
pub(crate) fn persist_with_lock(
    file_path: &Path,
    merge: impl FnOnce(Option<&[u8]>) -> io::Result<Vec<u8>>,
) -> io::Result<()> {
    if let Some(parent) = file_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let lock_path = sibling_suffix(file_path, ".lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    lock_file.lock_exclusive()?;
    let existing = match fs::read(file_path) {
        Ok(bytes) => Some(bytes),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err),
    };
    let body = merge(existing.as_deref())?;
    let tmp = sibling_suffix(file_path, ".tmp");
    let write_result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&body)?;
        file.flush()?;
        fs::rename(&tmp, file_path)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_result
}
