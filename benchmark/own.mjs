// The one way a module of a job that holds a secret reads or writes a member by a computed key
// (docs/design-review.md, untrusted-test-isolation). It reads only an object's own property and
// defines only an own property, so that no key, however it is made, reaches what a prototype
// holds: `constructor`, and through it Function, above all. The key becomes a property key once,
// and that one is both checked and used: a key whose `toString` answers otherwise the second time
// would otherwise pass the check as one name and read another. The workflow policy refuses every
// other computed member access in those modules, and holds this file's text.

/** A key as the property key it names, made once. */
const keyOf = (key) => (typeof key === "symbol" ? key : String(key));

/** The own property `key` of `object`, or undefined: never one its prototypes hold. */
export function own(object, key) {
  const k = keyOf(key);
  return object !== null && object !== undefined && Object.hasOwn(Object(object), k) ? object[k] : undefined;
}

/** Defines `key` as an own property of `object`, whatever its prototypes hold, and returns `value`. */
export function setOwn(object, key, value) {
  Object.defineProperty(object, keyOf(key), { value, writable: true, enumerable: true, configurable: true });
  return value;
}
