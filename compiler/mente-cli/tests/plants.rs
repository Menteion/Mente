//! Plants for the sandbox the tests run in on a Linux runner (docs/design-review.md,
//! untrusted-test-isolation): each test tries one route by which the reviews of pull requests 127
//! and 131 had test code fake a later step of its job, and passes only when the route is closed.
//! This file never merges: it rides a throwaway pull request, so that the real steps of the
//! required Rust jobs and of the mutation run execute it.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// What stopped a write, or `None` when it went through.
fn write_refused(path: &Path, text: &str) -> Option<String> {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .err()
        .map(|e| e.to_string())
}

fn assert_refused(route: &str, path: &Path, text: &str) {
    let refused = write_refused(path, text);
    println!("{route}: {} -> {:?}", path.display(), refused);
    assert!(
        refused.is_some(),
        "{route}: the test wrote {}",
        path.display()
    );
}

/// The runner's home, where cargo and rustup keep the tools: the test's own HOME is not it.
fn runner_home() -> PathBuf {
    PathBuf::from("/home/runner")
}

#[test]
fn plant_a_stub_earlier_on_the_path_is_refused() {
    let stub = "#!/bin/sh\necho '        FAIL [   0.004s] mente-syntax tests::x'\nexit 100\n";
    for dir in [".local/bin", ".cargo/bin"] {
        for tool in ["cargo", "cargo-nextest", "rustc", "cargo-mutants"] {
            assert_refused(
                "stub on the path",
                &runner_home().join(dir).join(tool),
                stub,
            );
        }
    }
    // Where CARGO_HOME says cargo's own directory is, if the variable reached the test.
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        assert_refused(
            "stub in CARGO_HOME",
            &Path::new(&home).join("bin/cargo-nextest"),
            stub,
        );
    }
    for dir in ["/usr/local/bin", "/usr/bin", "/usr/local/sbin"] {
        assert_refused(
            "stub in a system directory",
            &Path::new(dir).join("cargo"),
            stub,
        );
    }
}

#[test]
fn plant_a_link_in_cargo_bin_is_refused() {
    let link = runner_home().join(".cargo/bin/cargo-nextest-plant");
    let made = std::os::unix::fs::symlink("/usr/bin/true", &link);
    println!("link: {made:?}");
    assert!(made.is_err(), "the test made {}", link.display());
}

#[test]
fn plant_a_path_toolchain_is_refused() {
    assert_refused(
        "path = toolchain",
        &workspace().join("rust-toolchain.toml"),
        "path = \"/tmp/toolchain\"\n",
    );
    assert_refused(
        "legacy toolchain file",
        &workspace().join("rust-toolchain"),
        "nightly\n",
    );
    for home in [runner_home().join(".rustup"), PathBuf::from("/tmp/rustup")] {
        let toolchains = home.join("toolchains");
        if toolchains.is_dir() {
            for entry in fs::read_dir(&toolchains).unwrap().flatten() {
                assert_refused(
                    "toolchain's own rustc",
                    &entry.path().join("bin/rustc"),
                    "stub",
                );
            }
        }
    }
    assert!(
        !runner_home().join(".rustup").exists(),
        "the toolchains are in the test's sight"
    );
}

#[test]
fn plant_a_runner_in_cargo_config_is_refused() {
    let config = "[target.x86_64-unknown-linux-gnu]\nrunner = \"true\"\n";
    assert_refused(
        "runner in CARGO_HOME",
        &runner_home().join(".cargo/config.toml"),
        config,
    );
    assert_refused(
        "runner in the workspace",
        &workspace().join(".cargo/config.toml"),
        config,
    );
    assert_refused(
        "runner above the workspace",
        &workspace().join("../.cargo/config.toml"),
        config,
    );
}

#[test]
fn plant_a_replaced_file_under_target_is_refused() {
    let exe = std::env::current_exe().unwrap();
    let refused = fs::write(&exe, "stub");
    println!("own executable {}: {refused:?}", exe.display());
    assert!(refused.is_err(), "the test replaced its own executable");
    let mn = Path::new(env!("CARGO_BIN_EXE_mn"));
    let refused = fs::write(mn, "stub");
    println!("mn {}: {refused:?}", mn.display());
    assert!(refused.is_err(), "the test replaced mn");
    let removed = fs::remove_file(mn);
    assert!(removed.is_err(), "the test removed mn");
    let renamed = fs::rename(mn, mn.with_extension("moved"));
    assert!(renamed.is_err(), "the test moved mn");
    assert_refused(
        "new file under target",
        &workspace().join("target/debug/deps/plant"),
        "stub",
    );
}

#[test]
fn plant_origin_main_moved_or_the_settings_edited_is_refused() {
    let root = workspace();
    let zero = "0000000000000000000000000000000000000000\n";
    assert_refused(
        "origin/main moved",
        &root.join(".git/refs/remotes/origin/main"),
        zero,
    );
    assert_refused("packed refs", &root.join(".git/packed-refs"), zero);
    assert_refused("git config", &root.join(".git/config"), "[core]\n");
    assert_refused(
        "nextest's settings",
        &root.join(".config/nextest.toml"),
        "[profile.ci]\nretries = 9\n",
    );
    assert_refused(
        "the mutants' settings",
        &root.join(".cargo/mutants.toml"),
        "exclude_globs = [\"**\"]\n",
    );
    assert_refused(
        "a gate script",
        &root.join(".github/scripts/tools.sh"),
        "exit 0\n",
    );
    assert_refused(
        "the workflow",
        &root.join(".github/workflows/ci.yml"),
        "# plant\n",
    );
    // The trusted inputs and the outcomes of the mutation run, in the workspace's root.
    for file in [
        "pr.diff",
        "shard-mutants.json",
        "deleted-re.txt",
        "base-mutants.toml",
        "mutants.out/caught.txt",
        "mutants.out/outcomes.json",
        "mutants.out/log/plant.log",
        "baseline.log",
    ] {
        assert_refused("the mutation run's files", &root.join(file), "plant\n");
    }
}

#[test]
fn plant_the_environment_of_later_steps_is_out_of_reach() {
    for name in [
        "GITHUB_ENV",
        "GITHUB_PATH",
        "GITHUB_OUTPUT",
        "GITHUB_STEP_SUMMARY",
        "RUNNER_TEMP",
    ] {
        let value = std::env::var_os(name);
        println!("{name}: {value:?}");
        assert!(value.is_none(), "{name} reached the test");
    }
    let commands = Path::new("/home/runner/work/_temp/_runner_file_commands");
    assert!(
        !commands.exists(),
        "the runner's file commands are in the test's sight"
    );
    assert_refused(
        "the runner's temporary directory",
        &Path::new("/home/runner/work/_temp").join("tools.sha256"),
        "plant\n",
    );
}

#[test]
fn plant_sudo_is_refused() {
    let out = Command::new("sudo").args(["-n", "true"]).output();
    println!("sudo: {out:?}");
    assert!(
        !matches!(&out, Ok(o) if o.status.success()),
        "sudo ran for the test"
    );
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let field = |name: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(name))
            .map(|v| v.trim().to_string())
            .unwrap()
    };
    println!(
        "uid {} NoNewPrivs {} CapEff {} CapBnd {}",
        field("Uid:"),
        field("NoNewPrivs:"),
        field("CapEff:"),
        field("CapBnd:")
    );
    assert_eq!(field("NoNewPrivs:"), "1");
    assert_eq!(u64::from_str_radix(&field("CapEff:"), 16).unwrap(), 0);
    assert_eq!(u64::from_str_radix(&field("CapBnd:"), 16).unwrap(), 0);
    assert!(
        !field("Uid:")
            .split_whitespace()
            .any(|u| u == "0" || u == "1001"),
        "the test runs as root or as the runner's user"
    );
}

#[test]
fn plant_the_tools_cannot_be_signalled_or_traced() {
    // Every process the test can see: those of its own namespace, a handful, numbered from 1.
    // cargo, nextest, cargo-mutants and the runner are not among them.
    let mut seen = Vec::new();
    for entry in fs::read_dir("/proc").unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Ok(pid) = name.parse::<u32>() {
            seen.push(pid);
        }
    }
    println!("processes in sight: {seen:?}");
    assert!(seen.contains(&1), "the test sees no first process");
    assert!(
        seen.len() <= 8 && seen.iter().all(|pid| *pid < 100),
        "the test sees processes beyond its own namespace: {seen:?}"
    );
    let first = fs::read_to_string("/proc/1/cmdline").unwrap_or_default();
    assert!(
        first.contains("test-sandbox.sh"),
        "the first process in sight is not the sandbox's: {first}"
    );
    // Its parent is the namespace's first process, root's: no signal, no trace.
    let killed = Command::new("kill").args(["-TERM", "1"]).status().unwrap();
    assert!(!killed.success(), "the test signalled the first process");
    assert!(
        fs::read("/proc/1/environ").is_err(),
        "the test read the first process's environment"
    );
    assert!(
        fs::read("/proc/1/mem").is_err(),
        "the test opened the first process's memory"
    );
}

#[test]
fn plant_the_network_is_out_of_reach() {
    let addresses = [
        "140.82.112.3:443",
        "1.1.1.1:443",
        "127.0.0.1:22",
        "169.254.169.254:80",
    ];
    for address in addresses {
        let socket = address.to_socket_addrs().unwrap().next().unwrap();
        let reached = TcpStream::connect_timeout(&socket, Duration::from_secs(3));
        println!("{address}: {reached:?}");
        assert!(reached.is_err(), "the test reached {address}");
    }
    let udp = UdpSocket::bind("0.0.0.0:0").and_then(|s| s.send_to(b"plant", "1.1.1.1:53"));
    println!("udp: {udp:?}");
    assert!(udp.is_err(), "the test sent a datagram");
    let resolved = "github.com:443".to_socket_addrs();
    assert!(resolved.is_err(), "the test resolved a name");
    let interfaces = fs::read_to_string("/proc/net/dev").unwrap();
    println!("{interfaces}");
    assert!(
        interfaces
            .lines()
            .skip(2)
            .all(|l| l.trim_start().starts_with("lo:")),
        "the test has a network interface"
    );
}

#[test]
fn plant_what_the_tests_need_is_there() {
    // The workspace to read, mn to run, and a directory of its own to write.
    assert!(workspace().join("Cargo.toml").is_file());
    let out = Command::new(env!("CARGO_BIN_EXE_mn"))
        .arg("--version")
        .output();
    println!("mn: {out:?}");
    assert!(out.is_ok(), "the test cannot run mn");
    let own = std::env::temp_dir().join("plant-own-file");
    fs::write(&own, "kept").unwrap();
    assert_eq!(fs::read_to_string(&own).unwrap(), "kept");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("plant-own-file");
    fs::write(&target, "kept").unwrap();
}
