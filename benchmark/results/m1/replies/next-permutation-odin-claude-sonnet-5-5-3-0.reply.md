Standard algorithm: find the largest `i` with `a[i] < a[i+1]`. If none exists, reverse the whole array and return false. Otherwise find the largest `j > i` with `a[j] > a[i]`, swap them, and reverse the suffix after `i`.

Indexes are `int`, so I use `int` loop variables and avoid underflow by checking `i >= 0` with signed ints. Example 3 checks out: `1 3 5 4 2 0` gives `i=1` (3<5), `j=3` (4>3), the swap gives `1 4 5 3 2 0`, and reversing the suffix gives `1 4 0 2 3 5`, which matches.

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