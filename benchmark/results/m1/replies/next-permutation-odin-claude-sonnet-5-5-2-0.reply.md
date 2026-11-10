Standard algorithm: find the largest `i` with `a[i] < a[i+1]`. If none exists, reverse the whole array and return `false`. Otherwise find the largest `j > i` with `a[j] > a[i]`, swap `a[i]` and `a[j]`, reverse the suffix after `i`, and return `true`.

Indexes are `int`, so `i := n - 2` can go to -1 safely. Because `a` is a pointer, elements are accessed with `a^[i]`.

```odin
next_permutation :: proc(a: ^[6]i32) -> bool {
	n := len(a^)
	i := n - 2
	for i >= 0 && a^[i] >= a^[i+1] {
		i -= 1
	}
	if i >= 0 {
		j := n - 1
		for a^[j] <= a^[i] {
			j -= 1
		}
		t := a^[i]
		a^[i] = a^[j]
		a^[j] = t
	}
	lo := i + 1
	hi := n - 1
	for lo < hi {
		t := a^[lo]
		a^[lo] = a^[hi]
		a^[hi] = t
		lo += 1
		hi -= 1
	}
	return i >= 0
}
```