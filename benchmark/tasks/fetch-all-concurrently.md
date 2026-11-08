---
id: fetch-all-concurrently
milestone: M7
core: true
set: public
kind: solve
---

# Fetch pages concurrently

## Task

Fetch every URL in `urls` with `Net.get` and return every result, success or error, in the order of `urls`. The requests must run concurrently: in the tests each response takes up to 200 ms, and the whole call must finish well within the time that requests made one after another would take.

## Signature

    fetch_all_concurrently(urls: slice<string>) -> list<result<string, NetError>> uses Net

## Examples

    given Net.get("https://d.test/1") => ok("one") after 200ms
    given Net.get("https://d.test/2") => ok("two") after 200ms
    given Net.get("https://d.test/3") => ok("three") after 200ms
    fetch_all_concurrently(["https://d.test/1", "https://d.test/2", "https://d.test/3"]) == [ok("one"), ok("two"), ok("three")]
    then Net.get calls in any order ["https://d.test/1", "https://d.test/2", "https://d.test/3"]
    then elapsed under 500ms

    fetch_all_concurrently([]) == []
    then Net.get calls in any order []
    then elapsed under 100ms

## Hidden tests

    given Net.get("https://d.test/1") => ok("one") after 200ms
    given Net.get("https://d.test/2") => err(Timeout) after 200ms
    given Net.get("https://d.test/3") => ok("three") after 200ms
    given Net.get("https://d.test/4") => ok("four") after 200ms
    given Net.get("https://d.test/5") => err(Refused) after 200ms
    fetch_all_concurrently(["https://d.test/1", "https://d.test/2", "https://d.test/3", "https://d.test/4", "https://d.test/5"]) == [ok("one"), err(Timeout), ok("three"), ok("four"), err(Refused)]
    then Net.get calls in any order ["https://d.test/1", "https://d.test/2", "https://d.test/3", "https://d.test/4", "https://d.test/5"]
    then elapsed under 500ms

    given Net.get("https://d.test/a") => ok("a") after 200ms
    given Net.get("https://d.test/b") => ok("b") after 50ms
    given Net.get("https://d.test/c") => ok("c") after 150ms
    fetch_all_concurrently(["https://d.test/a", "https://d.test/b", "https://d.test/c"]) == [ok("a"), ok("b"), ok("c")]
    then Net.get calls in any order ["https://d.test/a", "https://d.test/b", "https://d.test/c"]
    then elapsed under 450ms

    given Net.get("https://d.test/a") => ok("a") after 200ms
    fetch_all_concurrently(["https://d.test/a", "https://d.test/missing", "https://d.test/a", "https://d.test/a"]) == [ok("a"), err(NotFound), ok("a"), ok("a")]
    then Net.get calls in any order ["https://d.test/a", "https://d.test/missing", "https://d.test/a", "https://d.test/a"]
    then elapsed under 500ms

    given Net.get("https://d.test/p0") => ok("page 0") after 200ms
    given Net.get("https://d.test/p1") => ok("page 1") after 200ms
    given Net.get("https://d.test/p2") => ok("page 2") after 200ms
    given Net.get("https://d.test/p3") => ok("page 3") after 200ms
    given Net.get("https://d.test/p4") => ok("page 4") after 200ms
    given Net.get("https://d.test/p5") => ok("page 5") after 200ms
    given Net.get("https://d.test/p6") => ok("page 6") after 200ms
    given Net.get("https://d.test/p7") => ok("page 7") after 200ms
    fetch_all_concurrently(["https://d.test/p0", "https://d.test/p1", "https://d.test/p2", "https://d.test/p3", "https://d.test/p4", "https://d.test/p5", "https://d.test/p6", "https://d.test/p7"]) == [ok("page 0"), ok("page 1"), ok("page 2"), ok("page 3"), ok("page 4"), ok("page 5"), ok("page 6"), ok("page 7")]
    then Net.get calls in any order ["https://d.test/p0", "https://d.test/p1", "https://d.test/p2", "https://d.test/p3", "https://d.test/p4", "https://d.test/p5", "https://d.test/p6", "https://d.test/p7"]
    then elapsed under 600ms
