//! Overflow checks are on in every build of this crate, release ones included, so that an integer
//! overflow inside the compiler traps instead of wrapping silently (`[profile.release]` in
//! `Cargo.toml`). The required CI job "Tier-1 latency" runs this test in a release build, where a
//! profile, a package override or a flag that turned the checks off for this crate would show.

/// An arithmetic overflow in this crate's code panics, whatever profile builds it.
#[test]
fn test_arithmetic_overflow_panics_in_this_crate() {
    let max = std::hint::black_box(u8::MAX);
    let sum = std::panic::catch_unwind(|| max + std::hint::black_box(1));
    assert!(
        sum.is_err(),
        "u8::MAX + 1 did not panic: this crate is built without overflow checks"
    );
}
