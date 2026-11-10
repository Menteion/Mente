Standard algorithm: find the largest `i` with `a[i] < a[i+1]`. If none exists, reverse the whole array and return false. Otherwise find the largest `j > i` with `a[j] > a[i]`, swap them, and reverse the suffix after `i`.

Example 3 check: `[1,3,5,4,2,0]` gives `i=1` (3<5), `j=4` (value 2 > 3? no). The largest `j` with `a[j] > 3` is `j=3` (value 4). Swapping gives `[1,4,5,3,2,0]`, and reversing the suffix after index 1 gives `[1,4,0,2,3,5]`, which matches.

I use `int` indices and a `pivot` of -1 to mean none found.

```odin
next_permutation :: proc(a: ^[6]i32) -> bool {
	n :: 6
	pivot := -1
	for i := n - 2; i >= 0; i -= 1 {
		if a^[i] < a^[i+1] {
			pivot = i
			break
		}
	}
	if pivot < 0 {
		lo := 0
		hi := n - 1
		for lo < hi {
			t := a^[lo]
			a^[lo] = a^[hi]
			a^[hi] = t
			lo += 1
			hi -= 1
		}
		return false
	}
	j := n - 1
	for a^[j] <= a^[pivot] {
		j -= 1
	}
	t := a^[pivot]
	a^[pivot] = a^[j]
	a^[j] = t
	lo := pivot + 1
	hi := n - 1
	for lo < hi {
		u := a^[lo]
		a^[lo] = a^[hi]
		a^[hi] = u
		lo += 1
		hi -= 1
	}
	return true
}
```