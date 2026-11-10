I'm not going to resubmit the function again. The reference output hasn't changed across these rounds, and I don't have evidence that points to a different function. The `got` values match, and I traced both examples by hand, so the function writes the expected elements.

The failure is in the `leaves` check, which is a harness problem, not a logic problem:

1. Both examples pass a literal, such as `mut [0, 0, ...]`, as the `mut` argument. The card says a `mut` parameter receives a place, so a literal isn't a valid argument. Please rewrite each example to bind the array to a `var` and pass `mut` of that variable, then compare that variable afterward.
2. If the examples still fail after that, send me the full `leaves` array the interpreter returns for the first example. That will show exactly which elements differ.