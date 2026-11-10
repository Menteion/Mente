// A canary for the benchmark sandbox (benchmark/harness/sandbox-check.ts). Run as an agent program
// would be, it tries to read a secret from the driver's process, from a planted credential file and
// from its own environment; to see a network interface beyond loopback; to read a file of the
// workspace, a file standing in for the held-out set and a file of another run's folder, as a
// held-out run's record beside a public run's build; and to write outside its own directory.
// It also writes into its own directory, which must keep working. It prints one line of flags,
// never what it read; the check reads the flags. Under the sandbox every flag but `work` must be
// false; without it, as a control, they are true, so a green result is not vacuous.
//
//   leak <credential file> <workspace file> <held-out file> <directory to write into> <another run's file>
use std::fs;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    let (credential, workspace, held_out, outside, other_run) = (arg(1), arg(2), arg(3), arg(4), arg(5));
    let needle = std::env::var("BENCH_CANARY_NEEDLE").unwrap_or_default();

    // A marker in this program's own environment, which the sandbox clears.
    let env_secret = std::env::var("BENCH_CANARY_ENV").is_ok();

    // The driver's environment, reachable through /proc when its process is in the table. The
    // program's own process is skipped, since the needle is in its own environment; the point is
    // to reach another process, as the driver is.
    let own = std::process::id().to_string();
    let mut proc_secret = false;
    if !needle.is_empty() {
        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy() == *own {
                    continue;
                }
                if let Ok(data) = fs::read(entry.path().join("environ")) {
                    if data.windows(needle.len()).any(|w| w == needle.as_bytes()) {
                        proc_secret = true;
                        break;
                    }
                }
            }
        }
    }

    // Files outside the working directory: a credential, the workspace, the held-out set, and
    // another run's folder.
    let readable = |p: &str| !p.is_empty() && fs::read(p).is_ok();
    let file_secret = readable(&credential);
    let workspace_file = readable(&workspace);
    let held_out_file = readable(&held_out);
    let other_run_file = readable(&other_run);

    // Network interfaces beyond loopback, from the program's own network namespace.
    let mut has_network = false;
    if let Ok(dev) = fs::read_to_string("/proc/net/dev") {
        for line in dev.lines() {
            if let Some((name, _)) = line.split_once(':') {
                let name = name.trim();
                if !name.is_empty() && name != "lo" {
                    has_network = true;
                }
            }
        }
    }

    // A write outside the working directory, which a read-only root refuses, and one inside it.
    let probe = Path::new(&outside).join("canary-write");
    let wrote_outside = !outside.is_empty() && fs::write(&probe, b"x").is_ok();
    if wrote_outside {
        let _ = fs::remove_file(&probe);
    }
    let wrote_inside = fs::write("canary-work", b"x").is_ok();
    let _ = fs::remove_file("canary-work");

    println!(
        "env={env_secret} proc={proc_secret} file={file_secret} net={has_network} ws={workspace_file} held={held_out_file} run={other_run_file} write={wrote_outside} work={wrote_inside}"
    );
}
