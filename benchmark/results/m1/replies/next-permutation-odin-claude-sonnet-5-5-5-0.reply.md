Standard algorithm: find the largest `i` with `a[i] < a[i+1]`. If there is none, reverse the whole array and return false. Otherwise find the largest `j > i` with `a[j] > a[i]`, swap them, and reverse the suffix after `i`.

Indexes are `int`. The loops use `for` with explicit `int` counters, and the array is accessed through the pointer as `a^[i]`.

```odin
next_permutation :: proc(a: ^[6]i32) -> bool {
	n :: 6
	pivot := -1
	for i := n - 2; i >= 0; i -= 1 {
		if a^[i] < a^[i + 1] {
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