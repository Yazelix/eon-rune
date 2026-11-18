//! Scheduler contention for callback and interaction timing. Samples use syscalls; a callback
//! that was descheduled on a loaded machine gets the benefit of the doubt.

#[cfg(debug_assertions)]
use super::{STALL_CPU_RATIO, STALL_MS};

#[cfg(unix)]
static CORES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

/// Resolve affinity/cgroup limits during runtime construction, never in a timing sample.
pub(super) fn prepare() {
    #[cfg(unix)]
    CORES.get_or_init(|| std::thread::available_parallelism().map_or(1, usize::from));
}

#[cfg(debug_assertions)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Switches {
    voluntary: u64,
    involuntary: u64,
}

/// Both kinds of switch can incur scheduler wait, including a sleep's wake-up.
#[cfg(debug_assertions)]
pub(crate) fn snapshot() -> Option<Switches> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: `usage` points to writable storage for the requested thread's usage.
        if unsafe { libc::getrusage(libc::RUSAGE_THREAD, usage.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: a successful getrusage initialized the structure.
        let usage = unsafe { usage.assume_init() };
        Some(Switches {
            voluntary: usage.ru_nvcsw as u64,
            involuntary: usage.ru_nivcsw as u64,
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        None
    }
}

/// Equal counters show no scheduler wait seen by this thread; a switch's duration is unknown.
/// Interrupt and hypervisor steal time are not measured.
#[cfg(debug_assertions)]
pub(crate) fn wait_ms(start: Option<Switches>, end: Option<Switches>) -> Option<f64> {
    start
        .zip(end)
        .filter(|(start, end)| start == end)
        .map(|_| 0.0)
}

#[cfg(unix)]
fn load() -> Option<f64> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut info = std::mem::MaybeUninit::<libc::sysinfo>::uninit();
        // SAFETY: `info` points to writable storage for the system information.
        if unsafe { libc::sysinfo(info.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: a successful sysinfo initialized the structure; loads use 16 fractional bits.
        Some(unsafe { info.assume_init() }.loads[0] as f64 / 65536.0)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let mut load = [0f64; 1];
        // SAFETY: `load` is a writable array of the one sample requested.
        (unsafe { libc::getloadavg(load.as_mut_ptr(), 1) } == 1).then_some(load[0])
    }
}

/// One-minute load at or above the initialized core count. Refresh on demand, at most every
/// five seconds; failed/unprepared samples are conservatively loaded. No idle timer.
pub(crate) fn overloaded() -> bool {
    #[cfg(unix)]
    {
        use std::{
            sync::Mutex,
            time::{Duration, Instant},
        };
        static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
        let Some(&cores) = CORES.get() else {
            return true;
        };
        let mut cache = CACHE.lock().unwrap();
        if let Some((at, busy)) = *cache
            && at.elapsed() < Duration::from_secs(5)
        {
            return busy;
        }
        let busy = load().is_none_or(|load| load >= cores as f64);
        *cache = Some((Instant::now(), busy));
        busy
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// The wall ms a callback was blocked, or `None` when it wasn't. `wait` is the run-queue time
/// it spent when known; without it, a `loaded` machine gets the benefit of the doubt.
#[cfg(debug_assertions)]
pub(crate) fn blocked_ms(wall: f64, cpu: f64, wait: Option<f64>, loaded: bool) -> Option<f64> {
    if wait.is_none() && loaded {
        return None;
    }
    let blocked = wall - wait.unwrap_or(0.0).clamp(0.0, wall);
    (blocked > STALL_MS && blocked > STALL_CPU_RATIO * cpu).then_some(blocked)
}

#[cfg(test)]
test_group! { native =>
#[cfg(all(test, debug_assertions))]
#[path = "runq_tests.rs"]
mod tests;
}
