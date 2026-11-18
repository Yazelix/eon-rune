use super::{Stall, Watch};
use crate::tooling::test_alloc::allocs;
use std::time::Duration;

#[test]
fn short_callback_timing_does_not_allocate() {
    let _watch = Watch::default();
    let before = allocs();
    let stall = Stall::start();
    stall.end(|| panic!("a short callback must not format a warning"));
    assert_eq!(allocs() - before, 0, "short callback timing allocations");
}

#[test]
fn long_cpu_bound_callback_timing_does_not_allocate() {
    let _watch = Watch::default();
    let before = allocs();
    let mut stall = Stall::start();
    let (wall, cpu, _) = stall
        .start
        .as_mut()
        .expect("default warnings time callbacks");
    // Exercise the long-path samples without warning formatting or a wall-clock sleep.
    *wall -= Duration::from_millis(30);
    *cpu -= 30.0;
    stall.end(|| panic!("a CPU-bound callback must not format a warning"));
    assert_eq!(allocs() - before, 0, "long callback timing allocations");
}
