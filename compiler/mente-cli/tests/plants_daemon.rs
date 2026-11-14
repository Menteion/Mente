//! Two more plants for the sandbox the tests run in on a Linux runner (docs/design-review.md,
//! untrusted-test-isolation), both expected to fail: the daemon a review of pull request 131
//! described, which the sandbox fails, and a report of what stopped each route, which fails on
//! purpose so that nextest prints it. This file never merges: it rides a throwaway pull request.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// The daemon would swap a tool between two checks and put it back. It is left behind in a
/// session of its own and holds none of the test's output, so nextest's leak-timeout alone would
/// not see it. The sandbox ends it with the test, and fails the test for leaving it.
#[test]
#[allow(clippy::zombie_processes)]
fn plant_a_daemon_left_behind_fails_its_test_and_ends_with_it() {
    let daemon = Command::new("setsid")
        .args(["sh", "-c", "sleep 600"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    println!("left behind: process {}", daemon.id());
}

fn write(path: &Path) -> String {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let result = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(b"plant\n"));
    match result {
        Ok(()) => format!("write {}: WRITTEN", path.display()),
        Err(e) => format!("write {}: {e}", path.display()),
    }
}

fn run(program: &str, args: &[&str]) -> String {
    match Command::new(program).args(args).output() {
        Ok(out) => format!(
            "{program} {}: {}, {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => format!("{program} {}: {e}", args.join(" ")),
    }
}

/// What stopped each route, printed by failing.
#[test]
fn plant_report_what_stopped_each_route() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut report = Vec::new();
    for path in [
        "/home/runner/.local/bin/cargo",
        "/home/runner/.cargo/bin/cargo-nextest",
        "/home/runner/.cargo/config.toml",
        "/usr/local/bin/cargo",
        "/home/runner/work/_temp/tools.sha256",
    ] {
        report.push(write(Path::new(path)));
    }
    for path in [
        "rust-toolchain.toml",
        ".cargo/config.toml",
        ".git/refs/remotes/origin/main",
        ".config/nextest.toml",
        ".github/scripts/tools.sh",
        "target/debug/deps/plant",
        "mutants.out/caught.txt",
    ] {
        report.push(write(&root.join(path)));
    }
    report.push(write(&std::env::current_exe().unwrap()));
    report.push(write(Path::new(env!("CARGO_BIN_EXE_mn"))));
    for name in ["GITHUB_ENV", "GITHUB_PATH", "RUNNER_TEMP"] {
        report.push(format!("variable {name}: {:?}", std::env::var_os(name)));
    }
    report.push(run("sudo", &["-n", "true"]));
    report.push(run("kill", &["-TERM", "1"]));
    report.push(format!(
        "read /proc/1/environ: {:?}",
        fs::read("/proc/1/environ").map(|bytes| bytes.len())
    ));
    let address = "140.82.112.3:443"
        .to_socket_addrs()
        .unwrap()
        .next()
        .unwrap();
    report.push(format!(
        "connect {address}: {:?}",
        TcpStream::connect_timeout(&address, Duration::from_secs(3)).map(|_| "CONNECTED")
    ));
    let mut seen: Vec<String> = fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.chars().all(|c| c.is_ascii_digit()))
        .collect();
    seen.sort();
    report.push(format!("processes in sight: {}", seen.join(" ")));
    let status = fs::read_to_string("/proc/self/status").unwrap();
    for line in status.lines() {
        if ["Uid:", "Groups:", "NoNewPrivs:", "CapEff:", "CapBnd:"]
            .iter()
            .any(|field| line.starts_with(field))
        {
            report.push(line.replace('\t', " "));
        }
    }
    panic!("the report of the plants:\n{}", report.join("\n"));
}
