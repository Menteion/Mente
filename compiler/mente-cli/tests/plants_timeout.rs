//! A plant for the sandbox the tests run in on a Linux runner (docs/design-review.md,
//! untrusted-test-isolation): a test that runs past its time, expected to fail. nextest stops a
//! test by a signal, which the runner's user cannot send to the sandbox's user, so this shows that
//! the limit still holds there. This file never merges: it rides a throwaway pull request.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;
use std::time::Duration;

/// The test sleeps ten minutes, with a child that would sleep as long. nextest's limit is 40
/// seconds: it must report the test as timed out then, and neither process may go on.
#[test]
#[allow(clippy::zombie_processes)]
fn plant_a_test_past_its_time_is_stopped() {
    let child = Command::new("sleep").arg("600").spawn().unwrap();
    println!("sleeping, with the child {}", child.id());
    std::thread::sleep(Duration::from_secs(600));
}
