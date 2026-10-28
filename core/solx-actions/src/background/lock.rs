//! The per-machine lock on a background: at most one solx process on this
//! machine runs a given background against a given appdata dir.
//!
//! The per-key mutex in [`super::BackgroundManager`] only covers one host
//! process. A CLI or stdio MCP process that isn't attached to a server
//! starts its own backgrounds, and a background wrapping an embedded
//! database must not run twice against the same files. So a start first
//! takes an OS file lock on `<appdata>/backgrounds/<namespace>/<name>.lock`
//! and holds it until the process tree is gone.
//!
//! An OS lock (`LockFileEx` / `flock`), not a PID file: the OS releases it
//! when its holder dies, however it dies, so there is no stale lock to
//! clean up. Who holds it goes in a separate `<name>.owner.json`, because
//! on Windows a locked file can't be read by anyone else.
//!
//! On POSIX the background inherits the locked descriptor
//! ([`BackgroundLock::inherit_into`]). An `flock` lock belongs to the open
//! file, not the process, so it lasts until the host *and* the background
//! are gone — a `SIGKILL`ed host's orphaned process group still holds it.
//! Windows needs no such step: the Job Object takes the background down
//! with its host (see [`super::contain`]).
//!
//! The embedded database's own file lock stays the last line of defense;
//! this one stops the second start early and says who has it.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use solx_config::ConfigService;
use solx_surface::error::{Result, SolxError};

use super::Key;

pub(crate) struct BackgroundLock {
    /// The lock lives exactly as long as this stays open. Only POSIX reads
    /// it, to pass it on.
    #[cfg_attr(not(unix), allow(dead_code))]
    file: File,
    owner_path: PathBuf,
    owner: Value,
}

/// A namespace or name as one safe path segment.
fn segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_start_matches('.')
        .to_string()
}

impl BackgroundLock {
    /// Take the lock on `key`, waiting up to `wait` for another process to
    /// release it. Refused with the holder's details otherwise.
    pub(crate) async fn acquire(config: &ConfigService, key: &Key, wait: Duration) -> Result<Self> {
        let dir = config.appdata().join("backgrounds").join(segment(&key.0));
        std::fs::create_dir_all(&dir)?;
        let name = segment(&key.1);
        let lock_path = dir.join(format!("{name}.lock"));
        let owner_path = dir.join(format!("{name}.owner.json"));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;

        let deadline = Instant::now() + wait;
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(SolxError::Conflict(held_elsewhere(key, &owner_path)));
                }
                Err(TryLockError::Error(e)) => return Err(e.into()),
            }
        }

        let host = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()));
        let lock = BackgroundLock {
            file,
            owner_path,
            owner: json!({
                "hostPid": std::process::id(),
                "host": host,
                "since": chrono::Utc::now().to_rfc3339(),
            }),
        };
        lock.write_owner();
        Ok(lock)
    }

    /// Note the background's pid in the owner file, once it has one.
    pub(crate) fn set_pid(&mut self, pid: Option<u32>) {
        self.owner["pid"] = json!(pid);
        self.write_owner();
    }

    fn write_owner(&self) {
        // Best effort: it only improves another process's error message.
        if let Ok(text) = serde_json::to_string_pretty(&self.owner) {
            let _ = std::fs::write(&self.owner_path, text);
        }
    }

    /// Let the background `cmd` will spawn inherit the locked descriptor
    /// (POSIX only; see the module doc). Cleared in the child alone, after
    /// the fork, so no other command this host spawns meanwhile inherits it.
    #[cfg(unix)]
    pub(crate) fn inherit_into(&self, cmd: &mut tokio::process::Command) {
        use std::os::fd::AsRawFd;
        let fd = self.file.as_raw_fd();
        // SAFETY: `fcntl` is async-signal-safe, and `fd` stays open in the
        // parent (so in the forked child) until after the spawn returns.
        unsafe {
            cmd.pre_exec(move || {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags >= 0 {
                    libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                }
                Ok(())
            });
        }
    }

    #[cfg(not(unix))]
    pub(crate) fn inherit_into(&self, _cmd: &mut tokio::process::Command) {}
}

impl Drop for BackgroundLock {
    fn drop(&mut self) {
        // Before the lock goes (with `file`, just after this): the next
        // holder can only write its own once it has the lock.
        let _ = std::fs::remove_file(&self.owner_path);
    }
}

fn held_elsewhere(key: &Key, owner_path: &PathBuf) -> String {
    let owner: Option<Value> = std::fs::read_to_string(owner_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let who = match owner {
        Some(o) => {
            let host = o["host"].as_str().unwrap_or("a solx process");
            let mut who = format!("{host}, pid {}", o["hostPid"]);
            if let Some(since) = o["since"].as_str() {
                who.push_str(&format!(", since {since}"));
            }
            who
        }
        None => "a process that hasn't said who it is yet".into(),
    };
    format!(
        "background {}/{} is running in another solx process on this machine ({who}). \
         Stop it there, or run solx-server so other solx processes attach to it \
         instead of starting their own",
        key.0, key.1
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> Key {
        ("ns".into(), "db".into())
    }

    #[tokio::test]
    async fn a_second_holder_is_refused_until_the_first_drops() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(dir.path()).unwrap();
        let first = BackgroundLock::acquire(&cfg, &key(), Duration::ZERO)
            .await
            .unwrap();
        let err = BackgroundLock::acquire(&cfg, &key(), Duration::ZERO)
            .await
            .err()
            .unwrap();
        assert!(matches!(err, SolxError::Conflict(_)), "{err}");
        drop(first);
        BackgroundLock::acquire(&cfg, &key(), Duration::ZERO)
            .await
            .unwrap();
    }

    /// What a `SIGKILL`ed host leaves behind: its own descriptor gone, the
    /// background still running with the inherited one.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_background_keeps_the_lock_after_the_host_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(dir.path()).unwrap();
        let lock = BackgroundLock::acquire(&cfg, &key(), Duration::ZERO)
            .await
            .unwrap();
        let mut cmd = tokio::process::Command::new("sleep");
        cmd.arg("2");
        lock.inherit_into(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        drop(lock);

        assert!(BackgroundLock::acquire(&cfg, &key(), Duration::ZERO)
            .await
            .is_err());
        child.wait().await.unwrap();
        BackgroundLock::acquire(&cfg, &key(), Duration::ZERO)
            .await
            .unwrap();
    }
}
