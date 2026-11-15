//! The one place the tools ask how many cores the machine has, so that work spread over every core
//! is work the test runner knows of.
//!
//! nextest runs the tests side by side, one test on each core. A test that spreads its own work
//! over every core besides puts twice as many threads as cores on the machine, and the time the
//! tests next to it get then depends on what else runs: on a runner with four cores, their times
//! and the limits of `.config/nextest.toml` measure the schedule, not the test. So a test that
//! spreads more than one item over [`threads_for`] must be in the group `all-cores` of
//! `.config/nextest.toml`, whose tests run alone, each taking every test thread; under nextest,
//! any other test that does fails and says so. Whether it does depends on the number of items,
//! not on the machine's cores, so a test fails or passes alike on every machine. Clippy's `disallowed-methods` (clippy.toml) keeps
//! `std::thread::available_parallelism` to this module.

/// The test group of `.config/nextest.toml` whose tests run alone, given every core.
pub const GROUP: &str = "all-cores";

/// How many threads `items` pieces of work spread over every core run on: one for each core the
/// machine has (1 if it does not say), no more than the items, and at least one. More than one
/// item fails a test that nextest runs beside others.
#[allow(clippy::disallowed_methods)]
pub fn threads_for(items: usize) -> usize {
    if items > 1 {
        let nextest = std::env::var_os("NEXTEST");
        let group = std::env::var("NEXTEST_TEST_GROUP").ok();
        if let Err(problem) = may_use_every_core(nextest.is_some(), group.as_deref()) {
            panic!("{}", problem);
        }
    }
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    cores.min(items).max(1)
}

/// Whether a process may spread its work over every core: always outside nextest; under it, only
/// in a test of [`GROUP`], or a process such a test started, which inherits its environment.
fn may_use_every_core(under_nextest: bool, group: Option<&str>) -> Result<(), String> {
    if !under_nextest || group == Some(GROUP) {
        return Ok(());
    }
    Err(format!(
        "this test spreads its work over every core while nextest runs other tests beside it \
         (its test group is {}): name it in the override of .config/nextest.toml that puts tests \
         in the group `{}`, and in ALL_CORES_TESTS of .github/scripts/policy.test.mjs, so that it \
         runs alone",
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
    fn test_one_item_or_none_runs_on_one_thread_in_any_test() {
        // Neither asks whether the test may use every core, so neither fails under nextest.
        assert_eq!(threads_for(0), 1);
        assert_eq!(threads_for(1), 1);
    }
}
