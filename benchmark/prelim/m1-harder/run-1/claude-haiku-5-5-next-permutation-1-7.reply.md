I'm going to stop resubmitting this. It's the same function, and this is the fourth identical `leaves` failure since I last changed the code. Another rewrite would be a guess.

What I need is what the interpreter does with a `mut` array. Ask the reference interpreter to run this probe on a `var` bound to an array, not a literal:

```text
pub fn set_first(mut a: [6]i32) -> bool {
  a[0] = 9
  true
}
```

Call it as `var v: [6]i32 = [1, 2, 3, 4, 5, 6]`, then `set_first(mut v)`, and report what `v` holds afterward.

- If `v[0]` is `9`, write-back works, and the examples' literal arguments are the problem. Ask the examples to bind a `var` first, and I'll write the function to match.
- If `v` is unchanged, the `mut` write-back itself is broken or behaves differently than the card says. Then no function body can meet these examples, and the spec or the harness needs fixing.

Send me that result, or the value the interpreter reports for the first example's array after the call, and I'll revise the function from there.