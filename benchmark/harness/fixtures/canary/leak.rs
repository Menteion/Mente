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
// It also writes a marker into every directory up to two levels below each mount it sees as
// writable, but its own directory, and lists where it wrote in canary-wrote.txt there: the check
// looks for each from outside, where any it finds is a channel from one run to another.
//
//   leak <credential file> <workspace file> <held-out file> <directory to write into> <another run's file> <the broker's socket> <tag>
use std::fs;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    let (credential, workspace, held_out, outside, other_run, broker, tag) = (arg(1), arg(2), arg(3), arg(4), arg(5), arg(6), arg(7));
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

    // A marker on every writable mount, for the check to look for from outside.
    let shared = shared_writes(if tag.is_empty() { "untagged" } else { &tag });
    let _ = fs::write("canary-wrote.txt", shared.join("\n"));
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

/// The mount points this process sees as writable, by /proc/self/mountinfo: each whose own options
/// say rw, of a filesystem that holds files a program can make, so not proc, sysfs and their kind.
fn writable_mounts() -> Vec<String> {
    let kernel = ["proc", "sysfs", "cgroup", "cgroup2", "devpts", "securityfs", "debugfs", "tracefs", "bpf", "pstore", "configfs", "fusectl", "autofs", "binfmt_misc", "efivarfs", "selinuxfs", "nsfs", "rpc_pipefs"];
    let Ok(info) = fs::read_to_string("/proc/self/mountinfo") else { return Vec::new() };
    let mut found: Vec<String> = Vec::new();
    for line in info.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let Some(dash) = fields.iter().position(|f| *f == "-") else { continue };
        let (Some(point), Some(options), Some(fstype)) = (fields.get(4), fields.get(5), fields.get(dash + 1)) else { continue };
        if !options.split(',').any(|o| o == "rw") || kernel.contains(fstype) {
            continue;
        }
        let point = point.replace("\\040", " ").replace("\\011", "\t").replace("\\012", "\n").replace("\\134", "\\");
        if !found.contains(&point) {
            found.push(point);
        }
    }
    found
}

/// Makes a file named for `tag` in every directory up to two levels below each writable mount, but
/// its own directory, and returns where it could: each is a place another run might also reach.
fn shared_writes(tag: &str) -> Vec<String> {
    let own_dir = std::env::current_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let name = format!("bench-canary-{tag}");
    let mut wrote: Vec<String> = Vec::new();
    for mount in writable_mounts() {
        let mut dirs = vec![(Path::new(&mount).to_path_buf(), 0)];
        while let Some((dir, depth)) = dirs.pop() {
            let shown = dir.to_string_lossy().to_string();
            if !own_dir.is_empty() && (shown == own_dir || shown.starts_with(&format!("{own_dir}/"))) {
                continue;
            }
            let probe = dir.join(&name);
            if fs::OpenOptions::new().write(true).create_new(true).open(&probe).is_ok() {
                let shown = probe.to_string_lossy().to_string();
                if !wrote.contains(&shown) {
                    wrote.push(shown);
                }
            }
            if depth < 2 {
                let Ok(entries) = fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        dirs.push((entry.path(), depth + 1));
                    }
                }
            }
        }
    }
    wrote
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
