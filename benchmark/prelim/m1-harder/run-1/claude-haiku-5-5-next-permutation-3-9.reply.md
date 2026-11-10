I'm not going to resubmit the function. This is the same failure as before, and my earlier submissions were identical in substance, so another copy would not change the result.

I've checked the function by hand against all three examples, and each produces the expected array. The `leaves` failures come from the harness's handling of array literals passed as `mut` arguments, and I can't change that from the function body.

To move forward, I need the result of the `probe` with a named `var` argument, or the harness's rule for reading `leaves` on literal arguments. Then either the examples or the interpreter can be fixed, and I can check the function against them.