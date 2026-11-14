//! Plants for the review of pull request 137 (docs/design-review.md, untrusted-test-isolation):
//! what a test can do in the sandbox after the fix. The report fails on purpose, so that nextest
//! prints it. This file never merges: it rides a throwaway pull request.

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
fn plant_report_of_the_review_after_the_fix() {
    let mut report = Vec::new();
    report.push(format!(
        "M2 write /usr/lib/locale/plant-written-by-a-test: {:?}",
        fs::write("/usr/lib/locale/plant-written-by-a-test", "plant")
    ));
    report.push(format!(
        "M2 mounts in the sandbox: {}",
        sh("cut -d' ' -f5,6,9 /proc/self/mountinfo | tr '\\n' ';'")
    ));
    report.push(format!(
        "M1 core_pattern: {}",
        fs::read_to_string("/proc/sys/kernel/core_pattern")
            .unwrap_or_default()
            .trim()
    ));
    report.push(format!("M1 limits: {}", sh("ulimit -S -c; ulimit -H -c")));
    report.push(format!(
        "M1 crash at the highest limit it can set: {}",
        sh("cd /tmp && ulimit -S -c \"$(ulimit -H -c)\" && cp /usr/bin/sleep /tmp/plant-core-one && { /tmp/plant-core-one 30 & p=$!; sleep 0.3; kill -SEGV $p; wait $p; echo \"status $?\"; ls /tmp | grep -c core; }")
    ));
    report.push(format!(
        "M1 crash at a soft limit of 0: {}",
        sh("cd /tmp && ulimit -S -c 0 && cp /usr/bin/sleep /tmp/plant-core-zero && { /tmp/plant-core-zero 30 & p=$!; sleep 0.3; kill -SEGV $p; wait $p; echo \"status $?\"; ls /tmp | grep -c core; }")
    ));
    report.push(format!(
        "userns unshare --user --map-root-user id -u: {}",
        sh("unshare --user --map-root-user id -u")
    ));
    report.push(format!("devices: {}", sh("ls /dev")));
    report.push(format!(
        "ipc: {}",
        sh("ipcs -m | tail -n +4; ls /dev/shm /dev/mqueue 2>&1")
    ));
    report.push(format!(
        ".git/HEAD: {:?}",
        fs::read_to_string(".git/HEAD").map(|t| t.len())
    ));
    panic!(
        "the report of the review's plants after the fix:\n{}",
        report.join("\n")
    );
}
