// A canary for the benchmark sandbox (benchmark/harness/sandbox-check.ts). Run as an agent program
// would be, it tries to read a secret from the driver's process, from a planted credential file and
// from its own environment; to see a network interface beyond loopback; to read a file of the
// workspace, a file standing in for the held-out set and a file of another run's folder, as a
// held-out run's record beside a public run's build; to connect to the broker's socket, which
// would let it spend the API key; to reach any of these through a descriptor it did not open, which
// whatever started it left open, by listing, reading and connecting through /proc/self/fd and
// /proc/self/fdinfo; and to write outside its own directory.
// It also writes into its own directory, which must keep working. It prints one line of flags,
// never what it read; the check reads the flags. Under the sandbox every flag but `work` must be
// false; without it, as a control, they are true, so a green result is not vacuous.
//
//   leak <credential file> <workspace file> <held-out file> <directory to write into> <another run's file> <the broker's socket>
use std::fs;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    let (credential, workspace, held_out, outside, other_run, broker) = (arg(1), arg(2), arg(3), arg(4), arg(5), arg(6));
    let needle = std::env::var("BENCH_CANARY_NEEDLE").unwrap_or_default();

    // Descriptors it did not open, read first, before it opens anything: the sandbox starts it with
    // 0, 1 and 2 alone, and any other names something of the runner's outside the sandbox's view.
    // Through each it tries what the final pass of 05fe2f6 did: the held-out stand-in under a
    // descriptor of the workspace, the credential under one of the masked directory, and the
    // broker's socket under one of its directory; and it lists and reads each.
    let inherited = inherited_descriptors();
    let planted = [&credential, &workspace, &held_out, &other_run, &broker];
    let through_any = inherited.iter().any(|n| through(n, &planted));
    let fd_reached = !inherited.is_empty() || through_any;

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

    // The broker's socket, which holds the key's spending: a path the sandbox covers.
    let broker_reached = !broker.is_empty() && reaches(&broker);

    // A write outside the working directory, which a read-only root refuses, and one inside it.
    let probe = Path::new(&outside).join("canary-write");
    let wrote_outside = !outside.is_empty() && fs::write(&probe, b"x").is_ok();
    if wrote_outside {
        let _ = fs::remove_file(&probe);
    }
    let wrote_inside = fs::write("canary-work", b"x").is_ok();
    let _ = fs::remove_file("canary-work");

    println!(
        "env={env_secret} proc={proc_secret} file={file_secret} net={has_network} ws={workspace_file} held={held_out_file} run={other_run_file} broker={broker_reached} fd={fd_reached} write={wrote_outside} work={wrote_inside}"
    );
}

/// The descriptors above 2 this process holds that it did not open: those /proc/self/fd and
/// /proc/self/fdinfo list, but the one each listing opens to read itself.
fn inherited_descriptors() -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for dir in ["/proc/self/fd", "/proc/self/fdinfo"] {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(n) = name.parse::<u32>() else { continue };
            if n <= 2 {
                continue;
            }
            let target = fs::read_link(format!("/proc/self/fd/{n}")).map(|t| t.to_string_lossy().to_string()).unwrap_or_default();
            if target.starts_with("/proc/") && (target.ends_with("/fd") || target.ends_with("/fdinfo")) {
                continue;
            }
            if !found.contains(&name) {
                found.push(name);
            }
        }
    }
    found
}

/// Whether anything is reachable through descriptor `n`: a directory listed or a file read through
/// its /proc link, or a planted path under the directory it names read or, for a socket, connected.
fn through(n: &str, planted: &[&String]) -> bool {
    let base = format!("/proc/self/fd/{n}");
    let listed = fs::read_dir(&base).map(|mut d| d.next().is_some()).unwrap_or(false);
    let read = fs::read(&base).is_ok();
    let target = fs::read_link(&base).map(|t| t.to_string_lossy().to_string()).unwrap_or_default();
    let under = planted.iter().any(|p| {
        let Some(rest) = p.strip_prefix(&format!("{target}/")) else { return false };
        let path = format!("{base}/{rest}");
        !target.is_empty() && (fs::read(&path).is_ok() || reaches(&path))
    });
    listed || read || under
}

/// Whether a Unix socket at `path` accepts a connection.
#[cfg(unix)]
fn reaches(path: &str) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

#[cfg(not(unix))]
fn reaches(_path: &str) -> bool {
    false
}
