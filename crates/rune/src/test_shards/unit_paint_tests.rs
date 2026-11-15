include!("groups.rs");

macro_rules! test_group {
    (paint => $($item:item)*) => { $($item)* };
    ($other:ident => $($item:item)*) => { other_test_group!($other); };
}
