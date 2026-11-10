pub fn lower_bound(a: [i32; 16], x: i32) -> usize {
    a.iter().position(|&v| v >= x).unwrap_or(16)
}
