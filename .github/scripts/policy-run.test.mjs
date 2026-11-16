// The workflow policy's run tests (policy-runtime-judge in docs/design-review.md, its PR 2): every
// check that runs a script or a program of the tree, which the policy itself
// (.github/scripts/policy.test.mjs) no longer does. They test the change's own scripts in the
// change's own run, as the crates' tests do; the code owner's review and the pins of
// policy.test.mjs hold this file, the fixtures it runs and the scripts they run in turn. Each check
// counts itself when it runs, and the suite fails when fewer ran than FLOOR, the checks below:
// test_the_documentation_job_runs_the_workflow_policy keeps FLOOR equal to them.
import { after, test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import http from "node:http";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { parseYaml } from "./lib/yaml.mjs";
import { PATHS_FILE } from "./lib/watch.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "../..");
const read = (file) => fs.readFileSync(path.join(ROOT, file), "utf8");
const WORKFLOWS = ".github/workflows";
/// The text of every workflow.
const workflowText = () =>
  fs
    .readdirSync(path.join(ROOT, WORKFLOWS))
    .filter((f) => f.endsWith(".yml") || f.endsWith(".yaml"))
    .map((f) => read(`${WORKFLOWS}/${f}`));

const FLOOR = 11;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the run tests' ${FLOOR} checks ran: a skip or a filter left the rest out`);
});

/// git, in the repository.
const git = (args) => execFileSync("git", args, { cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });

/// The bash the run tests run scripts in, by its path: on Windows, Git's, beside the git on PATH,
/// since a bare `bash` there may be WSL's; elsewhere /usr/bin/bash, since a bare name is looked up on
/// the PATH a test hands the child, where a shim may stand first.
function bashPath() {
  if (process.platform !== "win32") return "/usr/bin/bash";
  const found = path.resolve(git(["--exec-path"]).trim(), "../../../usr/bin/bash.exe");
  assert.ok(fs.existsSync(found), `no bash beside git at ${found}`);
  return found;
}

check("overflow-proof.sh counts each crate's own overflow test, under a cargo that prints what it is told", () => {
  // What it reads, with a stand-in for cargo that prints, for the package it is asked for, the
  // lines given: a crate that prints the passing test twice does not vouch for one that shows
  // none, as it did while every crate's output was counted together.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "overflow-proof-"));
  try {
    for (const c of ["a", "b"]) {
      fs.mkdirSync(path.join(dir, "compiler", c, "src"), { recursive: true });
      fs.writeFileSync(path.join(dir, "compiler", c, "Cargo.toml"), `[package]\nname = "mente-${c}"\n`);
      fs.writeFileSync(path.join(dir, "compiler", c, "src", "lib.rs"), "mod overflow_traps;\n");
      fs.writeFileSync(path.join(dir, "compiler", c, "src", "overflow_traps.rs"), "#[test]\nfn test_arithmetic_overflow_panics_in_this_crate() {}\n");
    }
    fs.mkdirSync(path.join(dir, "bin"));
    const OK = "test overflow_traps::test_arithmetic_overflow_panics_in_this_crate ... ok";
    const proofWith = (printed, status = 0) => {
      const arms = Object.entries(printed).map(([pkg, lines]) => `  ${pkg}) printf '%s\\n' ${lines.map((l) => `'${l}'`).join(" ")} 'test result: ok.' ;;\n`);
      const stub = path.join(dir, "bin", "cargo");
      fs.writeFileSync(stub, `#!/usr/bin/env bash\nwhile [ "$#" -gt 0 ] && [ "$1" != -p ]; do shift; done\ncase "\${2:-}" in\n${arms.join("")}esac\nexit ${status}\n`);
      fs.chmodSync(stub, 0o755);
      const env = { ...process.env, PATH: `${path.join(dir, "bin")}${path.delimiter}${process.env.PATH}`, RUNNER_TEMP: dir.replace(/\\/g, "/"), GITHUB_STEP_SUMMARY: "" };
      const r = spawnSync(bashPath(), [path.join(ROOT, ".github/scripts/overflow-proof.sh").replace(/\\/g, "/")], { cwd: dir, encoding: "utf8", env, timeout: 30000 });
      return { status: r.status, out: `${r.stdout}${r.stderr}` };
    };
    const both = proofWith({ "mente-a": [OK], "mente-b": [OK] });
    assert.equal(both.status, 0, `the proof refuses two crates that each show their test: ${both.out}`);
    assert.match(both.out, /Crates whose release build traps on an overflow: 2 of 2/);
    const vouched = proofWith({ "mente-a": [OK, OK], "mente-b": [] });
    assert.equal(vouched.status, 1, `one crate vouches for another: ${vouched.out}`);
    assert.match(vouched.out, /::error::mente-a does not show its own overflow test passing, once/);
    assert.match(vouched.out, /::error::mente-b does not show its own overflow test passing, once/);
    assert.equal(proofWith({ "mente-a": [OK], "mente-b": [] }).status, 1, "a crate without its test passes");
    assert.equal(proofWith({ "mente-a": [OK], "mente-b": [OK] }, 101).status, 1, "a crate whose tests failed passes");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("limits.sh refuses an alias, a runner or a compiler from outside the repository", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "limits-"));
  try {
    const home = path.join(dir, "home");
    const other = path.join(dir, "other");
    fs.mkdirSync(home);
    fs.mkdirSync(other);
    fs.writeFileSync(path.join(other, "config.toml"), '[ alias ]\nnextest = "test --no-run --"\n');
    // Sourced on a runner the script knows no limits for, it stops either at a refusal or at the
    // case of the runner, after every refusal passed.
    const run = (vars, cargoHome = home) => {
      const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !/^(CARGO_|NEXTEST_|RUSTC|RUSTDOC)/i.test(k)));
      const r = spawnSync(bashPath(), ["-eo", "pipefail", ".github/scripts/fixtures/source-limits.sh"], { cwd: ROOT, encoding: "utf8", env: { ...env, ...vars, CARGO_HOME: cargoHome.replace(/\\/g, "/"), RUNNER_OS: "Elsewhere" }, timeout: 30000 });
      return `${r.stdout}${r.stderr}`;
    };
    assert.match(run({}), /defines no limits for Elsewhere/, "limits.sh refuses a clean environment");
    assert.match(run({ NEXTEST_PROFILE: "mutants" }), /defines no limits for Elsewhere/, "the mutation runs' profile is refused");
    assert.match(run({ RUSTFLAGS: "-D warnings" }), /defines no limits for Elsewhere/, "the Rust jobs' flags are refused");
    // Under the mutation runs' profile, and only there, every build caps the lints at warnings; a
    // cap already in the environment is refused, which could undo the Rust jobs' `-D warnings`.
    const flags = (vars) => {
      const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !/^(CARGO_|NEXTEST_|RUSTC|RUSTDOC)/i.test(k)));
      const r = spawnSync(bashPath(), ["-eo", "pipefail", ".github/scripts/fixtures/limits-flags.sh"], { cwd: ROOT, encoding: "utf8", env: { ...env, ...vars, CARGO_HOME: home.replace(/\\/g, "/"), RUNNER_OS: "Elsewhere" }, timeout: 30000 });
      return /flags=\[([^\]]*)\]/.exec(r.stdout)?.[1];
    };
    assert.equal(flags({ NEXTEST_PROFILE: "mutants" }), "--cap-lints=warn", "limits.sh does not cap the lints of a mutation run");
    assert.equal(flags({}), "unset", "limits.sh caps the lints outside a mutation run");
    assert.equal(flags({ RUSTFLAGS: "-D warnings" }), "unset", "limits.sh caps the lints of the Rust jobs");
    assert.match(run({ NEXTEST_PROFILE: "mutants", CARGO_ENCODED_RUSTFLAGS: "--cap-lints=warn" }), /would change what the tests run/, "a cap of the lints in the environment passes");
    for (const vars of [
      { RUSTFLAGS: "-C overflow-checks=off" },
      { rustflags: "--cap-lints allow" },
      { CARGO_ENCODED_RUSTFLAGS: "x" },
      { CARGO_BUILD_RUSTFLAGS: "x" },
      { CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS: "x" },
      { RUSTDOCFLAGS: "x" },
      { RUSTUP_TOOLCHAIN: "nightly" },
      { BASH_ENV: "/tmp/x" },
      { CARGO_PROFILE_DEV_OVERFLOW_CHECKS: "false" },
      { CARGO_PROFILE_TEST_DEBUG_ASSERTIONS: "false" },
      { CARGO_ALIAS_NEXTEST: "test --no-run --" },
      { CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER: "true" },
      { cargo_target_x86_64_pc_windows_msvc_runner: "true" },
      { CARGO_BUILD_RUSTC_WRAPPER: "x" },
      { RUSTC_WRAPPER: "x" },
      { RUSTC: "x" },
      { NEXTEST_RETRIES: "3" },
      { NEXTEST_PROFILE: "default" },
    ]) {
      assert.match(run(vars), /::error::a variable above would change what the tests run/, `limits.sh lets through ${JSON.stringify(vars)}`);
    }
    assert.match(run({}, other), /::error::a cargo configuration file outside the repository/, "limits.sh lets through a cargo configuration file in CARGO_HOME");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("tools.sh finds a tool replaced or added, and a step that sets the next one's path", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tools-"));
  try {
    const at = (...p) => path.join(dir, ...p);
    for (const d of [["home", "bin"], ["rustup", "toolchains", "t", "bin"], ["temp"]]) fs.mkdirSync(at(...d), { recursive: true });
    fs.writeFileSync(at("home", "bin", "cargo-nextest"), "nextest");
    fs.writeFileSync(at("rustup", "toolchains", "t", "bin", "cargo"), "cargo");
    fs.writeFileSync(at("path"), "");
    const slash = (p) => p.replace(/\\/g, "/");
    const run = (mode) => {
      const env = { ...process.env, CARGO_HOME: slash(at("home")), RUSTUP_HOME: slash(at("rustup")), RUNNER_TEMP: slash(at("temp")), GITHUB_PATH: slash(at("path")), GITHUB_ENV: "" };
      const r = spawnSync(bashPath(), [".github/scripts/tools.sh", mode], { cwd: ROOT, encoding: "utf8", env, timeout: 30000 });
      return { status: r.status, out: `${r.stdout}${r.stderr}` };
    };
    assert.equal(run("verify").status, 1, "tools.sh verifies without a record");
    assert.equal(run("record").status, 0, "tools.sh does not record");
    assert.equal(run("verify").status, 0, `tools.sh refuses unchanged tools: ${run("verify").out}`);
    fs.writeFileSync(at("home", "bin", "cargo-nextest"), "#!/bin/sh\nexit 1\n");
    assert.match(run("verify").out, /a tool, a setting or a script changed since it was recorded/, "tools.sh lets a stub in place of nextest through");
    fs.writeFileSync(at("home", "bin", "cargo-nextest"), "nextest");
    fs.writeFileSync(at("rustup", "toolchains", "t", "bin", "rustc"), "rustc");
    assert.match(run("verify").out, /a tool, a setting or a script changed since it was recorded/, "tools.sh lets a tool added to a toolchain through");
    fs.rmSync(at("rustup", "toolchains", "t", "bin", "rustc"));
    fs.writeFileSync(at("path"), "/tmp/stubs\n");
    assert.match(run("verify").out, /set the path or the environment of the steps after it/, "tools.sh lets a step set the path of the next one");
    fs.writeFileSync(at("path"), "");
    assert.equal(run("verify").status, 0, "tools.sh does not return to passing once the tools are as recorded");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("test-sandbox-env.sh stops a step with no sandbox, and test-sandbox.sh names, counts and bounds what ran in it", () => {
  const sourced = (os_) => {
    const env = { ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !/_RUNNER$/i.test(k))), RUNNER_OS: os_ };
    const r = spawnSync(bashPath(), ["-eo", "pipefail", ".github/scripts/fixtures/sandbox-env.sh"], { cwd: ROOT, encoding: "utf8", env, timeout: 30000 });
    return { status: r.status, out: `${r.stdout}${r.stderr}` };
  };
  const stopped = sourced("Linux");
  assert.equal(stopped.status, 1, `a step went on with no sandbox set up: ${stopped.out}`);
  assert.match(stopped.out, /::error::the sandbox is not set up/);
  assert.doesNotMatch(stopped.out, /the step went on/);
  assert.deepEqual(sourced("Windows"), { status: 0, out: "the step went on, its runner none\n" }, "test-sandbox-env.sh changes a step on Windows, which has no sandbox");
  // What the script does on any machine: it names no runner where no sandbox is set up, it fails a
  // step whose tools ran nothing in the sandbox, and it refuses a command it could not hand over
  // whole. The sandbox itself is proven where it runs, by the canary.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "test-sandbox-"));
  try {
    const run = (args, os_, temp = dir) => {
      const env = { ...process.env, RUNNER_OS: os_, RUNNER_TEMP: temp.replace(/\\/g, "/") };
      const r = spawnSync(bashPath(), [".github/scripts/test-sandbox.sh", ...args], { cwd: ROOT, encoding: "utf8", env, timeout: 30000 });
      return { status: r.status, out: `${r.stdout}${r.stderr}` };
    };
    const unset = run(["env"], "Linux");
    assert.equal(unset.status, 1, `test-sandbox.sh env names a runner with no sandbox set up: ${unset.out}`);
    assert.match(unset.out, /::error::the sandbox is not set up/);
    assert.doesNotMatch(unset.out, /RUNNER=/, "test-sandbox.sh env names a runner with no sandbox set up");
    assert.deepEqual(run(["env"], "Windows"), { status: 0, out: "" }, "test-sandbox.sh env names a runner on Windows, which has no sandbox");
    assert.match(run(["ran"], "Linux").out, /::error::no record of the commands that ran in the sandbox/);
    const log = path.join(dir, "test-sandbox-runs.log");
    // What the sandbox has cost a job: begun 100 s ago, 20 ms a process alone and 2 ms each at once,
    // and 3 s added so far. Each `ran` adds to it, so each case writes it afresh.
    const cost = path.join(dir, "test-sandbox-cost");
    const costs = (added, start = (Date.now() - 100_000) * 1000) => fs.writeFileSync(cost, `start ${start}\nserial 20000\nparallel 2000\nadded ${added}\n`);
    fs.writeFileSync(log, "");
    costs(3_000_000);
    assert.equal(run(["ran"], "Linux").status, 1, "a step whose tools ran nothing in the sandbox passes");
    costs(3_000_000);
    assert.equal(run(["ran", "0"], "Linux").status, 0);
    fs.writeFileSync(log, "target/debug/deps/a\ntarget/debug/deps/b\n");
    const began = (Date.now() - 100_000) * 1000;
    costs(3_000_000, began);
    const two = run(["ran"], "Linux");
    assert.equal(two.status, 0, `two commands in the sandbox are refused: ${two.out}`);
    // Two commands added 2 ms each and one 20 ms alone, and the rest of the record kept as it was
    // (the review of 41a54b6: a start left out of it made the job's time the epoch's, and the share
    // nothing).
    assert.equal(fs.readFileSync(cost, "utf8"), `start ${began}\nserial 20000\nparallel 2000\nadded 3024000\n`, "ran does not add each command's cost to the job's, or loses the rest of the record");
    assert.match(two.out, /The sandbox has added an estimated 3\.0 s to the 10\d\.\d s since setup: 3\.\d% /);
    costs(3_000_000);
    assert.equal(run(["ran", "2"], "Linux").status, 0);
    costs(3_000_000);
    assert.equal(run(["ran", "3"], "Linux").status, 1, "fewer commands in the sandbox than mutants caught passes");
    costs(3_000_000);
    assert.equal(run(["ran", "many"], "Linux").status, 1, "a count that is no number passes");
    // `ran` reports the share so far, whatever it is; `bound`, at the job's end, fails past a tenth
    // of the job's time without the sandbox, and on a record that is missing or not whole.
    costs(9_500_000);
    const so = run(["ran"], "Linux");
    assert.equal(so.status, 0, `ran holds the job's share before its end: ${so.out}`);
    assert.match(so.out, /The sandbox has added an estimated 9\.5 s to the 10\d\.\d s since setup: 10\.\d% /);
    const over = run(["bound"], "Linux");
    assert.equal(over.status, 1, `a sandbox that added more than a tenth to the job passes: ${over.out}`);
    assert.match(over.out, /::error::the sandbox has added more than a tenth to this job's time/);
    costs(8_000_000);
    assert.equal(run(["bound"], "Linux").status, 0, "a sandbox that added under a tenth to the job fails");
    // Close to the bound, over a job of 1000 s, where the milliseconds the script takes to start
    // move the share by a few thousandths of a point: 10.05% fails, which a share rounded down to
    // per mille would let through (the review of 41a54b6), and 9.95% passes.
    const thousand = (Date.now() - 1_000_000) * 1000;
    costs(91_320_000, thousand);
    assert.equal(run(["bound"], "Linux").status, 1, "a sandbox that added 10.05% to the job passes");
    costs(90_500_000, thousand);
    assert.equal(run(["bound"], "Linux").status, 0, "a sandbox that added 9.95% to the job fails");
    // An estimate no less than the whole time since setup is no estimate.
    costs(20_000_000, (Date.now() - 10_000) * 1000);
    assert.match(run(["bound"], "Linux").out, /::error::the sandbox's estimated cost, 20\.0 s, is no less than the job's time since setup/);
    assert.equal(run(["bound"], "Windows", path.join(dir, "none")).status, 0, "Windows, which has no sandbox, fails the bound");
    for (const sub of ["ran", "bound"]) {
      fs.writeFileSync(cost, "start 1\nserial 20000\nadded 0\n");
      assert.match(run([sub], "Linux").out, /::error::the record of what the sandbox costs this job is not whole/, sub);
      fs.rmSync(cost);
      assert.match(run([sub], "Linux").out, /::error::no record of what the sandbox costs this job/, sub);
    }
    assert.equal(run(["ran"], "Windows", path.join(dir, "none")).status, 0, "Windows, which has no sandbox, fails the check that a command ran in it");
    assert.equal(run(["run", "A=b"], "Linux").status, 2, "a command that env would read as a variable is handed over");
    assert.equal(run(["run"], "Linux").status, 2);
    assert.equal(run(["nothing"], "Linux").status, 2);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("the build-time check's CLI exits 1 with a line for a problem or an error, and 0 for none", () => {
  // The CLI's exit, which the jobs read (L-5 of the second review of 4e896ca): a line and 1 for a
  // problem or an error, 0 for none. It runs in a directory under target/, which the repository
  // ignores and above which no cargo configuration lies, with a cargo home of its own.
  fs.mkdirSync(path.join(ROOT, "target"), { recursive: true });
  const cli = fs.mkdtempSync(path.join(ROOT, "target", "build-time-cli-"));
  try {
    fs.mkdirSync(path.join(cli, "home"));
    fs.mkdirSync(path.join(cli, ".config"));
    fs.writeFileSync(path.join(cli, ".config", "approved-crates.txt"), "top: top\n");
    fs.writeFileSync(path.join(cli, "Cargo.toml"), '[workspace]\nmembers = ["compiler/a"]\n');
    const toolchainText = '[toolchain]\nchannel = "1.97.1"\ncomponents = ["clippy", "rustfmt"]\nprofile = "minimal"\n';
    const lockText = `# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 4\n\n[[package]]\nname = "leaf"\nversion = "2.0.0"\nsource = "git+https://example.invalid/leaf#abc"\n`;
    fs.writeFileSync(path.join(cli, "Cargo.lock"), lockText);
    const run = (...args) => spawnSync(process.execPath, [path.join(ROOT, ".github/scripts/build-time-code.mjs"), ...args], { cwd: cli, encoding: "utf8", env: { ...process.env, CARGO_HOME: path.join(cli, "home") } });
    // A checkout below a cargo configuration of the machine's, which cargo would read too, fails
    // the check by design; there the clean case shows that failure instead.
    const above = [];
    for (let d = path.dirname(cli); ; d = path.dirname(d)) {
      for (const name of ["config", "config.toml"]) if (fs.existsSync(path.join(d, ".cargo", name))) above.push(path.join(d, ".cargo", name));
      if (path.dirname(d) === d) break;
    }
    const missingToolchain = run("--before-toolchain");
    assert.equal(missingToolchain.status, 1, "the CLI does not fail when rust-toolchain.toml is missing");
    assert.match(missingToolchain.stderr, /^error: /m);
    fs.writeFileSync(path.join(cli, "rust-toolchain.toml"), toolchainText);
    const clean = run("--before-toolchain");
    if (above.length === 0) {
      assert.equal(clean.status, 0, `${clean.stdout}${clean.stderr}`);
      assert.match(clean.stdout, /names a release alone/);
    } else {
      assert.equal(clean.status, 1, `the CLI passes below ${above[0]}`);
      assert.ok(clean.stdout.includes(`::error::${above[0]}: a cargo configuration file`), clean.stdout);
    }
    fs.writeFileSync(path.join(cli, "rust-toolchain.toml"), toolchainText.replace('"1.97.1"', '"stable"'));
    const channel = run("--before-toolchain");
    assert.equal(channel.status, 1, "the CLI does not fail on a channel that is no release");
    assert.match(channel.stdout, /^::error::rust-toolchain\.toml:2: /m);
    fs.writeFileSync(path.join(cli, "rust-toolchain.toml"), toolchainText);
    fs.mkdirSync(path.join(cli, ".cargo"));
    fs.writeFileSync(path.join(cli, ".cargo", "config.toml"), "x\n");
    assert.equal(run("--before-toolchain").status, 1, "the CLI does not fail on a cargo configuration file");
    fs.rmSync(path.join(cli, ".cargo"), { recursive: true });
    // Without the flag, past the toolchain's check: a lock that names another source fails before
    // cargo is asked anything.
    const lockRun = run();
    assert.equal(lockRun.status, 1, "the CLI does not fail on a lock that names another source");
    if (above.length === 0) assert.match(lockRun.stdout, /^::error::.*leaf comes from git\+https/m);
  } finally {
    fs.rmSync(cli, { recursive: true, force: true });
  }
});

check("the memory guard goes on when the process it would stop is already gone", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "memory-guard-"));
  try {
    const meminfo = path.join(dir, "meminfo");
    fs.writeFileSync(meminfo, "MemTotal: 16384000 kB\nMemAvailable: 1024 kB\n");
    // Sourced in a subshell of a shell under errexit and pipefail, as the guard ran when it lost
    // itself, with `ps` naming a process that is gone, so that every `kill` fails, and `sleep`
    // counting the rounds instead of waiting: it lets five through and ends the loop at the sixth.
    // No clock decides: the guard either goes on through all five, and the subshell says so after
    // it, or stops at the first failed `kill`.
    const ROUNDS = 5;
    // `sudo` stands in too, and says what it was asked: the guard stops through it a process of
    // the sandbox's user, which the runner's user cannot stop, and no other
    // (.github/scripts/fixtures/memory-guard-rounds.sh).
    const guard = (owner) => [".github/scripts/fixtures/memory-guard-rounds.sh", owner, meminfo.replace(/\\/g, "/"), String(ROUNDS)];
    const r = spawnSync(bashPath(), ["-eo", "pipefail", ...guard("1001")], { cwd: ROOT, encoding: "utf8", timeout: 30000 });
    const rounds = (String(r.stdout).match(/^memory guard: 1 MB available, stopping /gm) ?? []).length;
    assert.equal(rounds, ROUNDS, `the guard stopped after ${rounds} of ${ROUNDS} rounds: ${r.error ?? ""}${r.stdout}${r.stderr}`);
    assert.match(String(r.stdout), new RegExp(`^the guard went on through ${ROUNDS} rounds$`, "m"), `the guard did not go on after its rounds: ${r.stdout}${r.stderr}`);
    assert.equal(r.status, 0, `the guard's shell failed: ${r.error ?? ""}${r.stderr}`);
    assert.doesNotMatch(String(r.stdout), /^sudo /m, "the guard stops through sudo a process that is not the sandbox's");
    const uid = spawnSync(bashPath(), [".github/scripts/test-sandbox.sh", "uid"], { cwd: ROOT, encoding: "utf8", timeout: 30000 }).stdout.trim();
    assert.match(uid, /^\d+$/, "test-sandbox.sh names no user for the tests");
    const s = spawnSync(bashPath(), ["-eo", "pipefail", ...guard(uid)], { cwd: ROOT, encoding: "utf8", timeout: 30000 });
    assert.equal((String(s.stdout).match(/^sudo -n kill -9 999999$/gm) ?? []).length, ROUNDS, `the guard does not stop a process of the sandbox's user through sudo: ${s.error ?? ""}${s.stdout}${s.stderr}`);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("the secret steps' shell drops what an earlier step could leave them, and the runner's own shell does not", () => {
  // The two secret steps as the benchmark's workflow holds them, which the policy holds to its
  // HELD_OUT_STEP and BROKER_STEP, and the shell they share.
  const benchSteps = parseYaml(read(".github/workflows/benchmark.yml")).jobs.bench.steps;
  const HELD_OUT_STEP = benchSteps.find((s) => s.name === "Check out the held-out set");
  const BROKER_STEP = benchSteps.find((s) => s.name === "The broker, the one process that holds the API key");
  assert.ok(HELD_OUT_STEP && BROKER_STEP, "benchmark.yml has no held-out step or no broker step");
  const SECRET_SHELL = HELD_OUT_STEP.shell;
  assert.equal(BROKER_STEP.shell, SECRET_SHELL, "the secret steps run in two shells");
  // SECRET_SHELL, run for real on both secret steps, with what an earlier step could have left
  // through $GITHUB_ENV: BASH_ENV and ENV naming a file that prints the secret, SHELLOPTS set to
  // trace, exported functions in place of printf and ulimit that print it, and env, mktemp, mkdir
  // and bash first on PATH as shims. Each step runs none of them, traces nothing and prints no
  // secret, and reaches its script, which stops before it fetches or starts anything: the held-out
  // step at the commit, the broker's at its directory, there already. The same step in the runner's
  // own shell, no bash of it privileged, prints the secret, so that the run would see each attack
  // (the re-check of c55cf8f; the final pass on 89c7691, whose exported printf printed the token and
  // the key). On Windows, env and bash are Git's, beside the git the policy runs, which read /usr/bin
  // as Git's own: the steps' text runs unchanged.
  {
    const usrBin = (name) => (process.platform === "win32" ? path.resolve(git(["--exec-path"]).trim(), `../../../usr/bin/${name}.exe`) : `/usr/bin/${name}`);
    const posix = (p) => p.replace(/\\/g, "/");
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "secret-shell-"));
    try {
      const secret = "secret-0123456789abcdef";
      const shims = path.join(dir, "shims");
      const log = path.join(dir, "shims.log");
      fs.mkdirSync(shims);
      for (const name of ["env", "mktemp", "mkdir", "bash"]) fs.writeFileSync(path.join(shims, name), `#!/bin/sh\necho "${name} from PATH" >> "${posix(log)}"\nexit 1\n`, { mode: 0o755 });
      const startup = path.join(dir, "startup.sh");
      fs.writeFileSync(startup, 'echo "a startup file ran with ${HELD_OUT_TOKEN:-}${ANTHROPIC_API_KEY:-}"\n');
      fs.symlinkSync(path.join(ROOT, "benchmark"), path.join(dir, "benchmark"), "junction");
      fs.mkdirSync(path.join(dir, "temp", "broker"), { recursive: true });
      const attacks = [
        ["a startup file", { BASH_ENV: posix(startup), ENV: posix(startup) }],
        // Each writes to the log, past the pipe printf's output goes into.
        ["exported functions", { "BASH_FUNC_printf%%": '() { echo "an exported function ran with $*" >&2; }', "BASH_FUNC_ulimit%%": '() { echo "an exported function ran with ${ANTHROPIC_API_KEY:-}" >&2; }' }],
        ["a trace", { SHELLOPTS: "xtrace" }],
      ];
      for (const [what, s, secrets, reached] of [
        ["the held-out step", HELD_OUT_STEP, { HELD_OUT_TOKEN: secret, HELD_OUT_COMMIT: "abc" }, /no full commit id/],
        ["the broker's step", BROKER_STEP, { ANTHROPIC_API_KEY: secret, BUDGET: "1", RUNNER_TEMP: posix(path.join(dir, "temp")), GITHUB_WORKSPACE: posix(ROOT), RUNNER_TOOL_CACHE: posix(path.join(dir, "tools")) }, /File exists/],
      ]) {
        const step = path.join(dir, "step.sh");
        for (const [attack, vars] of attacks) {
          fs.writeFileSync(step, s.run);
          fs.rmSync(log, { force: true });
          const env = { PATH: `${shims}${path.delimiter}${process.env.PATH ?? ""}`, HOME: posix(dir), ...vars, ...secrets };
          const [command, ...args] = s.shell.replace("{0}", posix(step)).split(" ");
          assert.equal(command, "/usr/bin/env");
          const r = spawnSync(usrBin("env"), args, { cwd: dir, env, encoding: "utf8", timeout: 30000 });
          const printed = `${r.stdout}${r.stderr}`;
          assert.ok(!fs.existsSync(log), `${what}, with ${attack}, ran a program of its PATH: ${fs.existsSync(log) ? fs.readFileSync(log, "utf8") : ""}`);
          assert.ok(!printed.includes(secret) && !printed.includes("ran with") && !/^\+/m.test(printed), `${what}, with ${attack}, ran it, traced, or printed its secret: ${printed}`);
          assert.equal(r.status, 1, printed);
          assert.match(printed, reached, `${what}, with ${attack}, did not reach its script: ${printed}`);
          // In the runner's own shell, no bash of it privileged, the same step prints the secret.
          fs.writeFileSync(step, s.run.replaceAll(" -p ", " "));
          const plain = spawnSync(usrBin("bash"), ["--noprofile", "--norc", "-eo", "pipefail", posix(step)], { cwd: dir, env, encoding: "utf8", timeout: 30000 });
          assert.ok(`${plain.stdout}${plain.stderr}`.includes(secret), `the simulation shows nothing of ${what} with ${attack}: the runner's shell printed no secret: ${plain.stdout}${plain.stderr}`);
        }
      }
      assert.ok(!fs.existsSync(path.join(dir, "held-out-set")), "the held-out step made held-out-set for a commit it refused");
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  }
});

check("node refuses to generate code from strings under the flag the secret jobs run it with", () => {
  for (const code of ["Function('return 1')()", "(() => 0)['constr' + 'uctor']('return 1')()", "eval('1')", "(async () => {}).constructor('return 1')()"]) {
    const r = spawnSync(process.execPath, ["--disallow-code-generation-from-strings", "-e", code], { encoding: "utf8" });
    assert.notEqual(r.status, 0, `node ran ${code} under --disallow-code-generation-from-strings`);
    assert.match(r.stderr, /Code generation from strings disallowed/, code);
  }
});

check("timeout, in bash, stops a command at its limit and passes one within it", () => {
  // The form the rule pins works where the steps run it, in bash: GNU timeout stops the command at
  // its limit (exit 124) and passes a command within it, where a timeout that only waits, as
  // Windows' own is, would do neither.
  // The limit is the one the workflows' own lines name, which the policy holds to its constant.
  const limits = new Set(workflowText().flatMap((t) => [...t.matchAll(/\btimeout (\S+) cargo test\b/g)].map((m) => m[1])));
  assert.equal(limits.size, 1, `the workflows run ignored tests under more than one limit, or none: ${[...limits].join(", ")}`);
  const [IGNORED_TEST_LIMIT] = limits;
  const timeout = (...args) => spawnSync(bashPath(), [".github/scripts/fixtures/timeout.sh", ...args], { cwd: ROOT, encoding: "utf8", timeout: 30000 }).status;
  assert.equal(timeout(IGNORED_TEST_LIMIT, "true"), 0, `timeout ${IGNORED_TEST_LIMIT} does not pass a command within its limit`);
  assert.equal(timeout("1s", "sleep", "10"), 124, "timeout does not stop a command at its limit");
});

check("the watch CLI decides on a real history, through any path, and fails rather than say nothing", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mente-watch-"));
  try {
    const sh = (...args) =>
      execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", ...args], { cwd: dir, encoding: "utf8" }).trim();
    const write = (file, text) => {
      fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true });
      fs.writeFileSync(path.join(dir, file), text);
    };
    const commit = (files) => {
      for (const [f, t] of Object.entries(files)) write(f, t);
      sh("add", "-A");
      sh("commit", "-q", "-m", "change");
      return sh("rev-parse", "HEAD");
    };
    sh("init", "-q");
    const base = commit({ [PATHS_FILE]: "[x.yml]\nsrc/**\n", "src/a.rs": "a\n", "docs/a.md": "a\n" });
    const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("GITHUB_")));
    const cli = (script, head, { output = true } = {}) => {
      const event = path.join(dir, "event.json");
      fs.writeFileSync(event, JSON.stringify({ pull_request: { base: { sha: base }, head: { sha: head } } }));
      const out = path.join(dir, `out-${Math.random().toString(36).slice(2)}.txt`);
      const r = spawnSync(process.execPath, [script, "x.yml"], {
        cwd: dir,
        encoding: "utf8",
        env: { ...env, GITHUB_EVENT_NAME: "pull_request", GITHUB_EVENT_PATH: event, ...(output ? { GITHUB_OUTPUT: out } : {}) },
      });
      return { status: r.status, output: fs.existsSync(out) ? fs.readFileSync(out, "utf8") : "", stderr: r.stderr };
    };
    const script = path.join(HERE, "watch.mjs");
    const docs = commit({ "docs/a.md": "b\n" });
    assert.deepEqual(cli(script, docs), { status: 0, output: "run=false\n", stderr: "" });
    const src = commit({ "src/a.rs": "b\n" });
    assert.equal(cli(script, src).output, "run=true\n");
    const cargo = commit({ ".cargo/config.toml": "[build]\n" });
    assert.equal(cli(script, cargo).output, "run=true\n");
    const silent = cli(script, docs, { output: false });
    assert.equal(silent.status, 1);
    assert.match(silent.stderr, /GITHUB_OUTPUT/);
    // Started through a junction, as a checkout under another path would be: main still runs.
    const link = path.join(dir, "linked");
    fs.symlinkSync(HERE, link, "junction");
    assert.equal(cli(path.join(link, "watch.mjs"), src).output, "run=true\n");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("the Commits check fails on every event while a base change is newer than the head's push", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mente-commits-"));
  // A stand-in for the GitHub API at GITHUB_API_URL, as the runner sets it: the pull request, its
  // issue events and timeline, and its head commits' workflow runs.
  const api = { events: [], timeline: [], runs: new Map(), head: null };
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://localhost");
    let body;
    if (url.pathname === "/repos/o/r/issues/7/events") body = api.events;
    else if (url.pathname === "/repos/o/r/issues/7/timeline") body = api.timeline;
    else if (url.pathname === "/repos/o/r/pulls/7") body = { number: 7, head: { sha: api.head } };
    else if (url.pathname === "/repos/o/r/actions/runs") body = { workflow_runs: api.runs.get(url.searchParams.get("head_sha")) ?? [] };
    res.writeHead(body ? 200 : 404, { "content-type": "application/json" });
    res.end(JSON.stringify(body ?? {}));
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  try {
    const sh = (...args) =>
      execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", ...args], { cwd: dir, encoding: "utf8" }).trim();
    sh("init", "-q");
    fs.writeFileSync(path.join(dir, "a"), "a\n");
    sh("add", "-A");
    sh("commit", "-q", "-m", "Base");
    const base = sh("rev-parse", "HEAD");
    const push = (text) => {
      fs.writeFileSync(path.join(dir, "a"), text);
      sh("commit", "-q", "-am", "Change a");
      return sh("rev-parse", "HEAD");
    };
    const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("GITHUB_")));
    const spawnWith = (eventName, payload) =>
      new Promise((done) => {
        const event = path.join(dir, "event.json");
        fs.writeFileSync(event, JSON.stringify(payload));
        const child = spawn(process.execPath, [path.join(HERE, "commits.mjs")], {
          cwd: dir,
          env: {
            ...env,
            GITHUB_EVENT_NAME: eventName,
            GITHUB_EVENT_PATH: event,
            GITHUB_REPOSITORY: "o/r",
            GITHUB_API_URL: `http://127.0.0.1:${server.address().port}`,
            GITHUB_TOKEN: "t",
            PR_TITLE: "Title",
            PR_BODY: "Body",
          },
        });
        let stdout = "";
        child.stdout.on("data", (d) => (stdout += d));
        child.on("close", (status) => done({ status, stdout }));
      });
    const run = (head, action, changes) =>
      spawnWith("pull_request", { action, changes, pull_request: { number: 7, base: { sha: base }, head: { sha: head } } });
    const change = (at) => ({ event: "base_ref_changed", created_at: at });
    const first = push("b\n");
    api.runs.set(first, [{ created_at: "2026-11-12T10:00:00Z" }, { created_at: "2026-11-12T12:00:00Z" }]);
    assert.equal((await run(first, "synchronize")).status, 0);
    api.events = [change("2026-11-12T09:00:00Z")];
    assert.equal((await run(first, "edited", { body: { from: "Old" } })).status, 0, "a base change before the push is answered");
    // The retarget's own run fails from its event, though the API does not show the change yet.
    const retargeted = await run(first, "edited", { base: { ref: { from: "feature" } } });
    assert.equal(retargeted.status, 1);
    assert.match(retargeted.stdout, /base changed: push a commit to re-run the instruments/);
    // Once the issue events show it, every later event fails too: an edit, a re-run of the push.
    api.events.push(change("2026-11-12T11:00:00Z"));
    assert.equal((await run(first, "edited", { title: { from: "Old" } })).status, 1);
    assert.equal((await run(first, "synchronize")).status, 1);
    // The timeline alone shows it as well.
    api.events = [];
    api.timeline = [change("2026-11-12T11:00:00Z")];
    assert.equal((await run(first, "synchronize")).status, 1);
    // A new commit answers it.
    const second = push("c\n");
    api.runs.set(second, [{ created_at: "2026-11-12T13:00:00Z" }]);
    assert.equal((await run(second, "synchronize")).status, 0);
    // In the merge queue, the pull request's head comes from the API: the group's branch names the
    // base, and the group's commit merges the head into it.
    api.head = second;
    const group = (sha) => ({ merge_group: { base_sha: base, head_sha: second, head_ref: `refs/heads/gh-readonly-queue/main/pr-7-${sha}` } });
    assert.equal((await spawnWith("merge_group", group(base))).status, 0);
    api.head = first;
    assert.equal((await spawnWith("merge_group", group(base))).status, 1, "the queued head's push is older than the base change");
    // Without the head's runs it fails rather than pass unseen.
    api.runs.delete(second);
    assert.equal((await run(second, "synchronize")).status, 1);
  } finally {
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
