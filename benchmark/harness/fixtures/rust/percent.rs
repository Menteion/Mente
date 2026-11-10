pub fn percent(part: u32, whole: u32) -> u32 {
    (part as u64 * 100 / whole as u64) as u32
}
