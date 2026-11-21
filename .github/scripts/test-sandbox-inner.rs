// The tests' sandbox's first process (.github/scripts/test-sandbox.sh; docs/design-review.md,
// untrusted-test-isolation): one program that does what the shell's `inner` did with seven, so that
// the sandbox costs a process less (the measurement of 2026-11-17: two thirds of a process's cost
// was those seven programs). It runs as root, as the first process of the namespaces `unshare`
// made, with the wrapper's lifeline on its standard input:
//
//   inner <cwd> <ulimit -v> <ulimit -f> [NAME=VALUE...] -- <command> [argument...]
//
// It mounts an empty /tmp and target/tmp of the namespace's own and its own /proc, makes the
// sandbox's root the root and drops the old one, enters the command's directory, sets the limits
// of the shell that started the wrapper and a core limit of one byte, starts the watcher that ends
// every process here when the wrapper ends, then runs the command as the sandbox's user, with no
// group, no capability, no way to gain one and only the environment named, waits for it, and fails
// it if it left a process behind. Its every effect on the command is the one the shell's `inner`
// had: the canary runs both and compares what each command sees.
//
// Built at setup by the toolchain rust-toolchain.toml pins, with rustc alone and no crate:
//
//   rustc --edition 2021 -C opt-level=2 -C panic=abort -o inner .github/scripts/test-sandbox-inner.rs
//
// Every system call that fails ends it with a message and status 1, before the command runs: the
// sandbox fails closed.

use std::ffi::{CString, OsString};
use std::os::raw::{c_char, c_int, c_long, c_ulong, c_void};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

// The sandbox's home, its root, and the user and group the command runs as (test-sandbox.sh).
const HOME: &str = "/opt/mente-test-sandbox";
const ROOT: &str = "/opt/mente-test-sandbox/root";
const UID: u32 = 4242424;
const GID: u32 = 4242424;

// Linux's numbers, the same on x86-64 and AArch64 but for the two system calls glibc wraps not.
const MS_NOSUID: c_ulong = 2;
const MS_NODEV: c_ulong = 4;
const MS_NOEXEC: c_ulong = 8;
const MNT_DETACH: c_int = 2;
const RLIMIT_FSIZE: c_int = 1;
const RLIMIT_CORE: c_int = 4;
const RLIMIT_AS: c_int = 9;
const RLIM_INFINITY: u64 = u64::MAX;
const PR_CAPBSET_DROP: c_int = 24;
const PR_SET_NO_NEW_PRIVS: c_int = 38;
const PR_CAP_AMBIENT: c_int = 47;
const PR_CAP_AMBIENT_CLEAR_ALL: c_ulong = 4;
const CAPABILITY_VERSION_3: u32 = 0x2008_0522;
const SIGKILL: c_int = 9;
const SIGPIPE: c_int = 13;
const SIG_DFL: usize = 0;
const O_RDWR: c_int = 2;
const WNOHANG: c_int = 1;
#[cfg(target_arch = "x86_64")]
const SYS_PIVOT_ROOT: c_long = 155;
#[cfg(target_arch = "x86_64")]
const SYS_CAPSET: c_long = 126;
#[cfg(target_arch = "aarch64")]
const SYS_PIVOT_ROOT: c_long = 41;
#[cfg(target_arch = "aarch64")]
const SYS_CAPSET: c_long = 91;

#[repr(C)]
struct Rlimit {
    cur: u64,
    max: u64,
}

#[repr(C)]
struct CapHeader {
    version: u32,
    pid: c_int,
}

#[repr(C)]
struct CapData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

extern "C" {
    fn mount(source: *const c_char, target: *const c_char, fstype: *const c_char, flags: c_ulong, data: *const c_void) -> c_int;
    fn umount2(target: *const c_char, flags: c_int) -> c_int;
    fn syscall(number: c_long, ...) -> c_long;
    fn chdir(path: *const c_char) -> c_int;
    fn getpid() -> c_int;
    fn fork() -> c_int;
    fn execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char) -> c_int;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn kill(pid: c_int, signal: c_int) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    fn dup2(old: c_int, new: c_int) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    fn setrlimit(resource: c_int, limit: *const Rlimit) -> c_int;
    fn setgroups(size: usize, list: *const u32) -> c_int;
    fn setresgid(real: u32, effective: u32, saved: u32) -> c_int;
    fn setresuid(real: u32, effective: u32, saved: u32) -> c_int;
    fn prctl(option: c_int, arg2: c_ulong, arg3: c_ulong, arg4: c_ulong, arg5: c_ulong) -> c_int;
    fn signal(signal: c_int, handler: usize) -> usize;
    fn _exit(status: c_int) -> !;
}

/// Ends the program as the shell's `set -e` would have: a message on standard error, status `code`.
fn fail(code: i32, message: &str) -> ! {
    eprintln!("test-sandbox: {message}");
    std::process::exit(code)
}

/// The last error of the system, for a message.
fn errno() -> std::io::Error {
    std::io::Error::last_os_error()
}

/// A C string of a path or a word, which holds no NUL: an argument that does is refused.
fn c(text: impl Into<Vec<u8>>) -> CString {
    CString::new(text).unwrap_or_else(|_| fail(2, "an argument holds a NUL byte"))
}

/// A system call that returns -1 on failure, checked: its failure ends the program.
fn checked(result: c_int, what: &str) {
    if result == -1 {
        fail(1, &format!("{what}: {}", errno()));
    }
}

fn mount_fs(source: &str, target: &[u8], fstype: &str, flags: c_ulong, data: Option<&str>) {
    let source = c(source);
    let target_c = c(target.to_vec());
    let fstype = c(fstype);
    let data = data.map(c);
    let data_ptr = data.as_ref().map_or(std::ptr::null(), |d| d.as_ptr() as *const c_void);
    // SAFETY: every pointer is to a NUL-terminated string that outlives the call, or null for data.
    let result = unsafe { mount(source.as_ptr(), target_c.as_ptr(), fstype.as_ptr(), flags, data_ptr) };
    checked(result, &format!("mount {}", String::from_utf8_lossy(target)));
}

/// The directory this process is in, as the kernel resolved it, every link followed.
fn cwd() -> Vec<u8> {
    std::env::current_dir().unwrap_or_else(|e| fail(1, &format!("getcwd: {e}"))).into_os_string().into_vec()
}

/// `cd -P`: into `dir`, true when the kernel let it, with every link on the way resolved.
fn enter(dir: &[u8]) -> bool {
    let dir = c(dir.to_vec());
    // SAFETY: a NUL-terminated path that outlives the call.
    unsafe { chdir(dir.as_ptr()) == 0 }
}

/// A limit as `ulimit` sets it, soft and hard: `unlimited`, or a count of `unit` bytes.
fn limit(resource: c_int, value: &str, unit: u64, what: &str) {
    let bytes = if value == "unlimited" {
        RLIM_INFINITY
    } else {
        match value.parse::<u64>() {
            Ok(n) => n.checked_mul(unit).unwrap_or_else(|| fail(2, &format!("{what}: {value} is past every limit"))),
            Err(_) => fail(2, &format!("{what}: {value} is no limit")),
        }
    };
    let rlimit = Rlimit { cur: bytes, max: bytes };
    // SAFETY: a pointer to a limit that outlives the call.
    checked(unsafe { setrlimit(resource, &rlimit) }, what);
}

/// The processes alive in this namespace beside its first and the watcher. A process has ended
/// when the kernel says so (State: Z) and no other thread of it runs: its name, which it chooses,
/// is never read, and one whose first thread alone has ended still counts.
fn remaining(watcher: c_int) -> Vec<String> {
    let mut left = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return left };
    let mut pids: Vec<u64> = entries.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok()).collect();
    pids.sort_unstable();
    for pid in pids {
        if pid == 1 || pid == watcher as u64 {
            continue;
        }
        let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else { continue };
        let mut state = "";
        let mut threads = 1;
        for line in status.lines() {
            if let Some(v) = line.strip_prefix("State:") {
                state = v.split_whitespace().next().unwrap_or("");
            } else if let Some(v) = line.strip_prefix("Threads:") {
                threads = v.trim().parse().unwrap_or(1);
            }
        }
        if state.is_empty() || (state == "Z" && threads <= 1) {
            continue;
        }
        left.push(pid.to_string());
    }
    left
}

/// The command's program: its path, or, for a bare name, the first of PATH=/usr/bin:/bin that is a
/// file it may run, as env(1) finds it in the environment it set.
fn program(name: &[u8]) -> Vec<u8> {
    if name.contains(&b'/') {
        return name.to_vec();
    }
    for dir in [&b"/usr/bin/"[..], &b"/bin/"[..]] {
        let candidate = [dir, name].concat();
        if Path::new(std::ffi::OsStr::from_bytes(&candidate)).is_file() {
            return candidate;
        }
    }
    name.to_vec()
}

/// The command's own process, never returning: the privileges dropped, as setpriv dropped them, and
/// the command run with only the environment named, as `env -i` ran it.
fn command(envs: &[Vec<u8>], argv: &[Vec<u8>]) -> ! {
    // The lifeline is the wrapper's and the watcher's alone; signals as the shell left them, none
    // ignored (Rust's start ignores SIGPIPE, which a command would inherit).
    // SAFETY: plain system calls on numbers.
    unsafe {
        close(8);
        signal(SIGPIPE, SIG_DFL);
    }
    let last_cap: c_ulong = std::fs::read_to_string("/proc/sys/kernel/cap_last_cap")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(|| fail(1, "cap_last_cap: unreadable"));
    // SAFETY: every call takes numbers or pointers to values that outlive it.
    unsafe {
        // No capability any process here may hold again: the bounding set, emptied while root may.
        for cap in 0..=last_cap {
            checked(prctl(PR_CAPBSET_DROP, cap, 0, 0, 0), "drop a capability of the bounding set");
        }
        checked(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), "no_new_privs");
        checked(setgroups(0, std::ptr::null()), "setgroups");
        checked(setresgid(GID, GID, GID), "setresgid");
        checked(setresuid(UID, UID, UID), "setresuid");
        // The user change emptied the permitted and effective sets; the inheritable and ambient ones
        // are emptied too.
        let header = CapHeader { version: CAPABILITY_VERSION_3, pid: 0 };
        let data = [CapData { effective: 0, permitted: 0, inheritable: 0 }, CapData { effective: 0, permitted: 0, inheritable: 0 }];
        if syscall(SYS_CAPSET, &header as *const CapHeader, data.as_ptr()) != 0 {
            fail(1, &format!("capset: {}", errno()));
        }
        checked(prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0), "clear the ambient capabilities");
    }
    // The environment `env -i <envs> PATH=/usr/bin:/bin HOME=/tmp TMPDIR=/tmp` gives: each in turn,
    // a later value of a name in the earlier one's place, as putenv(3) sets it.
    let mut env: Vec<Vec<u8>> = Vec::new();
    for entry in envs.iter().cloned().chain([b"PATH=/usr/bin:/bin".to_vec(), b"HOME=/tmp".to_vec(), b"TMPDIR=/tmp".to_vec()]) {
        let name = |e: &[u8]| e.split(|&b| b == b'=').next().unwrap_or(&[]).to_vec();
        match env.iter().position(|e| name(e) == name(&entry)) {
            Some(k) => env[k] = entry,
            None => env.push(entry),
        }
    }
    let path = c(program(&argv[0]));
    let args: Vec<CString> = argv.iter().map(|a| c(a.clone())).collect();
    let envc: Vec<CString> = env.into_iter().map(c).collect();
    let mut argp: Vec<*const c_char> = args.iter().map(|a| a.as_ptr()).collect();
    argp.push(std::ptr::null());
    let mut envp: Vec<*const c_char> = envc.iter().map(|e| e.as_ptr()).collect();
    envp.push(std::ptr::null());
    // SAFETY: NUL-terminated strings in NULL-terminated arrays, all alive at the call.
    unsafe { execve(path.as_ptr(), argp.as_ptr(), envp.as_ptr()) };
    let e = errno();
    eprintln!("env: '{}': {e}", String::from_utf8_lossy(&argv[0]));
    // SAFETY: ends this child at once, as a failed exec does.
    unsafe { _exit(if e.kind() == std::io::ErrorKind::NotFound { 127 } else { 126 }) }
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.len() < 3 {
        fail(2, "no command");
    }
    let (cwd_arg, vlimit, flimit) = (args[0].as_bytes().to_vec(), args[1].to_string_lossy().into_owned(), args[2].to_string_lossy().into_owned());
    let rest = &args[3..];
    let split = rest.iter().position(|a| a.as_bytes() == b"--");
    let (envs, argv): (Vec<Vec<u8>>, Vec<Vec<u8>>) = match split {
        Some(k) if k + 1 < rest.len() => (rest[..k].iter().map(|a| a.as_bytes().to_vec()).collect(), rest[k + 1..].iter().map(|a| a.as_bytes().to_vec()).collect()),
        _ => fail(2, "no command"),
    };
    // SAFETY: a plain system call.
    if unsafe { getpid() } != 1 {
        fail(2, "not the first process of a namespace of its own");
    }
    let workspace = std::fs::read_to_string(format!("{HOME}/workspace")).unwrap_or_else(|e| fail(1, &format!("{HOME}/workspace: {e}")));
    let workspace = workspace.lines().next().unwrap_or("").to_string();
    // The lifeline on 8, and nothing on standard input.
    let null = c("/dev/null");
    // SAFETY: plain system calls on numbers and a NUL-terminated path.
    unsafe {
        checked(dup2(0, 8), "dup2 the lifeline");
        let fd = open(null.as_ptr(), O_RDWR);
        checked(fd, "open /dev/null");
        checked(dup2(fd, 0), "dup2 /dev/null");
        if fd != 0 {
            close(fd);
        }
    }
    // Memory backs the temporary filesystems, so they are bounded: four tests at once write at most
    // 6 GB.
    mount_fs("tmpfs", format!("{ROOT}/tmp").as_bytes(), "tmpfs", MS_NOSUID | MS_NODEV, Some("mode=1777,size=1g"));
    // Where cargo sends an integration test's files: that very directory, no link and nothing under
    // a link, mounted on the directory this process entered by resolving every link, which the
    // kernel follows from /proc/self/cwd, never on its name resolved again.
    let dir = format!("{ROOT}{workspace}/target/tmp");
    if std::fs::symlink_metadata(&dir).is_ok() {
        let is_dir = std::fs::metadata(&dir).map(|m| m.is_dir()).unwrap_or(false);
        if !is_dir || !enter(dir.as_bytes()) || cwd() != dir.as_bytes() {
            fail(2, &format!("{workspace}/target/tmp is a link, lies under one or is no directory: root mounts nothing through a path the change can aim elsewhere"));
        }
        mount_fs("tmpfs", b"/proc/self/cwd", "tmpfs", MS_NOSUID | MS_NODEV, Some("mode=1777,size=512m"));
    }
    mount_fs("proc", format!("{ROOT}/proc").as_bytes(), "proc", MS_NOSUID | MS_NODEV | MS_NOEXEC, None);
    if !enter(ROOT.as_bytes()) {
        fail(1, &format!("cd {ROOT}: {}", errno()));
    }
    let (dot, old) = (c("."), c(".old"));
    // SAFETY: two NUL-terminated paths that outlive the call.
    if unsafe { syscall(SYS_PIVOT_ROOT, dot.as_ptr(), old.as_ptr()) } != 0 {
        fail(1, &format!("pivot_root: {}", errno()));
    }
    let gone = c("/.old");
    // SAFETY: a NUL-terminated path that outlives the call.
    checked(unsafe { umount2(gone.as_ptr(), MNT_DETACH) }, "umount /.old");
    // The command starts where cargo started it, a directory of the workspace itself.
    if !enter(&cwd_arg) {
        fail(2, &format!("the working directory {} is not in the sandbox", String::from_utf8_lossy(&cwd_arg)));
    }
    let here = cwd();
    let mut inside = workspace.as_bytes().to_vec();
    inside.push(b'/');
    if !([here.as_slice(), b"/"].concat()).starts_with(&inside) {
        fail(2, &format!("the working directory {} leads out of the workspace, to {}", String::from_utf8_lossy(&cwd_arg), String::from_utf8_lossy(&here)));
    }
    limit(RLIMIT_AS, &vlimit, 1024, "ulimit -v");
    limit(RLIMIT_FSIZE, &flimit, 1024, "ulimit -f");
    // A core limit of one byte, which the command cannot raise: no crash writes a core file.
    limit(RLIMIT_CORE, "1", 1, "the core limit");
    // The watcher: when the wrapper ends or asks, every process here but this one ends, and it goes
    // on ending them for as long as this process lives.
    // SAFETY: fork in a process of one thread; the child calls only system calls and exits.
    let watcher = unsafe { fork() };
    checked(watcher, "fork the watcher");
    if watcher == 0 {
        // SAFETY: plain system calls; the child never returns.
        unsafe {
            let null = open(null.as_ptr(), O_RDWR);
            if null >= 0 {
                dup2(null, 1);
                dup2(null, 2);
            }
            let mut byte = 0u8;
            while read(8, &mut byte as *mut u8 as *mut c_void, 1) == 1 && byte != b'\n' {}
            loop {
                kill(-1, SIGKILL);
            }
        }
    }
    // SAFETY: fork in a process of one thread; the child drops its privileges and execs.
    let child = unsafe { fork() };
    checked(child, "fork the command");
    if child == 0 {
        command(&envs, &argv);
    }
    let mut raw = 0;
    loop {
        // SAFETY: a pointer to a status that outlives the call.
        let r = unsafe { waitpid(child, &mut raw, 0) };
        if r == child {
            break;
        }
        if r == -1 && errno().kind() != std::io::ErrorKind::Interrupted {
            fail(1, &format!("waitpid: {}", errno()));
        }
    }
    let signal_number = raw & 0x7f;
    let mut status = if signal_number == 0 { (raw >> 8) & 0xff } else { 128 + signal_number };
    // A process the command left behind ends with the namespace. As under nextest's leak-timeout,
    // 100 ms after the command, that fails it.
    let mut left = remaining(watcher);
    for _ in 0..10 {
        if left.is_empty() {
            break;
        }
        // Children of this process that have ended are reaped, so that they count as gone.
        // SAFETY: a pointer to a status that outlives the call.
        while unsafe { waitpid(-1, &mut raw, WNOHANG) } > 0 {}
        std::thread::sleep(std::time::Duration::from_millis(10));
        left = remaining(watcher);
    }
    if !left.is_empty() {
        eprintln!("test-sandbox: the command left processes behind, which end with it: {}", left.join(" "));
        if status == 0 {
            status = 1;
        }
    }
    std::process::exit(status)
}
