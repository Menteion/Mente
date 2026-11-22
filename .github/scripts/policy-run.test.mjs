// The workflow policy's run tests (policy-runtime-judge in docs/design-review.md, its PR 2): every
// check that runs a script or a program of the tree, which the policy itself
// (.github/scripts/policy/) no longer does. They test the change's own scripts in the
// change's own run, as the crates' tests do; the code owner's review and the pins of
// the policy hold this file, the fixtures it runs and the scripts they run in turn. Each check
// counts itself when it runs, and the suite fails when fewer ran than FLOOR, the checks below:
// test_the_documentation_job_runs_the_workflow_policy keeps FLOOR equal to them.
import { after, test } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
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

const FLOOR = 22;
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
/// A path as git's sh and git's configuration take it, with forward slashes on Windows too.
const posixPath = (p) => p.replace(/\\/g, "/");
/// `env` without a variable of git's own, any of which could name another repository (GIT_DIR),
/// more configuration (GIT_CONFIG_PARAMETERS, GIT_CONFIG_COUNT) or another identity.
const withoutGitVariables = (env) => Object.fromEntries(Object.entries(env).filter(([name]) => !name.toUpperCase().startsWith("GIT_")));
/// The identity of every commit the run tests make in their own repositories, fixed here: never the
/// identity of whoever runs them (the review of 100e1e5).
const TEST_IDENTITY = { name: "t", email: "t@example.invalid" };
/// git's environment in the run tests' repositories, and the CLIs' the tests run there, from
/// `env`: no variable of git's own from it, no system or global configuration, and TEST_IDENTITY as
/// author and committer, given as an environment rather than `git -c`. /dev/null names no
/// configuration on Linux and to Git for Windows alike, which refuses node's os.devNull.
const gitTestEnv = (env) => ({
  ...withoutGitVariables(env),
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_CONFIG_GLOBAL: "/dev/null",
  GIT_AUTHOR_NAME: TEST_IDENTITY.name,
  GIT_AUTHOR_EMAIL: TEST_IDENTITY.email,
  GIT_COMMITTER_NAME: TEST_IDENTITY.name,
  GIT_COMMITTER_EMAIL: TEST_IDENTITY.email,
});
const GIT_TEST_ENV = gitTestEnv(process.env);
/// git in the run tests' repository at `dir`, its output trimmed.
const testGit =
  (dir, env = GIT_TEST_ENV) =>
  (...args) =>
    execFileSync("git", args, { cwd: dir, encoding: "utf8", env }).trim();
/// That every commit of the repository `sh` runs git in is TEST_IDENTITY's, as author and committer.
function assertTestIdentity(sh) {
  const who = `${TEST_IDENTITY.name} <${TEST_IDENTITY.email}>`;
  for (const line of sh("log", "--format=%an <%ae>|%cn <%ce>").split("\n")) assert.equal(line, `${who}|${who}`, "a commit of the run tests' repository is not the fixture's identity's");
}

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
    // `ran` reports the share so far, whatever it is, and its minutes; `bound`, at the job's end,
    // fails only past both a tenth of the job's time without the sandbox and two minutes (Norman,
    // 2026-11-21: a short job whose share the runner's speed moves by points fails only once the
    // sandbox has cost it minutes), and on a record that is missing or not whole.
    costs(9_500_000);
    const so = run(["ran"], "Linux");
    assert.equal(so.status, 0, `ran holds the job's share before its end: ${so.out}`);
    assert.match(so.out, /The sandbox has added an estimated 9\.5 s to the 10\d\.\d s since setup: 10\.\d% of the time without it, and 0\.1 minutes, against a bound for the whole job of more than 10% and more than 2 minutes together\./);
    // A job whose sandbox added `share` of its time without it, and `seconds`: the time since setup
    // is set so (added / (since - added)); the script's own start, a few milliseconds, lowers the
    // share by thousandths of a point.
    const job = (share, seconds) => {
      const added = Math.round(seconds * 1_000_000);
      costs(added, Date.now() * 1000 - Math.round(added + added / share));
      return run(["bound"], "Linux");
    };
    const short = job(0.11, 60);
    assert.equal(short.status, 0, `a sandbox that added 11% and 1 minute to the job fails: ${short.out}`);
    assert.match(short.out, /: 1[01]\.\d% of the time without it, and 1\.0 minutes, /, "the bound's line does not state the share and the minutes");
    const over = job(0.11, 180);
    assert.equal(over.status, 1, `a sandbox that added 11% and 3 minutes to the job passes: ${over.out}`);
    assert.match(over.out, /::error::the sandbox has added more than a tenth and more than two minutes to this job's time/);
    assert.equal(job(0.09, 300).status, 0, "a sandbox that added 9% and 5 minutes to the job fails");
    // Two minutes is no more than two minutes; a tenth of a second more is.
    assert.equal(job(0.11, 120).status, 0, "a sandbox that added 11% and exactly 2 minutes to the job fails");
    assert.equal(job(0.11, 120.1).status, 1, "a sandbox that added 11% and 2 minutes 0.1 s to the job passes");
    // Close to the tenth, past two minutes: 10.05% fails, which a share rounded down to per mille
    // would let through (the review of 41a54b6), and 9.95% passes.
    assert.equal(job(0.1005, 180).status, 1, "a sandbox that added 10.05% and 3 minutes to the job passes");
    assert.equal(job(0.0995, 180).status, 0, "a sandbox that added 9.95% and 3 minutes to the job fails");
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
    // The shell route's wrapper refuses what it could not hand over whole, as the program's does
    // (the canary runs both on a runner).
    assert.equal(run(["reference", "A=b"], "Linux").status, 2, "a command that env would read as a variable is handed over");
    assert.equal(run(["reference"], "Linux").status, 2);
    assert.equal(run(["run"], "Linux").status, 2, "the script still takes a command to run, which the program now does");
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
    const sh = testGit(dir);
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
    sh("init", "-q", "-b", "main");
    const base = commit({ [PATHS_FILE]: "[x.yml]\nsrc/**\n", "src/a.rs": "a\n", "docs/a.md": "a\n" });
    const env = Object.fromEntries(Object.entries(GIT_TEST_ENV).filter(([k]) => !k.startsWith("GITHUB_")));
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
    assertTestIdentity(sh);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("the Commits check fails on every event while a base change is newer than the head's push", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mente-commits-"));
  // A stand-in for the GitHub API at GITHUB_API_URL, as the runner sets it: the pull request, with
  // its text, author and commits, its issue events and timeline, and its head commits' workflow runs.
  const api = { events: [], timeline: [], runs: new Map(), head: null, body: "", author: "someone", commits: [] };
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://localhost");
    let body;
    if (url.pathname === "/repos/o/r/issues/7/events") body = api.events;
    else if (url.pathname === "/repos/o/r/issues/7/timeline") body = api.timeline;
    else if (url.pathname === "/repos/o/r/pulls/7") body = { number: 7, head: { sha: api.head }, title: "Title", body: api.body, user: { login: api.author } };
    else if (url.pathname === "/repos/o/r/pulls/7/commits") body = api.commits;
    else if (url.pathname === "/repos/o/r/actions/runs") body = { workflow_runs: api.runs.get(url.searchParams.get("head_sha")) ?? [] };
    res.writeHead(body ? 200 : 404, { "content-type": "application/json" });
    res.end(JSON.stringify(body ?? {}));
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  try {
    const sh = testGit(dir);
    sh("init", "-q", "-b", "main");
    fs.writeFileSync(path.join(dir, "a"), "a\n");
    sh("add", "-A");
    sh("commit", "-q", "-m", "Base");
    const base = sh("rev-parse", "HEAD");
    const push = (text) => {
      fs.writeFileSync(path.join(dir, "a"), text);
      sh("commit", "-q", "-am", "Change a");
      return sh("rev-parse", "HEAD");
    };
    const env = Object.fromEntries(Object.entries(GIT_TEST_ENV).filter(([k]) => !k.startsWith("GITHUB_")));
    // Every pull request's body declares what it is (root-cause-first).
    const DECLARED = "Not a fix: a fixture of the Commits check's run test.";
    const spawnWith = (eventName, payload, body = DECLARED) =>
      new Promise((done) => {
        api.body = body;
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
    // A body with no declaration fails, read from the API as it stands, not from the event's copy,
    // which a re-run of an old run would hold (the review of 5264a01).
    api.runs.set(first, [{ created_at: "2026-11-12T10:00:00Z" }]);
    const edited = { action: "edited", pull_request: { number: 7, base: { sha: base }, head: { sha: first }, title: "Title", body: DECLARED, user: { login: "someone" } } };
    const undeclared = await spawnWith("pull_request", edited, "Body");
    assert.equal(undeclared.status, 1, "the event's copy of the body was read, not the body as it stands");
    assert.match(undeclared.stdout, /::error::the pull request's body: it declares neither/);
    // Dependabot's declares itself by its author, the API's, while every commit is its own.
    api.author = "dependabot[bot]";
    api.commits = [{ sha: "a".repeat(40), author: { login: "dependabot[bot]" } }];
    assert.equal((await spawnWith("pull_request", edited, "Body")).status, 0, "Dependabot's body fails");
    api.commits.push({ sha: "b".repeat(40), author: { login: "someone" } });
    const pushedOnto = await spawnWith("pull_request", edited, "Body");
    assert.equal(pushedOnto.status, 1, "Dependabot's pull request declared itself with another's commit on it");
    assert.match(pushedOnto.stdout, /::error::the pull request's author, dependabot\[bot\], declares it only while every commit is its own, and bbbbbbb is another's/);
    assert.match(pushedOnto.stdout, /::error::the pull request's body: it declares neither/);
    api.author = "someone";
    api.commits = [];
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
    // The queue's run checks the body too, from the API, so that no pull request reaches main with
    // none declared.
    const queued = await spawnWith("merge_group", group(base), "Body");
    assert.equal(queued.status, 1, "the merge queue's run passed a body that declares nothing");
    assert.match(queued.stdout, /::error::the pull request's body: it declares neither/);
    api.head = first;
    assert.equal((await spawnWith("merge_group", group(base))).status, 1, "the queued head's push is older than the base change");
    // Without the head's runs it fails rather than pass unseen.
    api.runs.delete(second);
    assert.equal((await run(second, "synchronize")).status, 1);
    assertTestIdentity(sh);
  } finally {
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("the run tests' repositories commit as the fixture's identity, under no configuration of the caller's", () => {
  // A caller whose git names another identity and either runs a hook on every commit or points at
  // another repository, through each place git reads it from: the global configuration, the home
  // directory's .gitconfig, and the variables of git's own. Plain git takes each, as the controls
  // show; the tests' git takes none (the review of 100e1e5).
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mente-test-git-"));
  try {
    const marker = path.join(dir, "hook-ran");
    const hooks = path.join(dir, "hooks");
    fs.mkdirSync(hooks);
    fs.writeFileSync(path.join(hooks, "pre-commit"), `#!/bin/sh\necho ran >> '${posixPath(marker)}'\n`, { mode: 0o755 });
    const config = `[user]\n\tname = Someone\n\temail = someone@example.com\n[core]\n\thooksPath = ${posixPath(hooks)}\n`;
    fs.writeFileSync(path.join(dir, "global.gitconfig"), config);
    fs.mkdirSync(path.join(dir, "home"));
    fs.writeFileSync(path.join(dir, "home", ".gitconfig"), config);
    const other = path.join(dir, "other");
    fs.mkdirSync(other);
    testGit(other)("init", "-q", "-b", "main");
    const plain = { ...withoutGitVariables(process.env), GIT_CONFIG_NOSYSTEM: "1" };
    const someone = { GIT_AUTHOR_NAME: "Someone", GIT_AUTHOR_EMAIL: "someone@example.com", GIT_COMMITTER_NAME: "Someone", GIT_COMMITTER_EMAIL: "someone@example.com" };
    const callers = {
      "a global configuration": { ...plain, GIT_CONFIG_GLOBAL: posixPath(path.join(dir, "global.gitconfig")) },
      "a home directory's .gitconfig": { ...plain, HOME: posixPath(path.join(dir, "home")) },
      "git's variables": { ...plain, GIT_CONFIG_GLOBAL: "/dev/null", ...someone, GIT_CONFIG_COUNT: "1", GIT_CONFIG_KEY_0: "core.hooksPath", GIT_CONFIG_VALUE_0: posixPath(hooks) },
      "a GIT_DIR of another repository": { ...plain, GIT_CONFIG_GLOBAL: "/dev/null", ...someone, GIT_DIR: path.join(other, ".git") },
    };
    const who = `${TEST_IDENTITY.name} <${TEST_IDENTITY.email}>`;
    for (const [what, caller] of Object.entries(callers)) {
      const repo = fs.mkdtempSync(path.join(dir, "repo-"));
      const commit = (sh, text) => {
        fs.writeFileSync(path.join(repo, "a"), text);
        sh("add", "-A");
        sh("commit", "-q", "-m", text.trim());
      };
      testGit(repo)("init", "-q", "-b", "main");
      // The control: plain git, under the caller's environment, takes what it names.
      fs.rmSync(marker, { force: true });
      const plainGit = (...args) => execFileSync("git", args, { cwd: repo, encoding: "utf8", env: caller }).trim();
      commit(plainGit, "control\n");
      assert.equal(plainGit("log", "-1", "--format=%an <%ae>|%s"), "Someone <someone@example.com>|control", `${what}: plain git does not take the caller's identity`);
      if (caller.GIT_DIR) {
        assert.equal(testGit(other)("log", "--format=%s"), "control", `${what}: plain git does not commit to the repository GIT_DIR names`);
      } else {
        assert.ok(fs.existsSync(marker), `${what}: plain git runs no hook the caller names`);
      }
      // The tests' git, from the same environment, takes none of it.
      fs.rmSync(marker, { force: true });
      const sh = testGit(repo, gitTestEnv(caller));
      commit(sh, "tested\n");
      assert.ok(!fs.existsSync(marker), `${what}: the tests' git runs a hook the caller names`);
      assert.equal(sh("log", "-1", "--format=%an <%ae>|%cn <%ce>|%s"), `${who}|${who}|tested`, `${what}: the tests' git commits elsewhere, or as another identity`);
      if (caller.GIT_DIR) assert.equal(testGit(other)("log", "--format=%s"), "control", `${what}: the tests' git commits to the repository GIT_DIR names`);
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("the collector reads the repository it is in, and runs no program git's configuration names", () => {
  // The collector, copied into a repository of its own, under a caller whose git names an
  // fsmonitor in the repository's configuration, sorts tags backwards from the home directory's
  // .gitconfig and from git's variables, and points GIT_DIR at another repository. Plain git takes
  // each, as the controls show; the collector's facts are the repository's own, and the fsmonitor
  // never runs (the review of 100e1e5). The system configuration lies outside what a test may
  // write, so its refusal is held by GIT_CONFIG_NOSYSTEM's reading alone.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mente-collect-"));
  try {
    const repo = path.join(dir, "repo");
    fs.mkdirSync(path.join(repo, ".github", "scripts"), { recursive: true });
    fs.copyFileSync(path.join(HERE, "policy-collect.mjs"), path.join(repo, ".github", "scripts", "policy-collect.mjs"));
    fs.writeFileSync(path.join(repo, "ROADMAP.md"), "# Roadmap\n");
    const sh = testGit(repo);
    sh("init", "-q", "-b", "main");
    sh("add", "-A");
    sh("commit", "-q", "-m", "Base");
    sh("tag", "m0a");
    sh("tag", "m0b");
    const head = sh("rev-parse", "HEAD");
    const other = path.join(dir, "other");
    fs.mkdirSync(other);
    const osh = testGit(other);
    osh("init", "-q", "-b", "main");
    fs.writeFileSync(path.join(other, "ROADMAP.md"), "# Another\n");
    osh("add", "-A");
    osh("commit", "-q", "-m", "Other");
    const marker = path.join(dir, "fsmonitor-ran");
    const fsmonitor = path.join(dir, "fsmonitor.sh");
    fs.writeFileSync(fsmonitor, `#!/bin/sh\necho ran >> '${posixPath(marker)}'\n`, { mode: 0o755 });
    sh("config", "core.fsmonitor", posixPath(fsmonitor));
    fs.mkdirSync(path.join(dir, "home"));
    fs.writeFileSync(path.join(dir, "home", ".gitconfig"), "[tag]\n\tsort = -refname\n");
    const base = Object.fromEntries(Object.entries(GIT_TEST_ENV).filter(([k]) => !k.startsWith("GITHUB_")));
    const { GIT_CONFIG_GLOBAL: _, ...noGlobal } = base;
    const plain = (env, ...args) => execFileSync("git", args, { cwd: repo, encoding: "utf8", env, stdio: ["ignore", "pipe", "ignore"] }).trim();
    // The controls: plain git takes each.
    plain(base, "ls-files", "-z");
    assert.ok(fs.existsSync(marker), "plain git ls-files runs no fsmonitor of the repository's configuration");
    fs.rmSync(marker);
    assert.equal(plain({ ...noGlobal, HOME: posixPath(path.join(dir, "home")) }, "-c", "core.fsmonitor=false", "tag", "-l"), "m0b\nm0a", "plain git does not read the home directory's .gitconfig");
    const sorted = { GIT_CONFIG_COUNT: "1", GIT_CONFIG_KEY_0: "tag.sort", GIT_CONFIG_VALUE_0: "-refname" };
    assert.equal(plain({ ...base, ...sorted }, "-c", "core.fsmonitor=false", "tag", "-l"), "m0b\nm0a", "plain git does not read git's variables");
    assert.equal(plain({ ...base, GIT_DIR: path.join(other, ".git") }, "rev-parse", "HEAD"), osh("rev-parse", "HEAD"), "plain git does not read GIT_DIR's repository");
    // The collector, under all of it.
    const r = spawnSync(process.execPath, [path.join(repo, ".github", "scripts", "policy-collect.mjs")], {
      cwd: repo,
      encoding: "utf8",
      env: { ...noGlobal, HOME: posixPath(path.join(dir, "home")), ...sorted, GIT_DIR: path.join(other, ".git") },
    });
    assert.equal(r.status, 0, r.stderr);
    assert.ok(!fs.existsSync(marker), "the collector's git runs the fsmonitor of the repository's configuration");
    const facts = JSON.parse(fs.readFileSync(path.join(repo, "target", "policy-facts.json"), "utf8"));
    assert.deepEqual(facts, { head, tracked: [".github/scripts/policy-collect.mjs", "ROADMAP.md"], base: head, tags: ["m0a", "m0b"], baseRoadmap: "# Roadmap\n" });
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("the site is published only from a milestone tag on a commit of main", () => {
  // .github/scripts/commit-on-main.sh (the review of 7ba7a5c): a milestone tag on a pull request's
  // head, which main never took, is refused, and so is one on a commit inside a pull request main
  // merged, which never stood as main itself (the re-review of 40bb29a); a tag on main's head, on an
  // older commit of main or on the merge, lightweight or annotated, passes. And .github/scripts/publish-tag.sh, which pages-publish.yml
  // runs before anything of the tag (Norman, 2026-11-17): it checks out a tag on main, and refuses a
  // tag outside main, a name of another shape and none, leaving the checkout where it was.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mente-on-main-"));
  try {
    const sh = testGit(dir);
    sh("init", "-q", "-b", "main");
    const commit = (text) => {
      fs.writeFileSync(path.join(dir, "a"), text);
      sh("add", "-A");
      sh("commit", "-q", "-m", text.trim());
      return sh("rev-parse", "HEAD");
    };
    const older = commit("one\n");
    const head = commit("two\n");
    sh("checkout", "-q", "-b", "pull-request");
    const unmerged = commit("three\n");
    sh("checkout", "-q", "main");
    // A pull request main merged, as the merge queue merges one: a merge commit whose first parent is
    // main's head, and whose second is the pull request's own commit.
    sh("checkout", "-q", "-b", "merged", head);
    const inner = commit("inner\n");
    sh("checkout", "-q", "main");
    sh("merge", "-q", "--no-ff", "-m", "Merge pull request #1 from merged", "merged");
    const merge = sh("rev-parse", "HEAD");
    sh("tag", "m2", inner);
    sh("tag", "m3", merge);
    sh("tag", "m1", head);
    sh("tag", "-a", "-m", "m0", "m0", older);
    sh("tag", "m9", unmerged);
    const script = posixPath(path.join(HERE, "commit-on-main.sh"));
    const run = (...args) => spawnSync(bashPath(), [script, ...args], { cwd: dir, encoding: "utf8", env: GIT_TEST_ENV, timeout: 30000 });
    // Without origin/main there is nothing to check against: refused, never passed.
    const none = run("m1");
    assert.equal(none.status, 1, `${none.stdout}${none.stderr}`);
    assert.match(none.stdout, /no origin\/main/);
    sh("update-ref", "refs/remotes/origin/main", merge);
    for (const tag of ["m1", "m0", "m3", head, older, merge]) {
      const r = run(tag);
      assert.equal(r.status, 0, `${tag}: ${r.stdout}${r.stderr}`);
    }
    for (const tag of ["m2", inner, "m9", unmerged]) {
      const r = run(tag);
      assert.equal(r.status, 1, `${tag}: ${r.stdout}${r.stderr}`);
      assert.match(r.stdout, /^::error::.* is no commit of main's own history/m, tag);
    }
    // A name of no commit, and no argument at all.
    assert.equal(run("m404").status, 1);
    assert.equal(run().status, 2);
    assert.equal(run("").status, 2);
    // publish-tag.sh, run as the workflow runs it: from the repository's root, with the scripts at
    // their paths there, and the tag in TAG alone.
    fs.mkdirSync(path.join(dir, ".github", "scripts"), { recursive: true });
    for (const s of ["commit-on-main.sh", "publish-tag.sh"]) fs.copyFileSync(path.join(HERE, s), path.join(dir, ".github", "scripts", s));
    const publish = (tag) => {
      const env = { ...GIT_TEST_ENV };
      if (tag !== undefined) env.TAG = tag;
      return spawnSync(bashPath(), [".github/scripts/publish-tag.sh"], { cwd: dir, encoding: "utf8", env, timeout: 30000 });
    };
    for (const [tag, at] of [
      ["m1", head],
      ["m0", older],
    ]) {
      const r = publish(tag);
      assert.equal(r.status, 0, `${tag}: ${r.stdout}${r.stderr}`);
      assert.equal(sh("rev-parse", "HEAD"), at, `${tag} is not what is checked out`);
    }
    const before = sh("rev-parse", "HEAD");
    for (const [tag, refusal] of [
      ["m9", /is no commit of main's own history/],
      ["m2", /is no commit of main's own history/],
      ["m404", /names no commit/],
      ["m1; touch pwned", /no milestone's name/],
      ["$(touch pwned)", /no milestone's name/],
      ["main", /no milestone's name/],
      ["refs/tags/m1", /no milestone's name/],
      ["m1\n", /no milestone's name/],
      ["", /no milestone's name/],
      [undefined, /no milestone's name/],
    ]) {
      const r = publish(tag);
      assert.equal(r.status, 1, `${JSON.stringify(tag)}: ${r.stdout}${r.stderr}`);
      assert.match(r.stdout, refusal, JSON.stringify(tag));
      assert.equal(sh("rev-parse", "HEAD"), before, `${JSON.stringify(tag)} moved the checkout`);
    }
    assert.ok(!fs.existsSync(path.join(dir, "pwned")), "a tag's name ran as a command");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("the nightly alarm's CLI keeps one issue through GitHub's API, whatever the details' calls answer", async () => {
  // A stand-in for GitHub's API at GITHUB_API_URL: the repository's issues, a hundred others ahead
  // of the alarm's so that it lies on the second page; the run's jobs, the alarm's own still
  // running; a job's log, behind a redirect to its storage; and the nightlies' latest scheduled
  // runs. `mode` makes the jobs' listing answer 502, or a log stall, as the review of 4ae3c27 had
  // them, and `faults` the next listings of the issues stall or answer 502, one each; `latest` gives
  // each nightly's scheduled runs, newest first, each with its conclusion and its count of jobs.
  const FUZZ = ".github/workflows/fuzz-nightly.yml";
  const MUTANTS = ".github/workflows/mutants-full.yml";
  const fresh = () => ({ issues: Array.from({ length: 100 }, (_, k) => ({ number: k + 1, title: `Another issue ${k + 1}`, body: "", state: "open", user: { login: "github-actions[bot]" } })), comments: [], calls: [], blobAuth: [], next: 101 });
  let state = fresh();
  const mode = { jobs: "ok", logs: "ok", faults: [] };
  const latest = new Map();
  const jobsOf = new Map();
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://localhost");
    const send = (status, body, headers = {}) => {
      res.writeHead(status, { "content-type": "application/json", ...headers });
      res.end(body === undefined ? "" : JSON.stringify(body));
    };
    let chunks = "";
    req.on("data", (c) => (chunks += c));
    req.on("end", () => {
      state.calls.push({ method: req.method, path: url.pathname, query: url.search, auth: req.headers.authorization ?? null });
      const body = chunks ? JSON.parse(chunks) : null;
      const issue = /^\/repos\/o\/r\/issues\/(\d+)(\/comments)?$/.exec(url.pathname);
      const runs = /^\/repos\/o\/r\/actions\/workflows\/([\w.-]+)\/runs$/.exec(url.pathname);
      if (url.pathname === "/blob/1") {
        state.blobAuth.push(req.headers.authorization ?? null);
        res.writeHead(200, { "content-type": "text/plain" });
        res.end("2026-11-21T00:00:00.0Z Fuzzing...\n2026-11-21T00:01:00.0Z ##[error]the interpreter and native code disagree\n");
      } else if (req.method === "GET" && url.pathname === "/repos/o/r/issues" && mode.faults.length > 0) {
        if (mode.faults.shift() === "502") send(502, { message: "Bad Gateway" });
        // else no answer, ever
      } else if (req.method === "GET" && url.pathname === "/repos/o/r/issues") {
        const page = Number(url.searchParams.get("page"));
        const per = Number(url.searchParams.get("per_page"));
        const open = url.searchParams.get("state") === "open" ? state.issues.filter((i) => i.state === "open") : [];
        send(200, open.slice((page - 1) * per, page * per));
      } else if (req.method === "POST" && url.pathname === "/repos/o/r/issues") {
        const i = { number: state.next++, title: body.title, body: body.body, labels: body.labels, assignees: body.assignees, state: "open", user: { login: "github-actions[bot]" } };
        state.issues.push(i);
        send(201, i);
      } else if (issue && issue[2] && req.method === "POST") {
        state.comments.push({ n: Number(issue[1]), body: body.body });
        send(201, { id: state.comments.length });
      } else if (issue && !issue[2] && req.method === "PATCH") {
        state.issues.find((i) => i.number === Number(issue[1])).state = body.state;
        send(200, {});
      } else if (url.pathname === "/repos/o/r/actions/runs/77/jobs") {
        if (mode.jobs === "502") send(502, { message: "Bad Gateway" });
        else send(200, { jobs: [{ id: 1, name: "Fuzz, nightly", status: "completed", conclusion: "failure" }, { id: 2, name: "Nightly alarm (Fuzz)", status: "in_progress", conclusion: null }] });
      } else if (/^\/repos\/o\/r\/actions\/runs\/\d+\/jobs$/.test(url.pathname)) {
        send(200, { total_count: jobsOf.get(Number(url.pathname.split("/")[6])) ?? 1, jobs: [] });
      } else if (url.pathname === "/repos/o/r/actions/jobs/1/logs") {
        if (mode.logs === "stall") return; // no answer, ever
        send(302, undefined, { location: `http://127.0.0.1:${server.address().port}/blob/1` });
      } else if (runs) {
        const file = `.github/workflows/${runs[1]}`;
        const listed = latest.get(file) ?? [{ id: 500, created_at: new Date().toISOString(), conclusion: null }];
        send(200, { workflow_runs: listed.slice(0, Number(url.searchParams.get("per_page"))) });
      } else send(404, { message: "Not Found" });
    });
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  try {
    const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("GITHUB_") && !k.startsWith("ALARM_")));
    const alarm = (results, extra = {}) =>
      new Promise((done) => {
        const child = spawn(process.execPath, [path.join(HERE, "nightly-alarm.mjs")], {
          env: {
            ...env,
            GITHUB_TOKEN: "the-token",
            RESULTS: JSON.stringify(results),
            GITHUB_API_URL: `http://127.0.0.1:${server.address().port}`,
            GITHUB_REPOSITORY: "o/r",
            GITHUB_RUN_ID: "77",
            GITHUB_SERVER_URL: "https://github.example",
            GITHUB_SHA: "c".repeat(40),
            GITHUB_WORKFLOW: "Fuzz, nightly",
            GITHUB_WORKFLOW_REF: `o/r/${FUZZ}@refs/heads/main`,
            ALARM_TIMEOUT_MS: "500",
            ALARM_BACKOFF_MS: "10",
            ...extra,
          },
        });
        let out = "";
        // An alarm that never ends fails here, rather than hold the suite.
        const limit = setTimeout(() => child.kill(), 30000);
        child.stdout.on("data", (d) => (out += d));
        child.stderr.on("data", (d) => (out += d));
        child.on("close", (status) => {
          clearTimeout(limit);
          done({ status, out });
        });
      });
    const failed = { fuzz: { result: "failure", outputs: {} } };
    const alarms = () => state.issues.filter((i) => i.title.startsWith("Nightly: "));
    // A failing night: the issue first, from the run's own results, then the details.
    const first = await alarm(failed);
    assert.equal(first.status, 0, first.out);
    assert.deepEqual(alarms().map((i) => [i.number, i.title, i.labels, i.assignees, i.state]), [[101, "Nightly: Fuzz, nightly failing", ["nightly"], ["Dancraft3746"], "open"]]);
    assert.ok(alarms()[0].body.startsWith(`<!-- nightly-alarm: ${FUZZ} -->\n`), alarms()[0].body);
    assert.match(alarms()[0].body, /https:\/\/github\.example\/o\/r\/actions\/runs\/77 failed/);
    assert.match(alarms()[0].body, /- `fuzz`: failure/);
    const details = state.comments.find((c) => c.n === 101);
    assert.ok(details && /\*\*Fuzz, nightly\*\*: failure/.test(details.body) && /the interpreter and native code disagree/.test(details.body), JSON.stringify(state.comments));
    assert.ok(!state.comments.some((c) => c.body.includes("Nightly alarm (Fuzz)")), "the alarm's own running job is counted failing");
    // Every open issue is read, and which is the alarm's is the CLI's to tell, not the server's.
    assert.ok(state.calls.filter((c) => c.path === "/repos/o/r/issues" && c.method === "GET").every((c) => !c.query.includes("creator")), "the issues are read through a filter of the server's");
    const second = await alarm(failed);
    assert.equal(second.status, 0, second.out);
    assert.equal(alarms().length, 1, "a later failure opened a second issue");
    const green = await alarm({ fuzz: { result: "success", outputs: {} } });
    assert.equal(green.status, 0, green.out);
    assert.equal(alarms()[0].state, "closed");
    assert.equal(state.comments.at(-1).body, `Green again at ${"c".repeat(40)}: https://github.example/o/r/actions/runs/77`);
    assert.equal(state.issues.filter((i) => i.state === "open").length, 100, "the alarm closed another issue");
    assert.ok(state.calls.filter((c) => c.path !== "/blob/1").every((c) => c.auth === "Bearer the-token"), JSON.stringify(state.calls));
    assert.ok(state.blobAuth.length > 0 && state.blobAuth.every((a) => a === null), "the log's storage was handed the token");
    // The jobs' listing answers 502, every try: the issue is opened all the same.
    state = fresh();
    mode.jobs = "502";
    const bad = await alarm(failed);
    assert.equal(bad.status, 0, bad.out);
    assert.equal(alarms().length, 1, "a 502 on the jobs' listing left the night without an issue");
    assert.match(bad.out, /::warning::nightly-alarm: the details of the run could not be added/);
    mode.jobs = "ok";
    // A log that never answers: the alarm ends within its limits, the issue opened and the details
    // added without that log's lines.
    state = fresh();
    mode.logs = "stall";
    const started = Date.now();
    const stalled = await alarm(failed);
    assert.equal(stalled.status, 0, stalled.out);
    assert.ok(Date.now() - started < 20000, `a stalled log held the alarm ${Date.now() - started} ms`);
    assert.equal(alarms().length, 1);
    assert.ok(state.comments.some((c) => c.body.includes("(no error line in its log)")), JSON.stringify(state.comments));
    mode.logs = "ok";
    // The issues' listing stalls once, then answers 502 once: each call is tried again, within its time.
    state = fresh();
    mode.faults = ["stall", "502"];
    const shaky = await alarm(failed);
    assert.equal(shaky.status, 0, shaky.out);
    assert.equal(alarms().length, 1, "a call that stalled or answered 502 once left the night without an issue");
    assert.deepEqual(mode.faults, []);
    // A nightly that has not run: the weekly's latest scheduled run is an hour-old startup_failure,
    // and the one before it started no job, behind which it last ran 200 hours ago; it opens its own
    // issue (the review of d15fe30). The jobs of the startup_failure are not asked for.
    state = fresh();
    const hoursAgo = (h) => new Date(Date.now() - h * 3_600_000).toISOString();
    const lastRan = hoursAgo(200);
    latest.set(MUTANTS, [
      { id: 901, created_at: hoursAgo(1), conclusion: "startup_failure" },
      { id: 902, created_at: hoursAgo(25), conclusion: "failure" },
      { id: 903, created_at: lastRan, conclusion: "success" },
    ]);
    jobsOf.set(902, 0);
    const ran = await alarm({ fuzz: { result: "success", outputs: {} } });
    assert.equal(ran.status, 0, ran.out);
    assert.deepEqual(alarms().map((i) => [i.title, i.body.split("\n")[0]]), [["Nightly: mutants-full.yml has not run", `<!-- nightly-alarm: ${MUTANTS} -->`]]);
    assert.ok(alarms()[0].body.includes(`its last scheduled run that started a job was at ${lastRan}`), alarms()[0].body);
    assert.ok(!state.calls.some((c) => c.path === "/repos/o/r/actions/runs/901/jobs"), "the jobs of a startup_failure were asked for");
    latest.clear();
    jobsOf.clear();
    // Without its variables, or for a workflow it does not know, it fails rather than say nothing.
    const bare = await new Promise((done) => {
      const child = spawn(process.execPath, [path.join(HERE, "nightly-alarm.mjs")], { env });
      child.on("close", (status) => done(status));
    });
    assert.equal(bare, 1);
    assert.equal((await alarm(failed, { GITHUB_WORKFLOW_REF: "o/r/.github/workflows/ci.yml@refs/heads/main" })).status, 1);
  } finally {
    server.close();
  }
});

check("the fuzz summary stays under GitHub's limit for a step's summary, reads bytes as bytes, and says what it cut", () => {
  // .github/scripts/fuzz-summary.mjs, as both fuzz workflows run it: a small failure shown whole; the
  // nightly's minimized input of 2026-11-21, 3 MB on one line, which a cap of 40 lines let through
  // and which aborted the summary; inputs that end where a character would be cut, or hold a NUL, a
  // control byte or a byte of no character (the review of 4ae3c27); and so many failures that the
  // whole must stop early.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mente-fuzz-summary-"));
  try {
    const out = path.join(dir, "fuzz-out");
    fs.mkdirSync(out);
    const put = (name, text, input) => {
      fs.writeFileSync(path.join(out, `${name}.found`), text);
      fs.writeFileSync(path.join(out, `${name}.mn`), input);
    };
    fs.writeFileSync(path.join(out, "summary.txt"), "runs: 1234\nfailures: 306\n");
    // A text file beside the records, sorted first, is no failure: a record is `<name>.found`.
    fs.writeFileSync(path.join(out, "00notes.txt"), "not a record\n");
    put("0small", "a crash of mn check\n", "fn main() -> i32 {\n  0\n}\n");
    const big = 3 * 1024 * 1024;
    put("1big", "the interpreter and native code disagree\n", "x".repeat(big));
    put("2utf8", "a cut inside a character\n", Buffer.concat([Buffer.from("x".repeat(4095)), Buffer.from([0xc3, 0xa9])]));
    put("3nul", "a NUL\n", Buffer.from("ab\0cd\n"));
    put("4bytes", "a byte of no character, and a control byte\n", Buffer.from([0x61, 0xff, 0x62, 0x1b, 0x63, 0x0a]));
    put("5tildes", "tildes\n", "~~~~~~~~\nafter\n");
    // Characters of three bytes each: a cap counted in characters would let this grow three times.
    for (let k = 0; k < 300; k++) put(`f${String(k).padStart(3, "0")}`, `failure ${k}\n`, `${"\u20ac".repeat(33)}\n`.repeat(60));
    const summary = (...args) => spawnSync(process.execPath, [path.join(HERE, "fuzz-summary.mjs"), ...args], { maxBuffer: 16 * 1024 * 1024, timeout: 60000 });
    const r = summary(out, "mn fuzz --seed 1", "0");
    assert.equal(r.status, 0, String(r.stderr));
    const bytes = r.stdout.length;
    const text = r.stdout.toString("utf8");
    assert.ok(bytes < 600 * 1024, `the summary is ${bytes} bytes, past its own cap and one part (GitHub's is 1024k)`);
    assert.ok(text.startsWith("## mn fuzz --seed 1\n~~~~~~~~text\nruns: 1234\nfailures: 306\n~~~~~~~~\n"), text.slice(0, 200));
    assert.ok(!text.includes("00notes.txt") && !text.includes("### summary"), "a file that is no record was shown as a failure");
    assert.ok(text.includes("### 0small.found\n~~~~~~~~text\na crash of mn check\n~~~~~~~~\nminimized input:\n~~~~~~~~text\nfn main() -> i32 {\n  0\n}\n~~~~~~~~\n"), "the small failure is not shown whole, or is said to be cut");
    assert.ok(text.includes(`--- cut here: ${big - 4096} of its ${big} bytes are past 40 lines or 4096 bytes, and in the run's artifact`), "the 3 MB input's cut is not said");
    // A character the cap would split is left out whole, and its two bytes counted.
    assert.ok(text.includes(`${"x".repeat(4095)}\n--- cut here: 2 of its 4097 bytes`), "a character was split at the cut, or its bytes miscounted");
    assert.ok(!r.stdout.includes(Buffer.from([0x78, 0xc3, 0x0a])), "half a character was shown");
    // A NUL, a control byte and a byte of no character, each shown escaped and counted.
    assert.ok(text.includes("minimized input:\n~~~~~~~~text\nab\\x00cd\n~~~~~~~~\n"), "a NUL was dropped, or counted wrong");
    assert.ok(text.includes("minimized input:\n~~~~~~~~text\na\\xffb\\x1bc\n~~~~~~~~\n"), "a byte of no character or a control byte was shown raw");
    // A line of tildes cannot close the fence it stands in.
    assert.ok(text.includes("~~~~~~~~~text\n~~~~~~~~\nafter\n~~~~~~~~~\n"), "a line of the input closed its fence");
    assert.ok(text.includes("--- cut here: 2000 of its 6000 bytes are past 40 lines or 4096 bytes"), "a cut by lines is not said");
    const more = /--- (\d+) more failures are not shown here/.exec(text);
    assert.ok(more && Number(more[1]) > 0 && Number(more[1]) < 300, `the failures left out are not counted: ${more?.[0]}`);
    // Without its three arguments it fails, and without a summary it says so.
    assert.equal(summary().status, 2);
    fs.rmSync(path.join(out, "summary.txt"));
    assert.match(summary(out, "t", "101").stdout.toString("utf8"), /no summary: mn fuzz exited with 101/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("mutants-floor.mjs plan writes each shard's names and exact regexes and the plan's outputs, and assigned holds a shard to its names", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mutants-plan-"));
  try {
    const m = (name) => ({ name, file: "compiler/a/src/lib.rs", function: null, span: { start: { line: 1, column: 1 }, end: { line: 1, column: 2 } } });
    const write = (file, value) => fs.writeFileSync(path.join(dir, file), typeof value === "string" ? value : JSON.stringify(value));
    const floor = (...args) => spawnSync(process.execPath, [path.join(ROOT, ".github/scripts/mutants-floor.mjs"), ...args], { cwd: dir, encoding: "utf8", timeout: 30000 });
    // Five mutants, one of them around a deleted line, at 10770 s each with the build's 30: one
    // shard of three hours each, so five shards.
    write("diff.json", [m("a.rs:1:1: replace f -> bool with true"), m("a.rs:2:1: replace g with ()"), m("a.rs:3:1: delete ! in h"), m("a.rs:4:1: replace i -> u8 with 0")]);
    write("deleted.json", [m("a.rs:3:1: delete ! in h"), m("a.rs:9:1: replace j (x|y) with ()")]);
    const out = floor("plan", "diff.json", "deleted.json", "10770", "plan");
    assert.equal(out.status, 0, out.stderr);
    assert.equal(out.stdout, "reason=sharded\ncount=5\nshards=[0,1,2,3,4]\nneeded=5\n");
    const shard = (k, ext) => fs.readFileSync(path.join(dir, "plan", `shard-${k}.${ext}`), "utf8");
    assert.deepEqual(JSON.parse(shard(4, "json")), ["a.rs:9:1: replace j (x|y) with ()"]);
    assert.equal(shard(4, "re"), "^a\\.rs:9:1: replace j \\(x\\|y\\) with \\(\\)$\n", "the regex matches the one name exactly");
    assert.match(fs.readFileSync(path.join(dir, "plan", "summary.md"), "utf8"), /5 mutants at 10770 s of tests and 30 s of build each need 5 shard\(s\)/);
    // A plan into a directory that already holds files, which a shard could read as its own, fails
    // when it reads its shards back; so does one where a mutant alone outlasts a shard's time,
    // leaving a shard empty.
    assert.equal(floor("plan", "diff.json", "deleted.json", "10770", "plan").status, 1, "a plan into a directory of files passes");
    const long = floor("plan", "diff.json", "deleted.json", "11000", "long");
    assert.equal(long.stdout, "reason=too-large\ncount=0\nshards=[]\nneeded=6\n");
    assert.match(fs.readFileSync(path.join(dir, "long", "summary.md"), "utf8"), /more than the run has names/);
    // Nothing to mutate, a run past sixteen shards, and a time that is no count.
    write("none.json", "");
    assert.equal(floor("plan", "none.json", "none.json", "100", "empty").stdout, "reason=no-mutants\ncount=0\nshards=[]\nneeded=0\n");
    const big = floor("plan", "diff.json", "deleted.json", "200000", "big");
    assert.equal(big.stdout, "reason=too-large\ncount=0\nshards=[]\nneeded=93\n");
    assert.match(fs.readFileSync(path.join(dir, "big", "summary.md"), "utf8"), /more than the 16 a run takes: split the pull request/);
    for (const seconds of ["", "0", "1.5", "-3", "60 "]) assert.equal(floor("plan", "diff.json", "deleted.json", seconds, "x").status, 1, `a baseline of '${seconds}' seconds passes`);
    // assigned: against cargo-mutants' list for the shard, and against its run's outcomes.
    write("dealt.json", ["x", "y", "y"]);
    write("listed.json", [m("y"), m("x"), m("y")]);
    // Nothing at all when the names agree, not a line feed alone: a file of the output holds a
    // line only for a finding (the review of 560ccd9).
    assert.equal(floor("assigned", "dealt.json", "listed.json").stdout, "");
    write("short.json", [m("x"), m("y")]);
    assert.equal(floor("assigned", "dealt.json", "short.json").stdout, "y: dealt 1 more time(s) than it ran\n");
    fs.mkdirSync(path.join(dir, "out"));
    const outcome = (name) => ({ scenario: { Mutant: { name } }, summary: "CaughtMutant" });
    write("out/outcomes.json", { outcomes: [{ scenario: "Baseline", summary: "Success" }, outcome("x"), outcome("y"), outcome("y"), outcome("z")] });
    assert.equal(floor("assigned", "dealt.json", "out").stdout, "z: ran 1 more time(s) than it was dealt\n");
    write("out/outcomes.json", { outcomes: [outcome("x"), { scenario: { Mutant: {} } }] });
    assert.equal(floor("assigned", "dealt.json", "out").status, 1, "an outcome that names no mutant passes");
    write("bad.json", { x: 1 });
    assert.equal(floor("assigned", "bad.json", "listed.json").status, 1, "a shard's file that is no list of names passes");
    fs.rmSync(path.join(dir, "out", "outcomes.json"));
    assert.equal(floor("assigned", "dealt.json", "out").status, 1, "a run that wrote no outcomes passes");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("Mutants in the diff passes only when the plan succeeded and every shard it made passed, or it found nothing to mutate", () => {
  // The aggregate's script, as mutants.yml runs it, over the results its plan and shards can give.
  const job = parseYaml(read(`${WORKFLOWS}/mutants.yml`)).jobs.aggregate;
  const step = job.steps[0];
  const names = Object.keys(step.env).sort();
  assert.deepEqual(names, ["COUNT", "EVENT", "LIST", "NEEDED", "PLAN", "REASON", "RUN", "SHARDS"], "the aggregate reads other values than this test gives it");
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mutants-aggregate-"));
  try {
    const summary = path.join(dir, "summary.md");
    const run = (values) => {
      const env = { PATH: process.env.PATH, GITHUB_STEP_SUMMARY: summary.replace(/\\/g, "/"), EVENT: "pull_request", PLAN: "success", RUN: "true", REASON: "sharded", COUNT: "3", LIST: "[0,1,2]", NEEDED: "3", SHARDS: "success", ...values };
      const r = spawnSync(bashPath(), ["-e", "-c", step.run], { encoding: "utf8", env, timeout: 30000 });
      return { status: r.status, out: `${r.stdout}${r.stderr}` };
    };
    const passes = [
      [{}, "every one of the plan's 3 shards passed"],
      [{ COUNT: "1", LIST: "[0]", NEEDED: "1" }, "every one of the plan's 1 shards passed"],
      [{ REASON: "no-mutants", COUNT: "0", LIST: "[]", NEEDED: "0", SHARDS: "skipped" }, "the diff changes no line where a mutant lies"],
      [{ RUN: "false", REASON: "", COUNT: "", LIST: "", NEEDED: "", SHARDS: "skipped" }, "the change touches no path the instrument watches"],
      [{ EVENT: "merge_group", RUN: "false", REASON: "", COUNT: "", LIST: "", NEEDED: "", SHARDS: "skipped" }, "not again in the merge queue"],
    ];
    for (const [values, said] of passes) {
      const r = run(values);
      assert.equal(r.status, 0, `${JSON.stringify(values)} fails: ${r.out}`);
      assert.ok(r.out.includes(`::notice::Mutants in the diff: `) && r.out.includes(said), `${JSON.stringify(values)} passes without saying why: ${r.out}`);
    }
    const fails = [
      [{ PLAN: "failure" }, "the plan failed"],
      [{ PLAN: "cancelled" }, "the plan was cancelled"],
      [{ PLAN: "skipped" }, "the plan was skipped"],
      [{ PLAN: "failure", RUN: "false", SHARDS: "skipped" }, "the plan failed after its watch said no"],
      [{ RUN: "" }, "the watch decided nothing"],
      [{ RUN: "maybe" }, "the watch decided neither"],
      [{ RUN: "false", SHARDS: "success" }, "the shards ran though the watch said no"],
      [{ SHARDS: "failure" }, "a shard failed"],
      [{ SHARDS: "cancelled" }, "a shard was cancelled"],
      [{ SHARDS: "skipped" }, "the shards were skipped though the plan made some"],
      [{ SHARDS: "" }, "the shards gave no result"],
      [{ COUNT: "0", LIST: "[]" }, "shards made and none counted"],
      [{ COUNT: "" }, "no count"],
      [{ COUNT: "3x" }, "a count that is no number"],
      [{ NEEDED: "" }, "no count of shards needed"],
      [{ LIST: "[0,1]" }, "fewer shards than counted"],
      [{ LIST: "[0,1,2,3]" }, "more shards than counted"],
      [{ REASON: "no-mutants", COUNT: "2", SHARDS: "skipped" }, "nothing to mutate, with shards counted"],
      [{ REASON: "no-mutants", COUNT: "0", LIST: "[]", SHARDS: "success" }, "nothing to mutate, with shards run"],
      [{ REASON: "too-large", COUNT: "0", LIST: "[]", NEEDED: "20", SHARDS: "skipped" }, "a run past sixteen shards"],
      [{ REASON: "", COUNT: "0", LIST: "[]", SHARDS: "skipped" }, "a count of 0 with no reason"],
      [{ REASON: "unwatched", COUNT: "0", LIST: "[]", SHARDS: "skipped" }, "a reason the plan does not give"],
    ];
    for (const [values, what] of fails) {
      const r = run(values);
      assert.equal(r.status, 1, `${what} passes: ${r.out}`);
      assert.match(r.out, /^::error::Mutants in the diff: /m, `${what} fails without saying why`);
    }
    assert.match(run({ REASON: "too-large", COUNT: "0", LIST: "[]", NEEDED: "20", SHARDS: "skipped" }).out, /need 20 shards .* split the pull request/);
    // An output the plan gave otherwise is named, never echoed into a workflow command.
    for (const values of [{ REASON: "x\n::warning::planted" }, { RUN: "x\n::warning::planted" }, { COUNT: "1\n::warning::planted" }, { SHARDS: "x\n::warning::planted" }]) {
      const r = run(values);
      assert.equal(r.status, 1);
      assert.doesNotMatch(r.out, /^::warning::/m, `${JSON.stringify(values)} reaches the log as a command`);
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("the body CLI gives the verdict the Commits check gives, on the same pull request's text", async () => {
  // A session runs .github/scripts/commits-body.mjs on a pull request's text before it sends it
  // (the escape pull-request-body-unchecked-locally: #213's body quoted Norman's words in Chinese,
  // which only the required check on GitHub found). It must refuse exactly what the Commits job
  // refuses, line for line: both read the text through bodyProblems in lib/commits.mjs. The CLI
  // reads it as a Markdown draft with its title, or as the JSON request a session sends (the review
  // of 5264a01), and the job from the API, as the pull request stands.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mente-commits-body-"));
  const current = { title: "", body: "" };
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://localhost");
    const body =
      url.pathname === "/repos/o/r/actions/runs"
        ? { workflow_runs: [{ created_at: "2026-11-22T10:00:00Z" }] }
        : url.pathname === "/repos/o/r/pulls/7"
          ? { number: 7, title: current.title, body: current.body, user: { login: "menteion-agent[bot]" } }
          : url.pathname.startsWith("/repos/o/r/issues/7/")
            ? []
            : null;
    res.writeHead(body ? 200 : 404, { "content-type": "application/json" });
    res.end(JSON.stringify(body ?? {}));
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  try {
    const sh = testGit(dir);
    sh("init", "-q", "-b", "main");
    fs.writeFileSync(path.join(dir, "a"), "a\n");
    sh("add", "-A");
    sh("commit", "-q", "-m", "Base");
    const base = sh("rev-parse", "HEAD");
    fs.writeFileSync(path.join(dir, "a"), "b\n");
    sh("commit", "-q", "-am", "Change a");
    const head = sh("rev-parse", "HEAD");
    const env = Object.fromEntries(Object.entries(GIT_TEST_ENV).filter(([k]) => !k.startsWith("GITHUB_")));
    const event = path.join(dir, "event.json");
    fs.writeFileSync(event, JSON.stringify({ action: "edited", pull_request: { number: 7, base: { sha: base }, head: { sha: head }, user: { login: "menteion-agent[bot]" } } }));
    const job = (title, body) =>
      new Promise((done) => {
        Object.assign(current, { title, body });
        const child = spawn(process.execPath, [path.join(HERE, "commits.mjs")], {
          cwd: dir,
          env: { ...env, GITHUB_EVENT_NAME: "pull_request", GITHUB_EVENT_PATH: event, GITHUB_REPOSITORY: "o/r", GITHUB_API_URL: `http://127.0.0.1:${server.address().port}`, GITHUB_TOKEN: "t" },
        });
        let stdout = "";
        child.stdout.on("data", (d) => (stdout += d));
        child.on("close", (status) => done({ status, said: stdout.split("\n").filter((l) => l.startsWith("::error::the pull request's")).map((l) => l.slice("::error::".length)) }));
      });
    const local = (title, body) => {
      const file = path.join(dir, "body.md");
      fs.writeFileSync(file, body);
      const r = spawnSync(process.execPath, [path.join(HERE, "commits-body.mjs"), file, "--title", title], { encoding: "utf8", timeout: 30000 });
      return { status: r.status, said: r.stdout.split("\n").filter((l) => l.startsWith("the pull request's")) };
    };
    const request = (title, body) => {
      const file = path.join(dir, "request.json");
      fs.writeFileSync(file, `\uFEFF${JSON.stringify({ title, body, head: "h", base: "main" })}`);
      const r = spawnSync(process.execPath, [path.join(HERE, "commits-body.mjs"), file], { encoding: "utf8", timeout: 30000 });
      return { status: r.status, said: r.stdout.split("\n").filter((l) => l.startsWith("the pull request's")) };
    };
    const han = String.fromCodePoint(0x4e2d, 0x6587);
    const cases = [
      ["T", "Root cause: x.\nRemoved at the root: y."],
      ["T", "Root cause: x.\nNot removable at the root: y.\nGuard: policy check \"z\""],
      ["T", "Not a fix: a brief."],
      ["T", "A summary, and no declaration."],
      ["T", "Root cause: x\nRemoved at the root: y\nNot a fix: z"],
      ["T", "Root cause:\nRemoved at the root: y"],
      ["T", "**Root cause:** x\nRemoved at the root: y"],
      ["T", `Not a fix: a brief.\nNorman: ${han}`],
      [`A title ${han}`, "Not a fix: a brief."],
      ["T", "```\nNot a fix: quoted\n```"],
      ["T", ""],
    ];
    let refused = 0;
    for (const [title, body] of cases) {
      const [onGitHub, here, sent] = [await job(title, body), local(title, body), request(title, body)];
      assert.equal(here.status, onGitHub.status, `the verdicts differ on ${JSON.stringify([title, body])}: here ${here.status}, on GitHub ${onGitHub.status}`);
      assert.deepEqual(here.said, onGitHub.said, `the messages differ on ${JSON.stringify([title, body])}`);
      assert.equal(sent.status, onGitHub.status, `the verdicts differ on the request of ${JSON.stringify([title, body])}: here ${sent.status}, on GitHub ${onGitHub.status}`);
      assert.deepEqual(sent.said, onGitHub.said, `the messages differ on the request of ${JSON.stringify([title, body])}`);
      if (onGitHub.status === 1) refused++;
    }
    assert.equal(refused, cases.length - 3, "the cases refused are not those that break a rule");
    // A call the CLI cannot read is no verdict.
    assert.equal(spawnSync(process.execPath, [path.join(HERE, "commits-body.mjs")], { encoding: "utf8" }).status, 2);
    assert.equal(spawnSync(process.execPath, [path.join(HERE, "commits-body.mjs"), "a", "b"], { encoding: "utf8" }).status, 2);
  } finally {
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("the session's hook refuses a request whose pull request text fails the Commits check's rules, read from the file the client sends", () => {
  // .claude/settings.json runs tools/push-guard.mjs before each shell command, with the event on
  // standard input; it refuses, status 2 and the reason on standard error, a request that would send
  // a pull request's text the Commits check refuses (tools/pr-body-guard.mjs; the review of 5264a01:
  // nothing made a session run the body CLI, so a body could still be sent unchecked).
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mente-body-guard-"));
  try {
    const at = (name, request) => {
      const file = path.join(dir, name).replace(/\\/g, "/");
      fs.writeFileSync(file, JSON.stringify(request));
      return file;
    };
    const good = at("good.json", { title: "T", body: "Not a fix: a brief." });
    const bare = at("bare.json", { title: "T", body: "No declaration." });
    const hook = (command) => spawnSync(process.execPath, [path.join(ROOT, "tools", "push-guard.mjs")], { input: JSON.stringify({ tool_input: { command }, cwd: dir }), encoding: "utf8", timeout: 30000 });
    const client = "node C:/Users/x/tools/github-app/gh-app.mjs api PATCH repos/Menteion/Mente/pulls/214 --data-file";
    const refused = hook(`${client} ${bare}`);
    assert.equal(refused.status, 2, `a body with no declaration was let through: ${refused.stderr}`);
    assert.match(refused.stderr, /the pull request body guard refused it: .* sends a pull request's text that fails the Commits check's rules[\s\S]*- the pull request's body: it declares neither/);
    assert.doesNotMatch(refused.stderr, /No declaration\./, "the body's text reached the hook's words");
    const passed = hook(`${client} ${good}`);
    assert.equal(passed.status, 0, `a declared body was refused: ${passed.stderr}`);
    assert.equal(hook(`${client} ${path.join(dir, "missing.json").replace(/\\/g, "/")}`).status, 2, "a request file the hook cannot read was let through");
    assert.equal(hook("git status").status, 0);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("a shard's listing and its run hold it to the names the plan dealt, as mutants.yml runs them", () => {
  // The two steps of a shard that read the plan, under bash, as the workflow runs them, in a tree
  // of the floor's own scripts, with stand-ins for cargo, the sandbox's script and the tools'
  // check. The review of 560ccd9 found every shard failing on a listing that matched, since a line
  // feed alone counted as a difference; no test ran these steps, and the pull request's own run
  // dealt no mutant.
  const job = parseYaml(read(`${WORKFLOWS}/mutants.yml`)).jobs.shard;
  const listing = job.steps.find((s) => s.name === "This shard's mutants, as the plan dealt them, before any code of the change runs");
  const mutate = job.steps.find((s) => s.name === "Mutate this shard's mutants");
  assert.ok(listing && mutate, "mutants.yml's shard has no listing step or no step that mutates, by those names");
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mutants-shard-"));
  try {
    const put = (file, text) => {
      fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true });
      fs.writeFileSync(path.join(dir, file), text);
    };
    for (const f of [".github/scripts/mutants-floor.mjs", ".github/scripts/lib/mutants-floor.mjs"]) put(f, read(f));
    put(".github/scripts/tools.sh", "exit 0\n");
    put(".github/scripts/test-sandbox.sh", 'echo "test-sandbox.sh $*" >> "$RUNNER_TEMP/sandbox.log"\n');
    put(".github/scripts/limits.sh", ": limits\n");
    // cargo lists what STUB_LIST holds, and a run of mutants writes STUB_RUN's outcomes into its
    // --output directory, each caught, with a log as nextest leaves it.
    put(
      "bin/cargo",
      [
        "#!/usr/bin/env bash",
        'case " $* " in',
        '  *" --list "*) cat "$STUB_LIST" ;;',
        '  *) out=; while [ "$#" -gt 0 ]; do if [ "$1" = --output ]; then out=$2; fi; shift; done',
        '     mkdir -p "$out/mutants.out/log"; node "$STUB_RUN" "$out/mutants.out" ;;',
        "esac",
        "",
      ].join("\n"),
    );
    fs.chmodSync(path.join(dir, "bin", "cargo"), 0o755);
    put(
      "stub-run.mjs",
      [
        'import fs from "node:fs";',
        "const out = process.argv[2];",
        'const names = JSON.parse(fs.readFileSync(process.env.STUB_TESTED, "utf8"));',
        "const log = \"info: using target runner `/opt/mente-test-sandbox/test-sandbox run` defined by environment variable `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER`\\n        FAIL [   0.004s] mente-syntax tests::x\\n\";",
        "const outcomes = names.map((name, k) => {",
        "  fs.writeFileSync(`${out}/log/m${k}.log`, log);",
        '  return { scenario: { Mutant: { name, file: "compiler/a/src/lib.rs" } }, summary: "CaughtMutant", log_path: `log/m${k}.log`, phase_results: [{ phase: "Build", process_status: "Success" }, { phase: "Test", process_status: { Failure: 100 } }] };',
        "});",
        'fs.writeFileSync(`${out}/outcomes.json`, JSON.stringify({ outcomes: [{ scenario: "Baseline", summary: "Success" }, ...outcomes] }));',
        'fs.writeFileSync(`${out}/caught.txt`, names.map((n) => `${n}\\n`).join(""));',
        'for (const f of ["missed.txt", "timeout.txt", "unviable.txt"]) fs.writeFileSync(`${out}/${f}`, "");',
        "",
      ].join("\n"),
    );
    // The plan, by the real command: two mutants at a baseline that gives them one shard.
    const m = (name) => ({ name, file: "compiler/a/src/lib.rs", function: null, span: { start: { line: 1, column: 1 }, end: { line: 1, column: 2 } } });
    const both = [m("a.rs:1:1: replace f -> bool with true"), m("a.rs:2:1: replace g (x|y) with ()")];
    put("diff.json", JSON.stringify(both));
    put("none.json", "");
    const planned = spawnSync(process.execPath, [".github/scripts/mutants-floor.mjs", "plan", "diff.json", "none.json", "100", "plan-made/plan"], { cwd: dir, encoding: "utf8", timeout: 30000 });
    assert.equal(planned.status, 0, planned.stderr);
    assert.match(planned.stdout, /^count=1$/m);
    const posix = (p) => p.replace(/\\/g, "/");
    const run = (script, lists) => {
      put("list.json", JSON.stringify(lists.listed ?? both));
      put("tested.json", JSON.stringify((lists.tested ?? both).map((x) => x.name)));
      fs.rmSync(path.join(dir, "runner"), { recursive: true, force: true });
      fs.mkdirSync(path.join(dir, "runner"));
      // The plan stands where the shard downloads it, under the runner's temporary directory, outside
      // the tree (the escape plan-downloaded-into-the-checkout): the plan made beside it has another
      // name, so that a step reading the plan from the tree finds nothing.
      fs.cpSync(path.join(dir, "plan-made"), path.join(dir, "runner", "mutants-plan"), { recursive: true });
      const env = {
        ...process.env,
        PATH: `${path.join(dir, "bin")}${path.delimiter}${process.env.PATH}`,
        SHARD: lists.shard ?? "0",
        RUNNER_TEMP: posix(path.join(dir, "runner")),
        GITHUB_STEP_SUMMARY: posix(path.join(dir, "summary.md")),
        STUB_LIST: posix(path.join(dir, "list.json")),
        STUB_RUN: posix(path.join(dir, "stub-run.mjs")),
        STUB_TESTED: posix(path.join(dir, "tested.json")),
      };
      const r = spawnSync(bashPath(), ["-e", "-c", script], { cwd: dir, encoding: "utf8", env, timeout: 60000 });
      return { status: r.status, out: `${r.stdout}${r.stderr}` };
    };
    // The listing: the names dealt, each once, pass; one left out, or one beside them, fails.
    const ok = run(listing.run, {});
    assert.equal(ok.status, 0, `a listing of exactly the names dealt fails: ${ok.out}`);
    const short = run(listing.run, { listed: both.slice(0, 1) });
    assert.equal(short.status, 1, `a listing short of a name passes: ${short.out}`);
    assert.match(short.out, /::error::cargo-mutants lists 1 of this shard's mutants otherwise than the plan dealt them/);
    const extra = run(listing.run, { listed: [...both, m("a.rs:3:1: delete ! in h")] });
    assert.equal(extra.status, 1, "a listing with a name beside those dealt passes");
    const bad = run(listing.run, { shard: "0\n::warning::planted" });
    assert.equal(bad.status, 1);
    assert.doesNotMatch(bad.out, /^::warning::/m, "the matrix's value reaches the log as a command");
    // The run: every name dealt tested passes, with the sandbox's count of the mutants caught;
    // one left untested fails.
    const tested = run(mutate.run, {});
    assert.equal(tested.status, 0, `a run that tested exactly the names dealt fails: ${tested.out}`);
    assert.match(fs.readFileSync(path.join(dir, "runner", "sandbox.log"), "utf8"), /^test-sandbox\.sh ran 2$/m);
    const untested = run(mutate.run, { tested: both.slice(1) });
    assert.equal(untested.status, 1, `a run that left a dealt name untested passes: ${untested.out}`);
    assert.match(untested.out, /::error::1 of this shard's mutants were tested otherwise than the plan dealt them/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("a shard installs cargo-mutants only as the binary its builder took the digest of", () => {
  // The step of a pull request's shards and of the full run's, under bash: the binary handed on is
  // installed when its digest is the builder's, and refused, uninstalled, otherwise.
  const step = (file, job) => parseYaml(read(`${WORKFLOWS}/${file}`)).jobs[job].steps.find((s) => /sha256sum -c "\$RUNNER_TEMP\/cargo-mutants\.sha256"/.test(String(s.run ?? "")));
  const shard = step("mutants.yml", "shard");
  const full = step("mutants-full.yml", "full");
  assert.ok(shard && full, "a shard has no step that checks cargo-mutants' digest");
  assert.equal(full.run, shard.run, "the full run's shards check cargo-mutants otherwise than a pull request's");
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mutants-binary-"));
  try {
    const posix = (p) => p.replace(/\\/g, "/");
    const binary = path.join(dir, "temp", "cargo-mutants", "cargo-mutants");
    fs.mkdirSync(path.dirname(binary), { recursive: true });
    fs.mkdirSync(path.join(dir, "home", ".cargo", "bin"), { recursive: true });
    fs.writeFileSync(binary, "the binary the builder built\n");
    const digest = crypto.createHash("sha256").update(fs.readFileSync(binary)).digest("hex");
    const installed = path.join(dir, "home", ".cargo", "bin", "cargo-mutants");
    const run = (DIGEST) => {
      fs.rmSync(installed, { force: true });
      const env = { ...process.env, DIGEST, HOME: posix(path.join(dir, "home")), RUNNER_TEMP: posix(path.join(dir, "temp")) };
      const r = spawnSync(bashPath(), ["-e", "-c", shard.run], { encoding: "utf8", env, timeout: 30000 });
      return { status: r.status, out: `${r.stdout}${r.stderr}`, installed: fs.existsSync(installed) };
    };
    const ok = run(digest);
    assert.equal(ok.status, 0, `the builder's binary is refused: ${ok.out}`);
    assert.ok(ok.installed, "the builder's binary is not installed");
    assert.equal(fs.readFileSync(installed, "utf8"), "the binary the builder built\n");
    for (const [what, value] of [
      ["another binary's digest", crypto.createHash("sha256").update("another").digest("hex")],
      ["no digest", ""],
      ["a digest in capitals", digest.toUpperCase()],
      ["a short digest", digest.slice(0, 63)],
      ["a digest and more", `${digest}0`],
    ]) {
      const r = run(value);
      assert.equal(r.status, 1, `${what} passes: ${r.out}`);
      assert.ok(!r.installed, `${what} installs the binary`);
    }
    const injected = run(`${digest}\n::warning::planted`);
    assert.equal(injected.status, 1);
    assert.doesNotMatch(injected.out, /^::warning::/m, "the digest reaches the log as a command");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
