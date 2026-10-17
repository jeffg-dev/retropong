//! One game window per machine. The OS releases the advisory lock on exit,
//! including crashes; keep the file in place so every process locks one inode.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

const LOCK_NAME: &str = "retropong-game.lock";

fn lock_path() -> PathBuf {
    #[cfg(unix)]
    let directory = PathBuf::from("/tmp");
    #[cfg(windows)]
    let directory =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into())).join("Temp");
    #[cfg(not(any(unix, windows)))]
    let directory = std::env::temp_dir();
    directory.join(LOCK_NAME)
}

/// Holds the exclusive lock until dropped. An existing, unlocked lock file is
/// harmless; it is deliberately never deleted, to avoid lock-inode races.
#[derive(Debug)]
pub struct InstanceGuard {
    _file: File,
}

impl InstanceGuard {
    pub fn acquire() -> io::Result<Option<Self>> {
        Self::acquire_at(&lock_path())
    }

    fn acquire_at(path: &Path) -> io::Result<Option<Self>> {
        let file = match OpenOptions::new().read(true).write(true).create_new(true).open(path) {
            Ok(file) => {
                // Every user can open the shared file read-only and take an
                // exclusive advisory lock, without permission to edit it.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
                }
                file
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => File::open(path)?,
            Err(error) => return Err(error),
        };
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn lock_child() {
        let Some(path) = std::env::var_os("RETROPONG_TEST_LOCK_PATH") else { return };
        let expected = std::env::var("RETROPONG_TEST_LOCK_EXPECT").unwrap();
        let guard = InstanceGuard::acquire_at(Path::new(&path)).unwrap();
        assert_eq!(guard.is_some(), expected != "blocked");
        if expected == "exit-held" {
            // No Rust destructors: exercise the OS's process-exit cleanup.
            std::process::exit(0);
        }
    }

    fn child(path: &Path, expected: &str) {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "single_instance::tests::lock_child", "--nocapture"])
            .env("RETROPONG_TEST_LOCK_PATH", path)
            .env("RETROPONG_TEST_LOCK_EXPECT", expected)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn concurrent_process_is_rejected_and_exit_releases_lock() {
        let path = std::env::temp_dir().join(format!("retropong-lock-test-{}.lock", std::process::id()));
        let first = InstanceGuard::acquire_at(&path).unwrap().expect("first instance");
        child(&path, "blocked");
        drop(first);
        child(&path, "exit-held");
        let next = InstanceGuard::acquire_at(&path).unwrap().expect("OS released child lock");
        drop(next);
        std::fs::remove_file(path).unwrap();
    }
}
