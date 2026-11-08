---
id: fetch-lengths
milestone: M4
core: true
set: public
kind: solve
---

# Fetch pages and measure them

## Task

Fetch every URL in `urls` with `Net.get`, one after another in order, and return the length in bytes of each response body, in the same order. Stop at the first request that fails and return its error, making no request after it.

## Signature

    fetch_lengths(urls: slice<string>) -> result<list<u64>, NetError> uses Net

## Examples

    given Net.get("https://a.test/1") => ok("hello")
    given Net.get("https://a.test/2") => ok("hi")
    fetch_lengths(["https://a.test/1", "https://a.test/2"]) == ok([5, 2])
    then Net.get calls ["https://a.test/1", "https://a.test/2"]

    given Net.get("https://a.test/1") => err(Timeout)
    fetch_lengths(["https://a.test/1", "https://a.test/2"]) == err(Timeout)
    then Net.get calls ["https://a.test/1"]

    fetch_lengths([]) == ok([])
    then Net.get calls []

## Hidden tests

    given Net.get("https://b.test/x") => ok("")
    fetch_lengths(["https://b.test/x"]) == ok([0])
    then Net.get calls ["https://b.test/x"]

    given Net.get("https://b.test/x") => ok("abc")
    given Net.get("https://b.test/y") => err(Refused)
    fetch_lengths(["https://b.test/x", "https://b.test/y", "https://b.test/x"]) == err(Refused)
    then Net.get calls ["https://b.test/x", "https://b.test/y"]

    given Net.get("https://b.test/x") => ok("abc")
    fetch_lengths(["https://b.test/x", "https://b.test/missing"]) == err(NotFound)
    then Net.get calls ["https://b.test/x", "https://b.test/missing"]

    given Net.get("https://b.test/r") => ok("first"), ok("second!"), err(Timeout)
    fetch_lengths(["https://b.test/r", "https://b.test/r", "https://b.test/r", "https://b.test/r"]) == err(Timeout)
    then Net.get calls ["https://b.test/r", "https://b.test/r", "https://b.test/r"]

    given Net.get("https://b.test/big") => ok("012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789")
    fetch_lengths(["https://b.test/big", "https://b.test/big"]) == ok([300, 300])
    then Net.get calls ["https://b.test/big", "https://b.test/big"]

    given Net.get("https://b.test/line") => ok("a\nb\tc")
    fetch_lengths(["https://b.test/line"]) == ok([5])
    then Net.get calls ["https://b.test/line"]
