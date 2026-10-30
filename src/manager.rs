use crate::output::{Journal, LineSink};
use crate::process::{self, ChildGroup};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Policy {
    pub backoff_initial: Duration,
    pub backoff_max: Duration,
    pub min_uptime: Duration,
    pub wait_poll: Duration,
    pub stop_grace: Duration,
    pub stop_signals: Vec<i32>,
    pub realtime: crate::realtime::Settings,
    pub robot_user: Option<String>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            backoff_initial: Duration::from_secs(1),
            backoff_max: Duration::from_secs(10),
            min_uptime: Duration::from_secs(10),
            wait_poll: Duration::from_secs(2),
            stop_grace: Duration::from_secs(5),
            stop_signals: vec![libc::SIGINT, libc::SIGTERM],
            realtime: crate::realtime::Settings::default(),
            robot_user: None,
        }
    }
}
pub struct Manager {
    pub command_file: PathBuf,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub policy: Policy,
    pub stopping: Arc<AtomicBool>,
    pub starts: Arc<AtomicUsize>,
    pub journal: Journal,
    pub console: Option<Arc<dyn LineSink>>,
}
impl Manager {
    pub fn run(&self) -> io::Result<()> {
        process::enable_subreaper()?;
        let mut backoff = self.policy.backoff_initial;
        let mut warned_missing = false;
        while !self.stopping.load(Ordering::Relaxed) {
            if !self.command_file.is_file() {
                if !warned_missing {
                    crate::log(
                        "WARNING",
                        format_args!(
                            "{} does not exist yet; waiting for a deploy",
                            self.command_file.display()
                        ),
                    );
                    warned_missing = true;
                }
                self.wait(self.policy.wait_poll);
                continue;
            }
            warned_missing = false;
            let started = Instant::now();
            let mut group = match ChildGroup::spawn(
                &self.command_file,
                &self.cwd,
                &self.env,
                self.journal.clone(),
                self.console.clone(),
                self.policy.realtime,
                self.policy.robot_user.as_deref(),
            ) {
                Ok(group) => group,
                Err(error) => {
                    crate::log(
                        "ERROR",
                        format_args!("could not start {}: {error}", self.command_file.display()),
                    );
                    self.wait(backoff);
                    backoff = backoff.saturating_mul(2).min(self.policy.backoff_max);
                    continue;
                }
            };
            self.starts.fetch_add(1, Ordering::Relaxed);
            crate::log(
                "INFO",
                format_args!("started robot program (pid {})", group.id()),
            );
            let status = loop {
                if self.stopping.load(Ordering::Relaxed) {
                    group.stop(&self.policy.stop_signals, self.policy.stop_grace)?;
                    return Ok(());
                }
                if let Some(status) = group.try_wait()? {
                    break status;
                }
                thread::sleep(Duration::from_millis(10));
            };
            let uptime = started.elapsed();
            group.finish_after_exit(self.policy.stop_grace)?;
            if self.stopping.load(Ordering::Relaxed) {
                break;
            }
            crate::log(
                "WARNING",
                format_args!(
                    "robot program exited with {status} after {:.1}s",
                    uptime.as_secs_f64()
                ),
            );
            let delay = if uptime >= self.policy.min_uptime {
                backoff = self.policy.backoff_initial;
                backoff
            } else {
                let delay = backoff;
                backoff = backoff.saturating_mul(2).min(self.policy.backoff_max);
                delay
            };
            self.wait(delay);
        }
        Ok(())
    }
    fn wait(&self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while !self.stopping.load(Ordering::Relaxed) && Instant::now() < deadline {
            thread::sleep(
                Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
}
