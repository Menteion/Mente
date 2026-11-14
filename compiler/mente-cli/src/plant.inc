#[cfg_attr(miri, mutants::skip)]
pub fn planted_untested_inc(n: u32) -> u32 {
    n.wrapping_add(1)
}
