---
id: rate-limited-fetch
milestone: M7
core: true
set: public
kind: solve
---

# Fetch with a concurrency limit

## Task

Fetch every URL in `urls` with `Net.get` and return every result in the order of `urls`. At most `max_in_flight` requests may be running at any moment, and as many as that must run whenever enough requests remain: in the tests each response takes 200 ms, and the call must finish within the time the limit allows. `max_in_flight` is at least 1.

## Signature

    rate_limited_fetch(urls: slice<string>, max_in_flight: u32) -> list<result<string, NetError>> uses Net

## Examples

    given Net.get("https://f.test/0") => ok("r0") after 200ms
    given Net.get("https://f.test/1") => ok("r1") after 200ms
    given Net.get("https://f.test/2") => ok("r2") after 200ms
    given Net.get("https://f.test/3") => ok("r3") after 200ms
    rate_limited_fetch(["https://f.test/0", "https://f.test/1", "https://f.test/2", "https://f.test/3"], 2) == [ok("r0"), ok("r1"), ok("r2"), ok("r3")]
    then Net.get calls in any order ["https://f.test/0", "https://f.test/1", "https://f.test/2", "https://f.test/3"]
    then Net.peak_in_flight at most 2
    then elapsed under 550ms

    rate_limited_fetch([], 3) == []
    then Net.get calls in any order []
    then elapsed under 100ms

## Hidden tests

    given Net.get("https://f.test/0") => ok("r0") after 200ms
    given Net.get("https://f.test/1") => ok("r1") after 200ms
    given Net.get("https://f.test/2") => ok("r2") after 200ms
    rate_limited_fetch(["https://f.test/0", "https://f.test/1", "https://f.test/2"], 1) == [ok("r0"), ok("r1"), ok("r2")]
    then Net.get calls in any order ["https://f.test/0", "https://f.test/1", "https://f.test/2"]
    then Net.peak_in_flight at most 1
    then elapsed under 750ms

    given Net.get("https://f.test/0") => ok("r0") after 200ms
    given Net.get("https://f.test/1") => ok("r1") after 200ms
    given Net.get("https://f.test/2") => ok("r2") after 200ms
    given Net.get("https://f.test/3") => ok("r3") after 200ms
    given Net.get("https://f.test/4") => ok("r4") after 200ms
    rate_limited_fetch(["https://f.test/0", "https://f.test/1", "https://f.test/2", "https://f.test/3", "https://f.test/4"], 5) == [ok("r0"), ok("r1"), ok("r2"), ok("r3"), ok("r4")]
    then Net.get calls in any order ["https://f.test/0", "https://f.test/1", "https://f.test/2", "https://f.test/3", "https://f.test/4"]
    then Net.peak_in_flight at most 5
    then elapsed under 350ms

    given Net.get("https://f.test/0") => ok("r0") after 200ms
    given Net.get("https://f.test/1") => err(Timeout) after 200ms
    given Net.get("https://f.test/2") => ok("r2") after 200ms
    given Net.get("https://f.test/3") => ok("r3") after 200ms
    given Net.get("https://f.test/4") => err(Timeout) after 200ms
    given Net.get("https://f.test/5") => ok("r5") after 200ms
    rate_limited_fetch(["https://f.test/0", "https://f.test/1", "https://f.test/2", "https://f.test/3", "https://f.test/4", "https://f.test/5"], 4) == [ok("r0"), err(Timeout), ok("r2"), ok("r3"), err(Timeout), ok("r5")]
    then Net.get calls in any order ["https://f.test/0", "https://f.test/1", "https://f.test/2", "https://f.test/3", "https://f.test/4", "https://f.test/5"]
    then Net.peak_in_flight at most 4
    then elapsed under 550ms

    given Net.get("https://f.test/0") => ok("r0") after 200ms
    given Net.get("https://f.test/1") => ok("r1") after 200ms
    rate_limited_fetch(["https://f.test/0", "https://f.test/1"], 10) == [ok("r0"), ok("r1")]
    then Net.get calls in any order ["https://f.test/0", "https://f.test/1"]
    then Net.peak_in_flight at most 2
    then elapsed under 350ms

    given Net.get("https://f.test/0") => ok("r0") after 200ms
    given Net.get("https://f.test/1") => ok("r1") after 200ms
    given Net.get("https://f.test/2") => ok("r2") after 200ms
    given Net.get("https://f.test/3") => ok("r3") after 200ms
    given Net.get("https://f.test/4") => ok("r4") after 200ms
    given Net.get("https://f.test/5") => ok("r5") after 200ms
    given Net.get("https://f.test/6") => ok("r6") after 200ms
    given Net.get("https://f.test/7") => ok("r7") after 200ms
    given Net.get("https://f.test/8") => ok("r8") after 200ms
    rate_limited_fetch(["https://f.test/0", "https://f.test/1", "https://f.test/2", "https://f.test/3", "https://f.test/4", "https://f.test/5", "https://f.test/6", "https://f.test/7", "https://f.test/8"], 3) == [ok("r0"), ok("r1"), ok("r2"), ok("r3"), ok("r4"), ok("r5"), ok("r6"), ok("r7"), ok("r8")]
    then Net.get calls in any order ["https://f.test/0", "https://f.test/1", "https://f.test/2", "https://f.test/3", "https://f.test/4", "https://f.test/5", "https://f.test/6", "https://f.test/7", "https://f.test/8"]
    then Net.peak_in_flight at most 3
    then elapsed under 750ms
