---
id: first-ok
milestone: M7
core: true
set: public
kind: solve
---

# First successful response

## Task

Request every URL in `urls` concurrently with `Net.get`, and return the body of the first successful response to arrive. Return `AllFailed` when every request fails, at once for an empty `urls`, and `TimedOut` when `timeout_ms` milliseconds pass before any success arrives and before every request has failed. Return as soon as the result is known, without waiting for the other requests. In every test the responses that decide the result arrive at least 100 ms apart from each other and from the timeout.

## Types

    enum FetchError { AllFailed, TimedOut }

## Signature

    first_ok(urls: slice<string>, timeout_ms: u64) -> result<string, FetchError> uses Net, Clock

## Examples

    given Net.get("https://e.test/slow") => ok("slow") after 300ms
    given Net.get("https://e.test/fast") => ok("fast") after 100ms
    first_ok(["https://e.test/slow", "https://e.test/fast"], 1000) == ok("fast")
    then elapsed under 250ms

    given Net.get("https://e.test/a") => err(Refused) after 100ms
    given Net.get("https://e.test/b") => err(Timeout) after 200ms
    first_ok(["https://e.test/a", "https://e.test/b"], 1000) == err(AllFailed)
    then elapsed under 350ms

    given Net.get("https://e.test/a") => ok("late") after 500ms
    first_ok(["https://e.test/a"], 200) == err(TimedOut)
    then elapsed under 350ms

## Hidden tests

    given Net.get("https://e.test/a") => err(Refused) after 100ms
    given Net.get("https://e.test/b") => ok("b") after 300ms
    first_ok(["https://e.test/a", "https://e.test/b"], 1000) == ok("b")
    then elapsed under 450ms

    given Net.get("https://e.test/a") => err(Refused) after 100ms
    given Net.get("https://e.test/b") => err(Timeout) after 400ms
    first_ok(["https://e.test/a", "https://e.test/b"], 200) == err(TimedOut)
    then elapsed under 350ms

    first_ok([], 1000) == err(AllFailed)
    then elapsed under 100ms

    given Net.get("https://e.test/a") => ok("a") after 200ms
    given Net.get("https://e.test/b") => ok("b") after 400ms
    given Net.get("https://e.test/c") => err(Timeout) after 100ms
    first_ok(["https://e.test/b", "https://e.test/c", "https://e.test/a"], 1000) == ok("a")
    then elapsed under 350ms

    first_ok(["https://e.test/missing"], 1000) == err(AllFailed)
    then elapsed under 150ms

    given Net.get("https://e.test/a") => ok("a") after 600ms
    given Net.get("https://e.test/b") => err(Refused) after 100ms
    first_ok(["https://e.test/a", "https://e.test/b"], 300) == err(TimedOut)
    then elapsed under 450ms
