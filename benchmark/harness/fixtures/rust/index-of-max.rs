pub fn index_of_max(a: [i32; 8]) -> usize {
    let mut best = 0;
    for i in 1..8 {
        if a[i] > a[best] {
            best = i;
        }
    }
    best
}
