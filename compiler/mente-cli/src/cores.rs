//! The one place the tools ask how many cores the machine has and run work on more than one
//! thread at once, so that such work is work the test runner knows of.
//!
//! nextest runs the tests side by side, one test on each core. A test that spreads its own work
//! over more threads besides puts more threads than cores on the machine, and the time the tests
//! next to it get then depends on what else runs: on a runner with four cores, their times and the
//! limits of `.config/nextest.toml` measure the schedule, not the test. So a test that runs more
//! than one item through [`threads_for`], or more than one thread through [`concurrently`], must
//! be in the group `all-cores` of `.config/nextest.toml`, whose tests run alone, each taking every
//! test thread; under nextest, any other test that does fails and says so, and so does a process
//! it starts, which inherits its environment. Whether a test does depends on its items and
//! threads, not on the machine's cores, so it fails or passes alike on every machine.
//!
//! Clippy's `disallowed-methods` (clippy.toml) keeps `std::thread::available_parallelism` and
//! `std::thread::Scope::spawn` to this module, and `tests/lints.rs` keeps the names of the core
//! count and of the lint that bans it out of every other file, so that neither a `cfg` clippy does
//! not build nor an `allow` reopens the way. Outside the guard stay threads started one at a time
//! for a larger stack or to drain a child's pipe (`std::thread::Builder`, `std::thread::spawn`).

/// The test group of `.config/nextest.toml` whose tests run alone, given every core.
pub const GROUP: &str = "all-cores";

/// How many threads `items` pieces of work spread over every core run on: one for each core the
/// machine has (1 if it does not say), no more than the items, and at least one. More than one
/// item fails a test that nextest runs beside others.
#[expect(
    clippy::disallowed_methods,
    reason = "the one place that reads the machine's core count"
)]
pub fn threads_for(items: usize) -> usize {
    if items > 1 {
        held_to_the_group();
    }
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    cores.min(items).max(1)
}

/// `f(0)` to `f(threads - 1)`, each on a thread of its own, all at once; what each returned, or
/// its panic, in that order. More than one thread fails a test that nextest runs beside others.
#[expect(
    clippy::disallowed_methods,
    reason = "the one place that starts threads which run at once"
)]
pub fn concurrently<U: Send>(
    threads: usize,
    f: impl Fn(usize) -> U + Sync,
) -> Vec<std::thread::Result<U>> {
    if threads > 1 {
        held_to_the_group();
    }
    let f = &f;
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads).map(|k| s.spawn(move || f(k))).collect();
        handles.into_iter().map(|h| h.join()).collect()
    })
}

/// `f` on every item, spread over [`threads_for`] threads, the results in the items' order. A
/// panic of `f` is raised again here, once every thread has stopped. Only the tests' comparisons
/// of the three paths use it, so it is built with them.
#[cfg(test)]
pub fn parallel<T: Send, U: Send>(items: Vec<T>, f: impl Fn(T) -> U + Sync) -> Vec<U> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    let threads = threads_for(items.len());
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<T>>> = items.into_iter().map(|t| Mutex::new(Some(t))).collect();
    let mut done: Vec<(usize, U)> = Vec::with_capacity(slots.len());
    for joined in concurrently(threads, |_| {
        let mut mine = Vec::new();
        loop {
            let k = next.fetch_add(1, Ordering::Relaxed);
            let Some(slot) = slots.get(k) else {
                return mine;
            };
            // Each slot is taken once, by the thread that drew its index; nothing panics while
            // its lock is held, so the lock is never poisoned.
            if let Some(item) = slot.lock().ok().and_then(|mut held| held.take()) {
                mine.push((k, f(item)));
            }
        }
    }) {
        match joined {
            Ok(mine) => done.extend(mine),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
    done.sort_by_key(|(k, _)| *k);
    done.into_iter().map(|(_, u)| u).collect()
}

/// Fails the test, under nextest, unless it is a test of [`GROUP`].
fn held_to_the_group() {
    let nextest = std::env::var_os("NEXTEST");
    let group = std::env::var("NEXTEST_TEST_GROUP").ok();
    if let Err(problem) = may_use_every_core(nextest.is_some(), group.as_deref()) {
        panic!("{}", problem);
    }
}

/// Whether a process may run work on more than one thread at once: always outside nextest; under
/// it, only in a test of [`GROUP`], or a process such a test started.
fn may_use_every_core(under_nextest: bool, group: Option<&str>) -> Result<(), String> {
    if !under_nextest || group == Some(GROUP) {
        return Ok(());
    }
    Err(format!(
        "this test runs work on more than one thread while nextest runs other tests beside it \
         (its test group is {}): name it in the override of .config/nextest.toml that puts tests \
         in the group `{}`, and in ALL_CORES_TESTS of .github/scripts/policy.test.mjs, so that it \
         runs alone. An ignored test runs through `cargo test`, outside nextest, and a nextest \
         older than 0.9.90 names no group",
        group.map_or("unknown".to_string(), |g| format!("`{}`", g)),
        GROUP
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_only_a_test_of_the_group_may_use_every_core_under_nextest() {
        assert_eq!(may_use_every_core(false, None), Ok(()));
        assert_eq!(may_use_every_core(false, Some("@global")), Ok(()));
        assert_eq!(may_use_every_core(true, Some(GROUP)), Ok(()));
        for group in [None, Some("@global"), Some("all-cores-2"), Some("")] {
            let problem = may_use_every_core(true, group).unwrap_err();
            assert!(problem.contains(".config/nextest.toml"), "{}", problem);
            assert!(problem.contains("ALL_CORES_TESTS"), "{}", problem);
        }
    }

    #[test]
    fn test_one_item_or_one_thread_runs_in_any_test() {
        // None of these asks whether the test may use every core, so none fails under nextest.
        assert_eq!(threads_for(0), 1);
        assert_eq!(threads_for(1), 1);
        let ran: Vec<usize> = concurrently(1, |k| k + 7)
            .into_iter()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(ran, vec![7]);
        assert!(concurrently(0, |k| k).is_empty());
        assert_eq!(parallel(vec![5], |x: u32| x * 3), vec![15]);
    }

    #[test]
    fn test_more_than_one_item_or_thread_fails_a_test_outside_the_group_under_nextest() {
        // This test is in no group. Under nextest (the required jobs and the mutation runs), each
        // call must fail; under `cargo test` each may run, and returns what it ran.
        let under_nextest = std::env::var_os("NEXTEST").is_some();
        let spread = std::panic::catch_unwind(|| threads_for(2));
        let at_once = std::panic::catch_unwind(|| concurrently(2, |k| k * 10));
        let mapped = std::panic::catch_unwind(|| parallel(vec![1, 2, 3], |x: u32| x + 1));
        if under_nextest {
            assert!(spread.is_err() && at_once.is_err() && mapped.is_err());
        } else {
            assert!(matches!(spread, Ok(1..=2)), "{:?}", spread);
            let at_once: Vec<usize> = at_once.unwrap().into_iter().map(|r| r.unwrap()).collect();
            assert_eq!(at_once, vec![0, 10]);
            assert_eq!(mapped.unwrap(), vec![2, 3, 4]);
        }
    }

    #[test]
    fn test_a_panic_of_the_work_is_raised_once_every_thread_has_stopped() {
        // One item, so that the test needs no group: the panic comes back through the join.
        let raised = std::panic::catch_unwind(|| parallel(vec![0u32], |_| panic!("planted")));
        assert!(raised.is_err());
        let joined = concurrently(1, |_| -> u32 { panic!("planted") });
        assert!(joined[0].is_err());
    }
}
