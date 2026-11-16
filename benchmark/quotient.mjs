// The one division of the modules the workflow policy reads as tokens (docs/design-review.md,
// untrusted-test-isolation). Those modules hold no `/` operator, so that every `/` in them starts a
// regular expression and no reading of their tokens takes one for the other; they divide through
// this function. The workflow policy holds this file's text.

/** The quotient of two numbers, as `/` gives it. */
export function quotient(dividend, divisor) {
  return dividend / divisor;
}
