//! Shared lock and atomic write for workspace-namespaced persist files.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use fs4::{FileExt, TryLockError};

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

/// Apply the changes one session made since `baseline` onto `disk`.
///
/// `baseline` is the bucket this session last loaded or saved, `current` is
/// its bucket now. A key in `baseline` that is gone from `current` is
/// removed from `disk`. A key that is new in `current`, or whose value
/// differs from `baseline`, is written to `disk`. Every other key of `disk`
/// stays as it is, so a key another process added or edited since
/// `baseline` survives.
pub(crate) fn apply_session_delta<K: Ord + Clone, V: PartialEq + Clone>(
    disk: &mut BTreeMap<K, V>,
    baseline: &BTreeMap<K, V>,
    current: &BTreeMap<K, V>,
) {
    for key in baseline.keys() {
        if !current.contains_key(key) {
            disk.remove(key);
        }
    }
    for (key, value) in current {
        if baseline.get(key) != Some(value) {
            disk.insert(key.clone(), value.clone());
        }
    }
}

/// Exclusive lock on a sibling `*.lock` file, merge on-disk bytes, then
/// atomic-write (`*.tmp` + flush + fsync + rename, then a best-effort fsync
/// of the parent directory on Unix so the rename survives a crash).
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
    // The lock file carries no bytes; keep whatever is there.
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    // Trait-path call: `File` has an inherent `lock` too, unstable on the
    // pinned toolchain.
    FileExt::lock(&lock_file)?;
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
        sync_file(&file)?;
        fs::rename(&tmp, file_path)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_result?;
    // The new bytes are already in place: a failed directory fsync only
    // weakens crash durability, so it must not report the save as failed
    // (the caller would keep its old baseline and apply the same delta again).
    let _ = sync_parent_dir(file_path);
    Ok(())
}

/// Fsync `file`. On macOS `sync_all` is `F_FULLFSYNC`, which some
/// filesystems (SMB shares, some external volumes) reject with `ENOTSUP` or
/// `EINVAL`. Fall back to a plain `fsync` there.
#[cfg(target_vendor = "apple")]
fn sync_file(file: &fs::File) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    match file.sync_all() {
        Err(err) if matches!(err.raw_os_error(), Some(libc::ENOTSUP | libc::EINVAL)) => {
            // SAFETY: `file` owns an open descriptor for this whole call.
            if unsafe { libc::fsync(file.as_raw_fd()) } == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
        other => other,
    }
}

#[cfg(not(target_vendor = "apple"))]
fn sync_file(file: &fs::File) -> io::Result<()> {
    file.sync_all()
}

/// Fsync the directory that holds `file_path`, so a rename into it is
/// durable. Windows cannot open a directory as a file; NTFS journals the
/// rename itself.
#[cfg(unix)]
fn sync_parent_dir(file_path: &Path) -> io::Result<()> {
    let parent = match file_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_dir(_file_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Non-blocking exclusive lock on the sibling file `<file_path><suffix>`.
///
/// Creates the parent directories and the lock file when missing (the lock
/// file carries no bytes). `Ok(Some(file))` holds the lock until the file is
/// dropped; the OS also drops it when the process dies. `Ok(None)` means
/// another open file description holds it. Other I/O errors return `Err`.
pub(crate) fn try_lock_sibling(file_path: &Path, suffix: &str) -> io::Result<Option<fs::File>> {
    if let Some(parent) = file_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(sibling_suffix(file_path, suffix))?;
    // `?` here would report a held lock as an `Err`: `WouldBlock` must stay
    // `Ok(None)`.
    match FileExt::try_lock(&lock_file) {
        Ok(()) => Ok(Some(lock_file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(err)) => Err(err),
    }
}
