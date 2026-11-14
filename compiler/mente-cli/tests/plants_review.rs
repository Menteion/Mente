//! Plants for the review of pull request 137 (docs/design-review.md, untrusted-test-isolation):
//! what a test can do in the sandbox as it stood at 7778d2e. The report fails on purpose, so that
//! nextest prints it. This file never merges: it rides a throwaway pull request.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::process::Command;

fn sh(script: &str) -> String {
    match Command::new("sh").args(["-c", script]).output() {
        Ok(out) => format!(
            "{} | {} | {}",
            out.status,
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .replace('\n', " / "),
            String::from_utf8_lossy(&out.stderr)
                .trim()
                .replace('\n', " / ")
        ),
        Err(e) => format!("not run: {e}"),
    }
}

#[test]
fn plant_report_of_the_review() {
    let mut report = Vec::new();
    // M2: target/tmp is a tracked link into /usr/lib/locale.
    report.push(format!(
        "M2 CARGO_TARGET_TMPDIR: {}",
        env!("CARGO_TARGET_TMPDIR")
    ));
    report.push(format!(
        "M2 write /usr/lib/locale/plant-written-by-a-test: {:?}",
        fs::write("/usr/lib/locale/plant-written-by-a-test", "plant")
    ));
    report.push(format!(
        "M2 ls -la /usr/lib/locale: {}",
        sh("ls -la /usr/lib/locale")
    ));
    report.push(format!(
        "M2 mounts over /usr: {}",
        sh("grep -E ' /usr| /tmp ' /proc/self/mountinfo | cut -d' ' -f5-9")
    ));
    // M1: a crash at the core limit the sandbox sets, and at a soft limit lowered to 0.
    report.push(format!(
        "M1 core_pattern: {}",
        fs::read_to_string("/proc/sys/kernel/core_pattern")
            .unwrap_or_default()
            .trim()
    ));
    report.push(format!("M1 limits: {}", sh("ulimit -S -c; ulimit -H -c")));
    report.push(format!(
        "M1 crash at the limit as set: {}",
        sh("cp /usr/bin/sleep /tmp/plant-core-one && { /tmp/plant-core-one 30 & p=$!; sleep 0.3; kill -SEGV $p; wait $p; echo \"status $?\"; }")
    ));
    report.push(format!(
        "M1 crash at a soft limit of 0: {}",
        sh("ulimit -S -c 0 && ulimit -S -c && cp /usr/bin/sleep /tmp/plant-core-zero && { /tmp/plant-core-zero 30 & p=$!; sleep 0.3; kill -SEGV $p; wait $p; echo \"status $?\"; }")
    ));
    // A user namespace from inside.
    for file in [
        "/proc/sys/kernel/apparmor_restrict_unprivileged_userns",
        "/proc/sys/kernel/unprivileged_userns_clone",
        "/proc/sys/user/max_user_namespaces",
    ] {
        report.push(format!(
            "userns {file}: {}",
            fs::read_to_string(file)
                .unwrap_or_else(|e| e.to_string())
                .trim()
        ));
    }
    report.push(format!(
        "userns unshare --user --map-root-user id -u: {}",
        sh("unshare --user --map-root-user id -u")
    ));
    report.push(format!(
        "userns a mount in it: {}",
        sh("unshare --user --map-root-user --mount sh -c 'mount -t tmpfs none /tmp && echo mounted'")
    ));
    report.push(format!(
        "userns the workspace remounted to write in it: {}",
        sh("unshare --user --map-root-user --mount sh -c 'mount -o remount,bind,rw \"$PWD\" && echo remounted'")
    ));
    // Devices and IPC in sight.
    report.push(format!("devices: {}", sh("ls /dev")));
    report.push(format!(
        "ipc: {}",
        sh("ipcs -m | tail -n +3; ls /dev/shm /dev/mqueue 2>&1")
    ));
    panic!("the report of the review's plants:\n{}", report.join("\n"));
}
