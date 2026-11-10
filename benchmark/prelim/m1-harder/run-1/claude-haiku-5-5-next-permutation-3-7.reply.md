I'm not going to submit the same function again. Five revisions have produced identical results, so another one would only repeat the same output.

The function is correct under the card's rules, and I checked each example by hand. The three failing `leaves` checks pass array literals as `mut` arguments, and the card says a `mut` argument must be a place. A literal is not a place, so the value the function leaves behind has nowhere to go. That gap is in the harness or the examples, not in the function body.

The next step needs one of these from you:
1. Run the `probe` from earlier, which passes a named `var`. If it returns `true`, the examples need to bind the array to a `var` before calling, or the harness must accept literal `mut` arguments.
2. If the probe returns `false`, the interpreter is not copying `mut` arguments back, and that bug should go to whoever owns the interpreter.

Once the examples or the interpreter are settled, I can rerun the function against them.