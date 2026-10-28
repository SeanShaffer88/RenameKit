//! Platform layer: filesystem primitives RenameKit needs, all from
//! `std::fs` so the same code runs on Windows and on the Linux test
//! host. There are no hand-declared Win32 bindings in this crate:
//! renaming, metadata and directory listing are portable std APIs,
//! and the Windows-specific *semantics* RenameKit promises (illegal
//! characters, reserved names, case-insensitive comparison) are
//! policy in `lib.rs`, not OS calls.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use crate::{ERR_NOT_FOUND, ERR_PLATFORM};

pub type PResult<T> = Result<T, (i32, String)>;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Size and modification stamp of one file, recorded in apply logs
/// so `rk_rollback` can tell whether the file changed since apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    /// Whole seconds since the Unix epoch (UTC).
    pub mtime_unix: i64,
    /// Nanoseconds since the Unix epoch; finer-grained change check.
    pub mtime_unix_nanos: i64,
}

pub fn stamp(path: &Path) -> PResult<Stamp> {
    let md = fs::metadata(path).map_err(|e| {
        (
            ERR_NOT_FOUND,
            format!("cannot read metadata for {}: {e}", path.display()),
        )
    })?;
    let (mtime_unix, mtime_unix_nanos) = match md.modified() {
        Ok(t) => match t.duration_since(UNIX_EPOCH) {
            Ok(d) => {
                let secs = d.as_secs() as i64;
                (
                    secs,
                    secs.saturating_mul(1_000_000_000)
                        .saturating_add(d.subsec_nanos() as i64),
                )
            }
            Err(_) => (0, 0), // pre-1970 stamp: report 0 rather than guess
        },
        Err(_) => (0, 0),
    };
    Ok(Stamp {
        size: md.len(),
        mtime_unix,
        mtime_unix_nanos,
    })
}

/// A unique temporary path inside `dir` that does not exist yet.
/// Used by the two-phase rename engine; `tag` distinguishes
/// callers in listings while debugging.
pub fn temp_path(dir: &Path, tag: &str) -> PathBuf {
    loop {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = dir.join(format!(".renamekit-{tag}-{}-{n}", std::process::id()));
        if !candidate.exists() {
            return candidate;
        }
    }
}

/// Rename one file, mapping the OS error into the family shape.
pub fn rename(from: &Path, to: &Path) -> PResult<()> {
    fs::rename(from, to).map_err(|e| {
        (
            ERR_PLATFORM,
            format!("rename {} -> {} failed: {e}", from.display(), to.display()),
        )
    })
}

/// Sorted names of the entries directly inside `dir` (files and
/// directories; callers filter).
pub fn list_names(dir: &Path) -> PResult<Vec<String>> {
    let rd = fs::read_dir(dir).map_err(|e| {
        (
            ERR_NOT_FOUND,
            format!("cannot list directory {}: {e}", dir.display()),
        )
    })?;
    let mut names = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|e| (ERR_PLATFORM, format!("directory read failed: {e}")))?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}
