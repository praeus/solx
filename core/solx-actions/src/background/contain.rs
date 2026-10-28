//! Process-tree containment for a background command.
//!
//! `kill_on_drop` kills only the `cmd /C` / `sh -c` shell, not what it
//! started, which a long-lived background can't live with. So each one
//! runs contained:
//!
//! * **Windows:** a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
//!   Closing the handle — on [`Containment::kill`], on drop, or when the
//!   host process dies however it dies — terminates every process in it.
//!   The shell is assigned right after it spawns; anything it starts
//!   before that instant escapes. `cmd /C` starts nothing that early in
//!   practice, and fixing it would mean spawning suspended, which
//!   `tokio::process` can't resume.
//! * **POSIX:** a new process group, killed as a group. Best effort: a
//!   host killed with `SIGKILL` leaves the group running.

use std::io;

use tokio::process::{Child, Command};

pub(crate) struct Containment {
    #[cfg(windows)]
    job: usize,
    #[cfg(unix)]
    pgid: Option<i32>,
}

impl Containment {
    /// Set `cmd` up to be contained. Call before spawning it.
    pub(crate) fn prepare(cmd: &mut Command) {
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(not(unix))]
        let _ = cmd;
    }

    /// Contain `child`, which was spawned from a [`Self::prepare`]d command.
    #[cfg(windows)]
    pub(crate) fn attach(child: &Child) -> io::Result<Self> {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("the background exited before it was contained"))?;
        // SAFETY: plain Win32 calls on handles this function owns (`job`)
        // or borrows for the call (`process`, alive while `child` is).
        unsafe {
            let job: HANDLE = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) != 0
                && AssignProcessToJobObject(job, process as HANDLE) != 0;
            if !ok {
                let err = io::Error::last_os_error();
                CloseHandle(job);
                return Err(err);
            }
            Ok(Containment { job: job as usize })
        }
    }

    #[cfg(unix)]
    pub(crate) fn attach(child: &Child) -> io::Result<Self> {
        // `process_group(0)` made the shell its own group's leader.
        Ok(Containment {
            pgid: child.id().map(|pid| pid as i32),
        })
    }

    /// Kill every process in the tree. Idempotent.
    pub(crate) fn kill(&self) {
        #[cfg(windows)]
        // SAFETY: `job` is the handle `attach` created, open until drop.
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job as _, 1);
        }
        #[cfg(unix)]
        if let Some(pgid) = self.pgid {
            // SAFETY: signalling a process group; no memory is involved.
            unsafe {
                libc::killpg(pgid, libc::SIGKILL);
            }
        }
    }
}

impl Drop for Containment {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: closes the handle `attach` created, exactly once. With
        // `KILL_ON_JOB_CLOSE` this also ends whatever is still in the job.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job as _);
        }
        #[cfg(unix)]
        self.kill();
    }
}
