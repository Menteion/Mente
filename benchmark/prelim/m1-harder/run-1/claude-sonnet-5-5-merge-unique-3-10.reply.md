I can't make progress on this one. Seven structurally different versions all produce this same report. Each returns the correct value, and by hand each leaves `out` as the examples state. The report never shows the `out` the interpreter ended up with, and without that I can't tell what differs. If the harness can print the actual `leaves` value, or a trace of the first write that differs, I can fix it from there.

This is my best version, the one I trust most. It uses a single forward pass and proves every index and the result type.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize[0..=8] = 0
  var j: usize[0..=8] = 0
  var k: usize[0..=16] = 0
  while i < 8 and j < 8 {
    val v1: i32 = if a[i] <= b[j] { a[i] } else { b[j] }
    guard k < 16 else {
      unreachable("at most 16 distinct values")
    }
    out[k] = v1
    k += 1
    while i < 8 {
      guard a[i] == v1 else {
        break
      }
      i += 1
    }
    while j < 8 {
      guard b[j] == v1 else {
        break
      }
      j += 1
    }
  }
  while i < 8 {
    val v2: i32 = a[i]
    guard k < 16 else {
      unreachable("at most 16 distinct values")
    }
    out[k] = v2
    k += 1
    while i < 8 {
      guard a[i] == v2 else {
        break
      }
      i += 1
    }
  }
  while j < 8 {
    val v3: i32 = b[j]
    guard k < 16 else {
      unreachable("at most 16 distinct values")
    }
    out[k] = v3
    k += 1
    while j < 8 {
      guard b[j] == v3 else {
        break
      }
      j += 1
    }
  }
  k
}
```