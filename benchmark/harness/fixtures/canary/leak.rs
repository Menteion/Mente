// A canary for the benchmark sandbox (benchmark/harness/sandbox-check.ts). Run as an agent program
// would be, it tries to read a secret from the driver's process, from a planted file and from its
// own environment, and to see a network interface beyond loopback. It prints one line of flags,
// never the secret itself; the check reads the flags. Under the sandbox every flag must be false;
// without it, as a control, the critical ones are true, so a green result is not vacuous.
use std::fs;

fn main() {
    let planted = std::env::args().nth(1).unwrap_or_default();
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

    // A file planted outside the working directory, under a credential directory.
    let file_secret = !planted.is_empty() && fs::read(&planted).is_ok();

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

    println!("env={env_secret} proc={proc_secret} file={file_secret} net={has_network}");
}
