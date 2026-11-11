// The same shapes in Rust, for comparison (README.md): check.sh compiles this file to assembly
// with `rustc -C opt-level=3` and prints each function. `rust_forward` differs from
// `rust_plain_forward` only by `?`, as `pair_forward` differs from `plain_forward` in the
// spike; `rust_middle` and `rust_outer` follow the spike's chain. Arithmetic wraps, since the
// spike's Cranelift code leaves out overflow checks.
#![crate_type = "lib"]

#[inline(never)]
#[no_mangle]
pub fn rust_leaf(x: i64) -> Result<i64, u64> {
    if x < 0 {
        return Err(1000u64.wrapping_sub(x as u64));
    }
    Ok(x.wrapping_add(x))
}

#[inline(never)]
#[no_mangle]
pub fn rust_forward(x: i64) -> Result<i64, u64> {
    let v = rust_leaf(x)?;
    Ok(v.wrapping_add(1))
}

#[inline(never)]
#[no_mangle]
pub fn rust_plain_leaf(x: i64) -> i64 {
    x.wrapping_add(x)
}

#[inline(never)]
#[no_mangle]
pub fn rust_plain_forward(x: i64) -> i64 {
    rust_plain_leaf(x).wrapping_add(1)
}

#[inline(never)]
#[no_mangle]
pub fn rust_middle(x: i64) -> Result<i64, u64> {
    let v = rust_leaf(x)?;
    if v > 100 {
        return Err(2000u64.wrapping_add(v as u64));
    }
    Ok(v.wrapping_add(1))
}

#[inline(never)]
#[no_mangle]
pub fn rust_outer(x: i64) -> Result<i64, u64> {
    let v = rust_middle(x)?;
    if (v as u64) % 5 == 0 {
        return Err(3000u64.wrapping_add(v as u64));
    }
    Ok(v.wrapping_mul(10))
}
