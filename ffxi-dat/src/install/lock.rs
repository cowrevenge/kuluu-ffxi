//! A file lock on the DAT root: a process reading an install holds it shared,
//! the updater wants it exclusive, so neither can surprise the other.

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};

/// Sits at the DAT root rather than the registry entry so a root the shell
/// named is covered too.
pub const LOCK_FILE: &str = ".kuluu-lock";

pub fn lock_path(root: &Path) -> PathBuf {
    root.join(LOCK_FILE)
}

/// The most recent reader's note about itself; the OS lock is what actually
/// holds, this only makes the refusal message name someone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    pub exe: String,
}

impl fmt::Display for Holder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (pid {})", self.exe, self.pid)
    }
}

impl Holder {
    fn this_process() -> Holder {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "kuluu".to_string());
        Holder {
            pid: std::process::id(),
            exe,
        }
    }

    fn parse(line: &str) -> Option<Holder> {
        let (pid, exe) = line.trim().split_once(' ')?;
        Some(Holder {
            pid: pid.parse().ok()?,
            exe: exe.to_string(),
        })
    }
}

/// Held for as long as the install is open for reading.
#[derive(Debug)]
pub struct SharedLock {
    _file: File,
}

/// Held for as long as the install is being rewritten.
#[derive(Debug)]
pub struct ExclusiveLock {
    _file: File,
}

#[derive(Debug)]
pub enum LockError {
    /// Another lock of the opposing kind is held; `holder` is the last reader's
    /// note when there is one.
    Held {
        path: PathBuf,
        holder: Option<Holder>,
    },
    Io {
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::Held {
                path,
                holder: Some(h),
            } => write!(f, "{h} has this install open ({})", path.display()),
            LockError::Held { path, holder: None } => {
                write!(f, "another process holds {}", path.display())
            }
            LockError::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for LockError {}

impl LockError {
    pub fn is_held(&self) -> bool {
        matches!(self, LockError::Held { .. })
    }
}

fn open_lock_file(root: &Path) -> Result<(PathBuf, File), LockError> {
    let path = lock_path(root);
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map(|file| (path.clone(), file))
        .map_err(|source| LockError::Io { path, source })
}

fn read_holder(file: &mut File) -> Option<Holder> {
    let mut text = String::new();
    file.rewind().ok()?;
    file.read_to_string(&mut text).ok()?;
    Holder::parse(&text)
}

fn classify(path: PathBuf, file: &mut File, err: TryLockError) -> LockError {
    match err {
        TryLockError::WouldBlock => LockError::Held {
            path,
            holder: read_holder(file),
        },
        TryLockError::Error(source) => LockError::Io { path, source },
    }
}

/// A reader's share of `root`; fails while an updater holds it exclusively.
pub fn shared(root: &Path) -> Result<SharedLock, LockError> {
    let (path, mut file) = open_lock_file(root)?;
    if let Err(e) = file.try_lock_shared() {
        return Err(classify(path, &mut file, e));
    }
    let me = Holder::this_process();
    let _ = file
        .set_len(0)
        .and_then(|_| file.rewind())
        .and_then(|_| writeln!(file, "{} {}", me.pid, me.exe));
    Ok(SharedLock { _file: file })
}

/// The updater's exclusive claim on `root`; fails while any reader holds it.
pub fn exclusive(root: &Path) -> Result<ExclusiveLock, LockError> {
    let (path, mut file) = open_lock_file(root)?;
    if let Err(e) = file.try_lock() {
        return Err(classify(path, &mut file, e));
    }
    Ok(ExclusiveLock { _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kuluu-lock-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_reader_blocks_the_updater_and_names_itself() {
        let root = temp_root("reader");
        let reader = shared(&root).unwrap();
        let err = exclusive(&root).unwrap_err();
        assert!(err.is_held(), "{err}");
        let LockError::Held { holder, .. } = err else {
            unreachable!()
        };
        assert_eq!(holder.map(|h| h.pid), Some(std::process::id()));
        drop(reader);
        assert!(exclusive(&root).is_ok());
    }

    #[test]
    fn readers_share_and_the_updater_blocks_them() {
        let root = temp_root("updater");
        let first = shared(&root).unwrap();
        let second = shared(&root).unwrap();
        drop((first, second));
        let updater = exclusive(&root).unwrap();
        assert!(shared(&root).unwrap_err().is_held());
        drop(updater);
        assert!(shared(&root).is_ok());
    }

    #[test]
    fn a_dead_holders_note_does_not_hold() {
        let root = temp_root("stale");
        std::fs::write(lock_path(&root), "999999 ghost\n").unwrap();
        assert!(exclusive(&root).is_ok());
    }
}
