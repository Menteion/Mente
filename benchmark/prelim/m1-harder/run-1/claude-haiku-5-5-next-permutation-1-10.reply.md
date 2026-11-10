I'm not going to submit another version of this function, because the same code has now failed the same way across many attempts.

The `leaves` failure comes from how the examples call the function, not from its body. The examples pass array literals, such as `mut [1, 2, 3, 4, 5, 6]`, into a `mut` parameter. The card says a `mut` parameter "receives a place", and a literal isn't a place, so the value the call copies back has nowhere to go. A rewrite can't change that.

To go forward, one of these has to change, and it's your decision:

1. **Change the examples** to bind the array to a `var` and pass that, for example `var v: [6]i32 = [1, 2, 3, 4, 5, 6]` followed by `next_permutation(mut v)`, then check `v`. The function would stay as it is.
2. **Confirm that the reference interpreter's `mut` write-back works** with a `var` argument. The `set_first` probe from earlier would settle that. If it fails, the spec or interpreter needs fixing, not the function.

Tell me which you want, or send the probe result, and I'll write the matching version.