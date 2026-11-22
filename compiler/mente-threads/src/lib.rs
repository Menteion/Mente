//! The one place the workspace starts threads or asks how many cores the machine has, so that
//! work on more than one thread at once is work the test runner knows of.
//!
//! nextest runs the tests side by side, one test on each core. A test that spreads its own work
//! over more threads besides puts more threads than cores on the machine, and the time the tests
//! next to it get then depends on what else runs: on a runner with four cores, their times and the
//! limits of `.config/nextest.toml` measure the schedule, not the test. So a test that runs more
//! than one item through [`threads_for`] or [`parallel`], or more than one thread through
//! [`concurrently`], must be in the group `all-cores` of `.config/nextest.toml`, whose tests run
//! alone, each taking every test thread; under nextest, any other test that does fails and says
//! so, and so does a process it starts, which inherits its environment. Whether a test does
//! depends on its items and threads, not on the machine's cores, so it fails or passes alike on
//! every machine.
//!
//! The other threads this crate starts cannot run work beside their caller: [`alone`] runs a
//! closure on a thread with a stack of the size asked and waits for it, and the readers
//! ([`read_to_end_beside`], [`lines_beside`], [`append_beside`]) take no closure, only a child
//! process's standard output or error ([`Pipe`], which no other type can implement), so the
//! only work they do is waiting for its bytes.
//!
//! Clippy's `disallowed-methods` (clippy.toml) bans `std::thread::available_parallelism`,
//! `std::thread::spawn` and the `spawn` methods of `std::thread::Builder` and `std::thread::Scope`,
//! and the workspace forbids that lint (Cargo.toml), so that no `allow`, `expect` or lint group can
//! lower it in any other crate; this crate alone denies it, and names each use with `#[expect]`.
//! clippy.toml also bans `std::env::set_var`, `std::env::remove_var` and
//! `std::process::Command::env_clear`, which could hide from a test, or from an `mn` it starts,
//! the variables the group check reads.
//!
//! The check runs in every build of `mn`, not only in tests: under any nextest run, its own or
//! another project's, `mn fuzz --jobs 2` or `mn test --mutate` over several programs panics with
//! the message below unless the test that started it is in a group named `all-cores`; outside
//! nextest it never fires. It counts threads, not processes: a test that starts several child
//! processes before waiting for any runs work at once without it, which no test does today.

use std::io::{BufRead, BufReader, Read};
use std::process::{ChildStderr, ChildStdout};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// The test group of `.config/nextest.toml` whose tests run alone, given every core.
pub const GROUP: &str = "all-cores";

/// A child process's standard output or error, the only things the readers read: a type whose
/// `read` did work would run that work beside the caller, so no other type may implement this.
pub trait Pipe: sealed::Sealed + Read + Send + 'static {}

impl Pipe for ChildStdout {}
impl Pipe for ChildStderr {}

mod sealed {
    pub trait Sealed {}
    impl Sealed for std::process::ChildStdout {}
    impl Sealed for std::process::ChildStderr {}
}

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
/// panic of `f` is raised again here, once every thread has stopped.
pub fn parallel<T: Send, U: Send>(items: Vec<T>, f: impl Fn(T) -> U + Sync) -> Vec<U> {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let count = items.len();
    let threads = threads_for(count);
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<T>>> = items.into_iter().map(|t| Mutex::new(Some(t))).collect();
    let mut done: Vec<(usize, U)> = Vec::with_capacity(count);
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
    assert_eq!(done.len(), count, "parallel lost an item");
    done.sort_by_key(|(k, _)| *k);
    done.into_iter().map(|(_, u)| u).collect()
}

/// `f` on a thread of its own, named `name` and with a stack of `stack` bytes if given, which the
/// caller waits for: what `f` returned or its panic, or why the thread could not start. Only one
/// of the two threads works at a time, so no test needs the group for it.
#[expect(
    clippy::disallowed_methods,
    reason = "the one place that starts a thread the caller waits for"
)]
pub fn alone<T: Send>(
    name: Option<&str>,
    stack: Option<usize>,
    f: impl FnOnce() -> T + Send,
) -> std::io::Result<std::thread::Result<T>> {
    let mut builder = std::thread::Builder::new();
    if let Some(name) = name {
        builder = builder.name(name.to_string());
    }
    if let Some(stack) = stack {
        builder = builder.stack_size(stack);
    }
    std::thread::scope(|s| builder.spawn_scoped(s, f).map(|handle| handle.join()))
}

/// Reads `pipe` to its end on a thread of its own, beside the caller, so that a child process
/// printing more than a pipe holds is not stopped in its write while the caller waits for it.
/// Joining the handle gives the bytes read; a read that fails ends them there.
#[expect(
    clippy::disallowed_methods,
    reason = "the one place that starts a reader of a pipe"
)]
pub fn read_to_end_beside(mut pipe: impl Pipe) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes).ok();
        bytes
    })
}

/// Reads `pipe` line by line on a thread of its own, beside the caller, sending each line as it
/// comes; the channel closes at the pipe's end, at a read that fails, or when the receiver is
/// dropped.
#[expect(
    clippy::disallowed_methods,
    reason = "the one place that starts a reader of a pipe"
)]
pub fn lines_beside(pipe: impl Pipe) -> Receiver<String> {
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    lines
}

/// Reads `pipe` on a thread of its own, beside the caller, appending what it reads to `tail`
/// (lossily, as UTF-8) until the pipe's end or a read that fails.
#[expect(
    clippy::disallowed_methods,
    reason = "the one place that starts a reader of a pipe"
)]
pub fn append_beside(mut pipe: impl Pipe, tail: Arc<Mutex<String>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = pipe.read(&mut buf) {
            if n == 0 {
                break;
            }
            if let Ok(mut t) = tail.lock() {
                t.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        }
    })
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
         in the group `{}`, and in ALL_CORES_TESTS of .github/scripts/policy/test-runs.test.mjs, so that it \
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
        assert_eq!(parallel(Vec::<u32>::new(), |x| x), Vec::<u32>::new());
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
            // A panic among several items comes back once every thread has stopped.
            let raised = std::panic::catch_unwind(|| {
                parallel(vec![1u32, 2, 3, 4], |x| {
                    assert!(x != 3, "planted");
                    x
                })
            });
            assert!(raised.is_err());
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

    #[test]
    fn test_a_thread_alone_has_its_name_and_stack_and_is_waited_for() {
        let mut written = 0;
        let named = alone(Some("worker"), Some(4 << 20), || {
            // A stack of 4 MiB holds an array of 3 MiB, which the default 2 MiB would not.
            let big = [1u8; 3 << 20];
            written = big.iter().map(|&b| b as usize).sum::<usize>();
            std::thread::current().name().map(str::to_string)
        })
        .unwrap()
        .unwrap();
        assert_eq!(named.as_deref(), Some("worker"));
        // The caller waited: what the thread wrote is there.
        assert_eq!(written, 3 << 20);
        assert!(alone(None, None, || panic!("planted")).unwrap().is_err());
        assert_eq!(alone(None, None, || 5).unwrap().unwrap(), 5);
    }

    /// This test binary run as a child: `--list` prints its tests to standard output, and an
    /// option it does not know prints an error to standard error.
    fn child(arg: &str) -> std::process::Child {
        std::process::Command::new(std::env::current_exe().unwrap())
            .arg(arg)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    }

    #[test]
    fn test_the_readers_give_what_the_pipe_held() {
        let name = "test_the_readers_give_what_the_pipe_held";
        let mut listed = child("--list");
        let bytes = read_to_end_beside(listed.stdout.take().unwrap());
        let bytes = String::from_utf8(bytes.join().unwrap()).unwrap();
        listed.wait().unwrap();
        assert!(bytes.contains(name), "{}", bytes);
        let mut listed = child("--list");
        let lines: Vec<String> = lines_beside(listed.stdout.take().unwrap()).iter().collect();
        listed.wait().unwrap();
        assert!(lines.iter().any(|l| l.contains(name)), "{:?}", lines);
        let mut refused = child("--no-such-option-here");
        let tail = Arc::new(Mutex::new(String::from(">")));
        append_beside(refused.stderr.take().unwrap(), Arc::clone(&tail))
            .join()
            .unwrap();
        refused.wait().unwrap();
        let tail = tail.lock().unwrap();
        assert!(
            tail.starts_with('>') && tail.contains("no-such-option-here"),
            "{}",
            tail
        );
    }
}

#[cfg(test)]
mod overflow_traps;
