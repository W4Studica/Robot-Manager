use crate::config::signal_name;
use crate::output::{Journal, LineSink, OutputReader};
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub fn enable_subreaper() -> io::Result<()> {
    // SAFETY: prctl only changes the calling process's child-reaping policy.
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn output_pipe() -> io::Result<(File, OwnedFd)> {
    let mut fds = [-1; 2];
    // SAFETY: fds has space for the two descriptors returned by pipe2.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: pipe2 succeeded and both descriptors are now owned exactly once.
    Ok(unsafe { (File::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}
fn kill_group(pgid: i32, signal: i32) -> io::Result<()> {
    // SAFETY: only our child's positive PID is negated to address its own group.
    if pgid <= 0 {
        return Err(io::Error::other("invalid robot process group"));
    }
    if unsafe { libc::kill(-pgid, signal) } < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
    }
    Ok(())
}
fn reap_orphans(pgid: i32) {
    loop {
        let mut status = 0;
        // SAFETY: waitpid is restricted to children in this robot's process
        // group. Its leader has already been waited on through std::Child.
        let pid = unsafe { libc::waitpid(-pgid, &mut status, libc::WNOHANG) };
        if pid <= 0 {
            break;
        }
    }
}
fn group_alive(pgid: i32) -> bool {
    reap_orphans(pgid);
    // SAFETY: signal 0 checks existence and cannot deliver a signal.
    unsafe {
        libc::kill(-pgid, 0) == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

pub struct ChildGroup {
    child: Child,
    pgid: i32,
    output: Option<OutputReader>,
    cleaned: bool,
    scheduling: crate::realtime::Report,
}
impl ChildGroup {
    pub fn spawn(
        command_file: &Path,
        cwd: &Path,
        env: &HashMap<String, String>,
        journal: Journal,
        console: Option<Arc<dyn LineSink>>,
        realtime: crate::realtime::Settings,
    ) -> io::Result<Self> {
        let scheduling = realtime.prepare()?;
        let (mut scheduling_read, scheduling_write) = output_pipe()?;
        let report_fd = scheduling_write.as_raw_fd();
        let (read, write) = output_pipe()?;
        let stderr = write.try_clone()?;
        let mut command = Command::new("/bin/sh");
        command
            .arg(command_file)
            .current_dir(cwd)
            .env_clear()
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::from(write))
            .stderr(Stdio::from(stderr));
        // SAFETY: the closure captures only validated Copy values. It calls
        // async-signal-safe syscalls without allocation, logging or locks.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                scheduling.apply_and_report(report_fd)
            });
        }
        let spawned = command.spawn();
        drop(scheduling_write);
        drop(command);
        let child = match spawned {
            Ok(child) => child,
            Err(error) => {
                if let Ok(report) = crate::realtime::Report::read(&mut scheduling_read) {
                    if report.fatal_errno != 0 {
                        return Err(io::Error::other(format!(
                            "robot scheduling setup failed: {}",
                            io::Error::from_raw_os_error(report.fatal_errno)
                        )));
                    }
                }
                return Err(error);
            }
        };
        let pgid = i32::try_from(child.id()).map_err(io::Error::other)?;
        let mut group = Self {
            child,
            pgid,
            output: None,
            cleaned: false,
            scheduling: crate::realtime::Report {
                actual: crate::realtime::Snapshot::NORMAL,
                fallback_errno: 0,
                fatal_errno: 0,
            },
        };
        group.scheduling = crate::realtime::Report::read(&mut scheduling_read)?;
        group.scheduling.log();
        // The process is already owned if starting the reader thread fails.
        group.output = Some(OutputReader::start(read, journal, console)?);
        Ok(group)
    }
    pub fn scheduling(&self) -> crate::realtime::Report {
        self.scheduling
    }
    pub fn id(&self) -> i32 {
        self.pgid
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
    fn wait_until(&mut self, deadline: Instant) -> io::Result<Option<ExitStatus>> {
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(
                Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
    pub fn stop(&mut self, signals: &[i32], grace: Duration) -> io::Result<()> {
        let started = Instant::now();
        let deadline = started + grace;
        let mut stopped_by = None;
        crate::log(
            "INFO",
            format_args!(
                "stopping robot program (pid {}): {}; {:.1}s in total",
                self.pgid,
                signals
                    .iter()
                    .map(|&s| signal_name(s))
                    .collect::<Vec<_>>()
                    .join(" then "),
                grace.as_secs_f64()
            ),
        );
        let step = grace / signals.len().max(1) as u32;
        for (index, &signal) in signals.iter().enumerate() {
            kill_group(self.pgid, signal)?;
            if self
                .wait_until((started + step * (index as u32 + 1)).min(deadline))?
                .is_some()
            {
                stopped_by = Some(signal_name(signal));
                break;
            }
        }
        if stopped_by.is_none() {
            crate::log(
                "WARNING",
                "robot program did not stop within its grace period; sending SIGKILL",
            );
            kill_group(self.pgid, libc::SIGKILL)?;
            self.child.wait()?;
        }
        self.cleanup_group(false, deadline)?;
        if let Some(signal) = stopped_by {
            crate::log(
                "INFO",
                format_args!(
                    "robot program stopped by {signal} after {:.1}s",
                    started.elapsed().as_secs_f64()
                ),
            );
        } else {
            crate::log(
                "INFO",
                format_args!(
                    "robot program killed after {:.1}s",
                    started.elapsed().as_secs_f64()
                ),
            );
        }
        self.finish();
        Ok(())
    }
    pub fn finish_after_exit(&mut self, grace: Duration) -> io::Result<()> {
        self.cleanup_group(true, Instant::now() + grace)?;
        self.finish();
        Ok(())
    }
    fn cleanup_group(&mut self, send_term: bool, deadline: Instant) -> io::Result<()> {
        if send_term && group_alive(self.pgid) {
            crate::log(
                "WARNING",
                "robot program exited but processes of its group remain; sending SIGTERM",
            );
            kill_group(self.pgid, libc::SIGTERM)?;
        }
        while group_alive(self.pgid) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if group_alive(self.pgid) {
            crate::log(
                "WARNING",
                "processes of the robot program's group remain; sending SIGKILL",
            );
            kill_group(self.pgid, libc::SIGKILL)?;
            let kill_deadline = Instant::now() + Duration::from_secs(2);
            while group_alive(self.pgid) && Instant::now() < kill_deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if group_alive(self.pgid) {
                return Err(io::Error::other("robot process group survived SIGKILL"));
            }
        }
        Ok(())
    }
    fn finish(&mut self) {
        self.cleaned = true;
        if let Some(mut output) = self.output.take() {
            output.stop();
        }
    }
}
impl Drop for ChildGroup {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = kill_group(self.pgid, libc::SIGKILL);
            let _ = self.child.wait();
            let _ = self.cleanup_group(false, Instant::now() + Duration::from_secs(2));
        }
        // OutputReader owns and joins its worker, including on error paths.
    }
}
