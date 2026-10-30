//! Robot scheduling is configured before exec. The supervisor keeps normal
//! scheduling; HAL threads inherit unless the HAL explicitly overrides it.
use serde::Deserialize;
use std::fs;
use std::io::{self, Read};
use std::os::fd::RawFd;
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Auto,
    On,
    Off,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Policy {
    #[default]
    Rr,
    Fifo,
}
impl Policy {
    fn raw(self) -> i32 {
        match self {
            Self::Rr => libc::SCHED_RR,
            Self::Fifo => libc::SCHED_FIFO,
        }
    }
}
/// auto: try RT, warn and fall back on failure; on: require RT; off: SCHED_OTHER.
/// Scheduling policies work on ordinary Linux too. PREEMPT_RT is a separate
/// kernel feature; applying RR/FIFO does not enable it.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub mode: Mode,
    pub policy: Policy,
    pub priority: i32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Auto,
            policy: Policy::Rr,
            priority: 10,
        }
    }
}
impl Settings {
    pub fn target(self) -> Snapshot {
        if self.mode == Mode::Off {
            Snapshot::NORMAL
        } else {
            Snapshot {
                policy: self.policy.raw(),
                priority: self.priority,
            }
        }
    }
    pub fn prepare(self) -> io::Result<Prepared> {
        if !(1..=99).contains(&self.priority) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "realtime.priority must be between 1 and 99",
            ));
        }
        Ok(Prepared {
            mode: self.mode,
            target: self.target(),
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub policy: i32,
    pub priority: i32,
}
impl Snapshot {
    pub const NORMAL: Self = Self {
        policy: libc::SCHED_OTHER,
        priority: 0,
    };
    pub fn name(self) -> &'static str {
        match self.policy {
            libc::SCHED_OTHER => "OTHER",
            libc::SCHED_RR => "RR",
            libc::SCHED_FIFO => "FIFO",
            _ => "unknown",
        }
    }
    pub fn current() -> io::Result<Self> {
        Linux.snapshot().map_err(io::Error::from_raw_os_error)
    }
}
trait Scheduler {
    fn set(&mut self, target: Snapshot) -> Result<(), i32>;
    fn snapshot(&self) -> Result<Snapshot, i32>;
}
struct Linux;
fn errno() -> i32 {
    io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}
impl Scheduler for Linux {
    fn set(&mut self, target: Snapshot) -> Result<(), i32> {
        let param = libc::sched_param {
            sched_priority: target.priority,
        };
        // SAFETY: pid 0 changes only the calling thread. No RESET_ON_FORK flag
        // is set, so robotCommand's shell/exec and HAL-created threads inherit.
        if unsafe { libc::sched_setscheduler(0, target.policy, &param) } < 0 {
            Err(errno())
        } else {
            Ok(())
        }
    }
    fn snapshot(&self) -> Result<Snapshot, i32> {
        // SAFETY: both calls query the current thread; param is writable.
        let policy = unsafe { libc::sched_getscheduler(0) };
        if policy < 0 {
            return Err(errno());
        }
        let mut param = libc::sched_param { sched_priority: 0 };
        if unsafe { libc::sched_getparam(0, &mut param) } < 0 {
            return Err(errno());
        }
        Ok(Snapshot {
            policy,
            priority: param.sched_priority,
        })
    }
}
/// The only scheduling state captured by pre_exec: validated integers, no locks,
/// allocations, filesystem access, logging or environment lookups after fork.
#[derive(Clone, Copy)]
pub struct Prepared {
    mode: Mode,
    target: Snapshot,
}
#[derive(Clone, Copy, Debug)]
pub struct Report {
    pub actual: Snapshot,
    pub fallback_errno: i32,
    pub fatal_errno: i32,
}
impl Prepared {
    fn apply(self, scheduler: &mut impl Scheduler) -> Report {
        let mut report = Report {
            actual: Snapshot::NORMAL,
            fallback_errno: 0,
            fatal_errno: 0,
        };
        let mut expected = self.target;
        if let Err(error) = scheduler.set(self.target) {
            if self.mode != Mode::Auto {
                report.fatal_errno = error;
                return report;
            }
            report.fallback_errno = error;
            expected = Snapshot::NORMAL;
            if let Err(error) = scheduler.set(expected) {
                report.fatal_errno = error;
                return report;
            }
        }
        match scheduler.snapshot() {
            Ok(actual) if actual == expected => report.actual = actual,
            Ok(_) => report.fatal_errno = libc::EIO,
            Err(error) => report.fatal_errno = error,
        }
        report
    }
    /// Called only in the child, before exec. Failures reach the parent through
    /// its CLOEXEC pipe even when Command::spawn returns an error.
    pub fn apply_and_report(self, fd: RawFd) -> io::Result<()> {
        let report = self.apply(&mut Linux);
        report.write(fd)?;
        if report.fatal_errno != 0 {
            Err(io::Error::from_raw_os_error(report.fatal_errno))
        } else {
            Ok(())
        }
    }
}
impl Report {
    fn write(self, fd: RawFd) -> io::Result<()> {
        let mut bytes = [0u8; 16];
        for (chunk, value) in bytes.chunks_exact_mut(4).zip([
            self.actual.policy,
            self.actual.priority,
            self.fallback_errno,
            self.fatal_errno,
        ]) {
            chunk.copy_from_slice(&value.to_ne_bytes());
        }
        let mut sent = 0;
        while sent < bytes.len() {
            // SAFETY: fd is the inherited pipe's write end; bytes remain live
            // for the syscall. write and errno access are async-signal-safe.
            let result =
                unsafe { libc::write(fd, bytes[sent..].as_ptr().cast(), bytes.len() - sent) };
            if result < 0 {
                let error = errno();
                if error == libc::EINTR {
                    continue;
                }
                return Err(io::Error::from_raw_os_error(error));
            }
            if result == 0 {
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            sent += result as usize;
        }
        Ok(())
    }
    pub fn read(reader: &mut impl Read) -> io::Result<Self> {
        let mut bytes = [0u8; 16];
        reader.read_exact(&mut bytes)?;
        let fields: Vec<_> = bytes
            .chunks_exact(4)
            .map(|chunk| i32::from_ne_bytes(chunk.try_into().unwrap()))
            .collect();
        Ok(Self {
            actual: Snapshot {
                policy: fields[0],
                priority: fields[1],
            },
            fallback_errno: fields[2],
            fatal_errno: fields[3],
        })
    }
    pub fn log(self) {
        if self.fallback_errno != 0 {
            crate::log(
                "WARNING",
                format_args!(
                    "robot RT scheduling could not be applied: {}; using OTHER/0",
                    io::Error::from_raw_os_error(self.fallback_errno)
                ),
            );
        }
        crate::log(
            "INFO",
            format_args!(
                "robot initial scheduling: {}/{}; new threads inherit unless they override it",
                self.actual.name(),
                self.actual.priority
            ),
        );
    }
}
/// Normalize the supervisor before its journal/RioLog workers are created,
/// including manual launches from a shell with an inherited RT policy,
/// and pin the supervisor to Core 4 (CPU index 3).
pub fn configure_supervisor() -> io::Result<Snapshot> {
    let report = Prepared {
        mode: Mode::Off,
        target: Snapshot::NORMAL,
    }
    .apply(&mut Linux);
    if report.fatal_errno != 0 {
        return Err(io::Error::from_raw_os_error(report.fatal_errno));
    }
    let _ = pin_to_core(3);
    Ok(report.actual)
}

/// Pin current thread/process to a specific CPU core index.
pub fn pin_to_core(core_index: usize) -> io::Result<usize> {
    unsafe {
        let num_cpus = libc::sysconf(libc::_SC_NPROCESSORS_ONLN);
        if num_cpus <= 0 {
            return Ok(0);
        }
        let target = if (core_index as i64) < num_cpus {
            core_index
        } else {
            (num_cpus - 1) as usize
        };
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(target, &mut set);
        if libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(target)
    }
}

/// Distinguish full kernel preemption from a userspace scheduling policy.
pub fn kernel_rt(root: &Path) -> Option<bool> {
    if let Ok(value) = fs::read_to_string(root.join("sys/kernel/realtime")) {
        match value.trim() {
            "1" => return Some(true),
            "0" => return Some(false),
            _ => {}
        }
    }
    fs::read_to_string(root.join("proc/version"))
        .ok()
        .map(|version| version.split_whitespace().any(|word| word == "PREEMPT_RT"))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        calls: Vec<Snapshot>,
        state: Snapshot,
        rt_error: i32,
        normal_error: i32,
        query_error: i32,
        mismatch: bool,
    }
    impl Default for Fake {
        fn default() -> Self {
            Self {
                calls: vec![],
                state: Snapshot::NORMAL,
                rt_error: 0,
                normal_error: 0,
                query_error: 0,
                mismatch: false,
            }
        }
    }
    impl Scheduler for Fake {
        fn set(&mut self, target: Snapshot) -> Result<(), i32> {
            self.calls.push(target);
            let error = if target == Snapshot::NORMAL {
                self.normal_error
            } else {
                self.rt_error
            };
            if error != 0 {
                return Err(error);
            }
            self.state = target;
            Ok(())
        }
        fn snapshot(&self) -> Result<Snapshot, i32> {
            if self.query_error != 0 {
                Err(self.query_error)
            } else if self.mismatch {
                Ok(Snapshot::NORMAL)
            } else {
                Ok(self.state)
            }
        }
    }
    #[test]
    fn auto_applies_rr_and_verifies_it() {
        let mut fake = Fake::default();
        let report = Settings::default().prepare().unwrap().apply(&mut fake);
        assert_eq!(
            report.actual,
            Snapshot {
                policy: libc::SCHED_RR,
                priority: 10
            }
        );
        assert_eq!(report.fallback_errno, 0);
        assert_eq!(report.fatal_errno, 0);
    }
    #[test]
    fn auto_falls_back_to_normal_when_rt_is_denied() {
        let mut fake = Fake {
            rt_error: libc::EPERM,
            ..Fake::default()
        };
        let report = Settings::default().prepare().unwrap().apply(&mut fake);
        assert_eq!(report.actual, Snapshot::NORMAL);
        assert_eq!(report.fallback_errno, libc::EPERM);
        assert_eq!(report.fatal_errno, 0);
        assert_eq!(fake.calls.last(), Some(&Snapshot::NORMAL));
    }
    #[test]
    fn required_mode_never_silently_falls_back() {
        let mut fake = Fake {
            rt_error: libc::EPERM,
            ..Fake::default()
        };
        let report = Settings {
            mode: Mode::On,
            ..Settings::default()
        }
        .prepare()
        .unwrap()
        .apply(&mut fake);
        assert_eq!(report.fatal_errno, libc::EPERM);
        assert_eq!(report.fallback_errno, 0);
        assert_eq!(fake.calls.len(), 1);
    }
    #[test]
    fn off_explicitly_uses_normal_policy() {
        let mut fake = Fake {
            state: Snapshot {
                policy: libc::SCHED_FIFO,
                priority: 20,
            },
            ..Fake::default()
        };
        let report = Settings {
            mode: Mode::Off,
            ..Settings::default()
        }
        .prepare()
        .unwrap()
        .apply(&mut fake);
        assert_eq!(report.actual, Snapshot::NORMAL);
        assert_eq!(fake.calls, [Snapshot::NORMAL]);
    }
    #[test]
    fn failed_fallback_or_unverifiable_policy_is_fatal() {
        for mut fake in [
            Fake {
                rt_error: libc::EPERM,
                normal_error: libc::EACCES,
                ..Fake::default()
            },
            Fake {
                query_error: libc::EIO,
                ..Fake::default()
            },
            Fake {
                mismatch: true,
                ..Fake::default()
            },
        ] {
            let report = Settings::default().prepare().unwrap().apply(&mut fake);
            assert_ne!(report.fatal_errno, 0);
        }
    }
    #[test]
    fn fifo_uses_the_configured_priority() {
        let report = Settings {
            policy: Policy::Fifo,
            priority: 23,
            ..Settings::default()
        }
        .prepare()
        .unwrap()
        .apply(&mut Fake::default());
        assert_eq!(
            report.actual,
            Snapshot {
                policy: libc::SCHED_FIFO,
                priority: 23
            }
        );
    }
}
