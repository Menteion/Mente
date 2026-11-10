pub fn merge_unique(a: [i32; 8], b: [i32; 8], out: &mut [i32; 16]) -> usize {
    let (mut i, mut j, mut k) = (0, 0, 0);
    let mut last: Option<i32> = None;
    while i < 8 || j < 8 {
        let v = if j >= 8 || (i < 8 && a[i] <= b[j]) {
            i += 1;
            a[i - 1]
        } else {
            j += 1;
            b[j - 1]
        };
        if last != Some(v) {
            out[k] = v;
            k += 1;
            last = Some(v);
        }
    }
    k
}
