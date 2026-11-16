use super::*;

#[test]
fn a_wait_for_a_core_is_not_blocking() {
    assert_eq!(blocked_ms(36.0, 0.5, Some(35.0), false), None);
    assert_eq!(blocked_ms(36.0, 0.5, Some(10.0), false), Some(26.0));
}

#[test]
fn real_blocking_still_warns() {
    assert_eq!(blocked_ms(30.0, 0.0, Some(0.0), false), Some(30.0));
    assert_eq!(blocked_ms(30.0, 0.0, None, false), Some(30.0));
}

#[test]
fn computing_is_not_blocking() {
    assert_eq!(blocked_ms(50.0, 40.0, Some(0.0), false), None);
}

#[test]
fn without_a_wait_a_loaded_machine_is_forgiven() {
    assert_eq!(blocked_ms(36.0, 0.5, None, true), None);
    assert_eq!(blocked_ms(36.0, 0.5, Some(0.0), true), Some(36.0));
}

#[test]
fn the_wait_never_exceeds_the_wall() {
    assert_eq!(blocked_ms(20.0, 0.0, Some(99.0), false), None);
}

#[test]
fn switched_or_unavailable_samples_do_not_claim_zero_wait() {
    let start = Switches {
        voluntary: 10,
        involuntary: 20,
    };
    assert_eq!(wait_ms(Some(start), Some(start)), Some(0.0));
    for end in [
        Switches {
            voluntary: 11,
            ..start
        },
        Switches {
            involuntary: 21,
            ..start
        },
        Switches {
            voluntary: 9,
            ..start
        },
        Switches {
            involuntary: 19,
            ..start
        },
    ] {
        let wait = wait_ms(Some(start), Some(end));
        assert_eq!(wait, None);
        assert_eq!(blocked_ms(30.0, 0.0, wait, true), None);
        assert_eq!(blocked_ms(30.0, 0.0, wait, false), Some(30.0));
    }
    assert_eq!(wait_ms(None, Some(start)), None);
    assert_eq!(wait_ms(Some(start), None), None);
    assert_eq!(wait_ms(None, None), None);
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn sleeping_has_unknown_wake_up_wait() {
    let start = snapshot().expect("the calling thread has resource usage");
    std::thread::sleep(std::time::Duration::from_millis(10));
    let end = snapshot().unwrap();
    assert!(end.voluntary > start.voluntary);
    assert_eq!(wait_ms(Some(start), Some(end)), None);
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn the_load_syscall_reports_a_finite_nonnegative_value() {
    let load = load().expect("sysinfo is available");
    assert!(load.is_finite() && load >= 0.0, "{load}");
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn first_and_repeated_samples_and_load_queries_allocate_nothing() {
    prepare();
    let before = crate::tooling::test_alloc::allocs();
    for _ in 0..100 {
        assert!(snapshot().is_some());
        assert!(load().is_some());
        std::hint::black_box(overloaded());
    }
    assert_eq!(crate::tooling::test_alloc::allocs() - before, 0);
}
