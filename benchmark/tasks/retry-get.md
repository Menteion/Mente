---
id: retry-get
milestone: M4
core: true
set: public
kind: solve
---

# Fetch with retries and backoff

## Task

Fetch `url` with `Net.get`, trying at most `attempts` times; `attempts` is at least 1. Return the first successful response. A `NotFound` error is final: return it at once. After a `Timeout` or `Refused` error, sleep with `Clock.sleep_ms` before the next attempt: 100 ms before the second attempt, 200 ms before the third, and twice as long before each later one. When the last attempt fails, return its error without sleeping.

## Signature

    retry_get(url: string, attempts: u32) -> result<string, NetError> uses Net, Clock

## Examples

    given Net.get("https://c.test/") => ok("data")
    retry_get("https://c.test/", 3) == ok("data")
    then Net.get calls ["https://c.test/"]
    then Clock.sleep_ms calls []

    given Net.get("https://c.test/") => err(Timeout), ok("data")
    retry_get("https://c.test/", 3) == ok("data")
    then Net.get calls ["https://c.test/", "https://c.test/"]
    then Clock.sleep_ms calls [100]

    given Net.get("https://c.test/") => err(NotFound), ok("data")
    retry_get("https://c.test/", 3) == err(NotFound)
    then Net.get calls ["https://c.test/"]
    then Clock.sleep_ms calls []

## Hidden tests

    given Net.get("https://c.test/") => err(Timeout), err(Refused), err(Timeout), ok("x")
    retry_get("https://c.test/", 3) == err(Timeout)
    then Net.get calls ["https://c.test/", "https://c.test/", "https://c.test/"]
    then Clock.sleep_ms calls [100, 200]

    given Net.get("https://c.test/") => err(Timeout), err(Refused), err(Timeout), ok("x")
    retry_get("https://c.test/", 4) == ok("x")
    then Net.get calls ["https://c.test/", "https://c.test/", "https://c.test/", "https://c.test/"]
    then Clock.sleep_ms calls [100, 200, 400]

    given Net.get("https://c.test/") => err(Timeout), err(NotFound), ok("x")
    retry_get("https://c.test/", 5) == err(NotFound)
    then Net.get calls ["https://c.test/", "https://c.test/"]
    then Clock.sleep_ms calls [100]

    given Net.get("https://c.test/") => err(Timeout)
    retry_get("https://c.test/", 1) == err(Timeout)
    then Net.get calls ["https://c.test/"]
    then Clock.sleep_ms calls []

    given Net.get("https://c.test/") => err(Refused)
    retry_get("https://c.test/", 6) == err(Refused)
    then Net.get calls ["https://c.test/", "https://c.test/", "https://c.test/", "https://c.test/", "https://c.test/", "https://c.test/"]
    then Clock.sleep_ms calls [100, 200, 400, 800, 1600]

    retry_get("https://c.test/unknown", 3) == err(NotFound)
    then Net.get calls ["https://c.test/unknown"]
    then Clock.sleep_ms calls []

    given Net.get("https://c.test/") => ok("")
    retry_get("https://c.test/", 1) == ok("")
    then Net.get calls ["https://c.test/"]
    then Clock.sleep_ms calls []
