// Only registered groups may discard tests in a different harness.
macro_rules! other_test_group {
    (core) => {};
    (layout) => {};
    (motion) => {};
    (input) => {};
    (paint) => {};
    (selection) => {};
    (app) => {};
    (native) => {};
    ($unknown:ident) => {
        compile_error!(concat!("unknown unit-test group: ", stringify!($unknown)));
    };
}
