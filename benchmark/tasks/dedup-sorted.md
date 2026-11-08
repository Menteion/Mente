---
id: dedup-sorted
milestone: M4
core: true
set: public
kind: solve
---

# Remove adjacent duplicates, generically

## Task

Return the values of `xs` with every run of equal adjacent values reduced to a single value, keeping their order. The function is generic over the element type, which needs only equality; the tests call it with integers and with strings.

## Signature

    dedup_sorted<T: Eq>(xs: slice<T>) -> list<T>

## Examples

    dedup_sorted<i32>([1, 1, 2, 3, 3, 3]) == [1, 2, 3]
    dedup_sorted<string>(["a", "a", "b"]) == ["a", "b"]
    dedup_sorted<i32>([]) == []

## Hidden tests

    dedup_sorted<i32>([1, 2, 1, 2]) == [1, 2, 1, 2]
    dedup_sorted<string>(["x"]) == ["x"]
    dedup_sorted<i32>([5, 5, 5, 5]) == [5]
    dedup_sorted<string>(["", "", "a", ""]) == ["", "a", ""]
    dedup_sorted<i32>([-1, -1, 0, 0, 1, 1]) == [-1, 0, 1]
    dedup_sorted<string>(["A", "a", "a", "A"]) == ["A", "a", "A"]
    dedup_sorted<string>([]) == []
    dedup_sorted<i32>([-2147483648, -2147483648, 2147483647]) == [-2147483648, 2147483647]
