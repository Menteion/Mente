// The local limits, proved on the machine they guard (tools/local-limits.mjs, tools/capped.ps1):
// a process that allocates without end dies alone at its limit; a command past its time ends with
// every process it started; a third build waits while two hold the machine's slots; a slot whose
// holder is killed is free at once; outside a checkout of this repository, menteOnly leaves a
// command untouched; every argument arrives as given; a grandchild started through Windows
// PowerShell stays in the job; the command ends with the process that started its wrapper; and the
// raw-build hook refuses when it cannot read its event. Windows only, but for the hook: elsewhere
// nothing is bounded, and these are skipped.
//
//   node --test tools/local-limits.test.mjs
//
// Each proof runs at small limits, so that it costs the machine little: 1 GB, a few seconds.

import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { gatesDir } from "./local-limits.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..");
const skip = process.platform !== "win32" && "the local limits bound processes on Windows alone";
// Slots of their own for every proof, which each process they start inherits: the machine's may
// both be held, by the quick checks running these proofs and by another session, and a proof's run
// held there would wait past its time (it did, on 2026-11-22, for the runner's proof).
process.env.MENTE_SLOTS_DIR = fs.mkdtempSync(path.join(os.tmpdir(), "proof-slots-"));

/// Runs `run` through local-limits' runCapped under `limits`, from a node process of its own in
/// `cwd`, as the tools do: its status, output and time; or, with `wait` false, that process.
function viaRunCapped(run, limits, { wait = true, cwd = ROOT, menteOnly = false } = {}) {
  const script = `import(${JSON.stringify(new URL("./local-limits.mjs", import.meta.url).href)}).then((m)=>process.exit(m.runCapped(JSON.parse(process.env.TEST_RUN),JSON.parse(process.env.TEST_LIMITS),{menteOnly:process.env.TEST_MENTE_ONLY==="1"})))`;
  const env = { ...process.env, TEST_RUN: JSON.stringify(run), TEST_LIMITS: JSON.stringify(limits), TEST_MENTE_ONLY: menteOnly ? "1" : "0" };
  if (!wait) return spawn(process.execPath, ["-e", script], { env, cwd, stdio: "ignore" });
  const started = Date.now();
  const r = spawnSync(process.execPath, ["-e", script], { env, cwd, encoding: "utf8", timeout: 120000 });
  return { status: r.status, out: `${r.stdout}${r.stderr}`, stdout: r.stdout, ms: Date.now() - started };
}

const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};

test("a process that allocates without end dies alone at its limit, which the wrapper names", { skip }, () => {
  // The loop stops itself at 2 GB, so that a cap that holds nothing fails this proof without
  // taking the machine down (the first design's children were born outside the job: the desktop
  // app's job, which the shell is in, lets them break away).
  const r = viaRunCapped([process.execPath, "-e", "const a=[];let n=0;try{for(;;){if(n*64>=2048){console.log('not refused at 2048 MB');process.exit(8)}a.push(Buffer.alloc(1<<26));n++}}catch(e){console.log(`refused after ${n*64} MB`);process.exit(7)}"], { processGB: 1, jobGB: 2, minutes: 1 });
  assert.equal(r.status, 7, r.out);
  const mb = Number(/refused after (\d+) MB/.exec(r.out)?.[1]);
  assert.ok(mb > 0 && mb < 1024, `allocated ${mb} MB under a limit of 1 GB: ${r.out}`);
  assert.match(r.out, /capped: a process of .* reached its limit of 1 GB committed/);
});

test("a command past its time ends, with every process it started", { skip }, () => {
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "capped-")), "child");
  const r = viaRunCapped([process.execPath, "-e", "const c=require('child_process').spawn(process.execPath,['-e','setTimeout(()=>{},600000)'],{stdio:'ignore'});require('fs').writeFileSync(process.argv[1],String(c.pid));setTimeout(()=>{},600000)", file], { processGB: 1, jobGB: 2, minutes: 0.05 });
  assert.equal(r.status, 124, r.out);
  assert.ok(r.ms < 30000, `it took ${r.ms} ms to stop a command at 3 s`);
  assert.match(r.out, /capped: stopped .* at its limit of 0.05 minutes/);
  const child = Number(fs.readFileSync(file, "utf8"));
  assert.ok(child > 0 && !alive(child), `the command's child ${child} outlived it`);
});

test("outside a checkout of this repository, menteOnly leaves a command untouched", { skip }, () => {
  const wait = [process.execPath, "-e", "setTimeout(()=>{},4000)"];
  const limits = { processGB: 1, jobGB: 2, minutes: 0.02 };
  assert.equal(viaRunCapped(wait, limits, { menteOnly: true }).status, 124, "menteOnly left a command in the repository unbounded");
  assert.equal(viaRunCapped(wait, limits, { menteOnly: true, cwd: os.tmpdir() }).status, 0, "menteOnly bounded a command outside the repository");
});

/// A process that takes a slot in `dir` and holds it for `ms`, and says "held <slot>" once it has.
function holder(dir, ms) {
  const script = `import(${JSON.stringify(new URL("./local-limits.mjs", import.meta.url).href)}).then((m)=>{const s=m.acquireSlot("test holder");console.log("held "+s.slot);setTimeout(()=>s.release(),${ms})})`;
  const child = spawn(process.execPath, ["-e", script], { env: { ...process.env, MENTE_SLOTS_DIR: dir }, stdio: ["ignore", "pipe", "inherit"] });
  const held = new Promise((done) => child.stdout.on("data", (d) => /held \d/.test(String(d)) && done()));
  return { child, held, ended: new Promise((done) => child.on("exit", done)) };
}

/// Takes a slot of `dir` in a process of its own, as a third build would: the slot it got, how long
/// it took, and whether it said it was waiting.
function third(dir) {
  const script = `import(${JSON.stringify(new URL("./local-limits.mjs", import.meta.url).href)}).then((m)=>{const t=Date.now();let waited=false;const s=m.acquireSlot("third",{pollMs:100,say:()=>{waited=true}});console.log(JSON.stringify({slot:s.slot,ms:Date.now()-t,waited}))})`;
  const r = spawnSync(process.execPath, ["-e", script], { env: { ...process.env, MENTE_SLOTS_DIR: dir }, encoding: "utf8", timeout: 60000 });
  return JSON.parse(r.stdout.trim());
}

test("a third build waits while two hold the machine's slots, and takes the first freed", { skip }, async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "slots-"));
  const a = holder(dir, 3000);
  const b = holder(dir, 60000);
  await Promise.all([a.held, b.held]);
  const got = third(dir);
  b.child.kill();
  await Promise.all([a.ended, b.ended]);
  assert.equal(got.waited, true, "the third build did not wait");
  assert.ok(got.ms >= 1500, `the third build waited ${got.ms} ms for a slot held 3 s`);
  assert.ok(got.slot === 1 || got.slot === 2, JSON.stringify(got));
});

test("every argument reaches the command as it was given, whatever PowerShell would make of it", { skip }, () => {
  // The review of 8f03074: through `pwsh -File`, `--skip=mir_run::coverage::` arrived as two
  // arguments, and `-x:` and `--%` vanished, so a check could run another set of tests.
  const args = ["--skip=mir_run::coverage::", "-Clink-arg=/STACK:8000000", "-x:", "--%", "", "a b", 'q"uote', "back\\slash\\", "-ProcessGB", "--", "%PATH%", "$env:PATH"];
  // A script file, after which node reads every argument as the script's.
  const echo = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "capped-")), "echo.js");
  fs.writeFileSync(echo, "console.log(JSON.stringify(process.argv.slice(2)))\n");
  const r = viaRunCapped([process.execPath, echo, ...args], { processGB: 1, jobGB: 2, minutes: 1 });
  assert.equal(r.status, 0, r.out);
  assert.deepEqual(JSON.parse(r.stdout.trim().split(/\r?\n/).at(-1)), args);
});

test("a grandchild started through Windows PowerShell stays in the job", { skip }, () => {
  // The review of 8f03074: the Store's pwsh, started inside the job, leaves it with every process
  // it starts. Windows PowerShell does not; the grandchild below allocates without end under 1 GB.
  const powershell = path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
  const grandchild = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "capped-")), "grow.js");
  fs.writeFileSync(grandchild, "const a=[];let n=0;try{for(;;){if(n*64>=2048){console.log('not refused at 2048 MB');process.exit(8)}a.push(Buffer.alloc(1<<26));n++}}catch(e){console.log(`refused after ${n*64} MB`);process.exit(7)}\n");
  const r = viaRunCapped([powershell, "-NoProfile", "-NonInteractive", "-Command", `& '${process.execPath}' '${grandchild}'; exit $LASTEXITCODE`], { processGB: 1, jobGB: 2, minutes: 1 });
  const mb = Number(/refused after (\d+) MB/.exec(r.out)?.[1]);
  assert.ok(mb > 0 && mb < 1024, `the grandchild allocated ${mb} MB under a limit of 1 GB: ${r.out}`);
});

test("when the process that started the wrapper ends, the command ends with it", { skip }, async () => {
  // The review of 8f03074: a killed slot holder left its wrapper and the build running, its slot
  // already free.
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "capped-")), "pid");
  const parent = viaRunCapped([process.execPath, "-e", "require('fs').writeFileSync(process.argv[1],String(process.pid));setTimeout(()=>{},600000)", file], { processGB: 1, jobGB: 2, minutes: 10 }, { wait: false });
  for (let k = 0; k < 100 && !fs.existsSync(file); k++) await new Promise((done) => setTimeout(done, 100));
  const command = Number(fs.readFileSync(file, "utf8"));
  assert.ok(command > 0 && alive(command), "the command did not start");
  parent.kill("SIGKILL");
  let gone = false;
  for (let k = 0; k < 100 && !gone; k++) {
    await new Promise((done) => setTimeout(done, 100));
    gone = !alive(command);
  }
  if (!gone) process.kill(command);
  assert.ok(gone, "the command outlived the process that started its wrapper by 10 s");
});

test("a capped command may not be the Store's pwsh, and is told what to run instead", { skip }, () => {
  // Norman, 2026-11-22: the Store's PowerShell 7 stays, and Mente closes its way out of the job.
  const limits = { processGB: 1, jobGB: 2, minutes: 1 };
  const store = path.join(process.env.LOCALAPPDATA ?? "C:\\Users\\x\\AppData\\Local", "Microsoft", "WindowsApps", "pwsh.exe");
  for (const program of ["pwsh", "PWSH.EXE", store, "C:\\Program Files\\WindowsApps\\Microsoft.PowerShell_7.7.0.0_x64__8wekyb3d8bbwe\\pwsh.exe"]) {
    const r = viaRunCapped([program, "-NoProfile", "-Command", "exit 0"], limits);
    assert.equal(r.status, 2, `${program}: ${r.out}`);
    assert.match(r.out, /C:\\Windows\\System32\\WindowsPowerShell\\v1\.0\\powershell\.exe instead/, program);
  }
});

test("a capped child that starts pwsh by name finds none on its PATH", { skip }, () => {
  // The App Execution Alias directory, where the Store's pwsh is found by name, is off the capped
  // PATH: a test that starts `pwsh` cannot take its processes out of the job.
  const r = viaRunCapped([process.execPath, "-e", "const r=require('child_process').spawnSync('pwsh',['-NoProfile','-Command','exit 0']);console.log(r.error?r.error.code:'started '+r.status)"], { processGB: 1, jobGB: 2, minutes: 1 });
  assert.equal(r.status, 0, r.out);
  assert.match(r.stdout, /^ENOENT$/m, `a capped child started pwsh by name: ${r.out}`);
});

test("cargo's runner hands a binary its arguments as given, bounded in this repository and untouched elsewhere", { skip }, () => {
  // tools/capped.mjs, as ~/.cargo/config.toml names it, with no slot: in a checkout of this
  // repository the command runs through capped.ps1, which sets the slot's share of the cores;
  // elsewhere it runs as cargo gave it.
  const echo = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "capped-")), "echo.js");
  fs.writeFileSync(echo, "console.log(JSON.stringify([process.env.CARGO_BUILD_JOBS ?? null, ...process.argv.slice(2)]))\n");
  const args = ["--skip=mir_run::coverage::", "-Clink-arg=/STACK:8000000", "--%", ""];
  const env = { ...process.env };
  delete env.CARGO_BUILD_JOBS;
  const runner = (cwd) => {
    const r = spawnSync(process.execPath, [path.join(HERE, "capped.mjs"), "--mente-only", "--", process.execPath, echo, ...args], { cwd, env, encoding: "utf8", timeout: 60000 });
    assert.equal(r.status, 0, `${r.stdout}${r.stderr}`);
    return JSON.parse(r.stdout.trim().split(/\r?\n/).at(-1));
  };
  const [jobs, ...inside] = runner(ROOT);
  assert.deepEqual(inside, args);
  assert.ok(Number(jobs) > 0, "cargo's runner left a binary of this repository unbounded");
  assert.deepEqual(runner(os.tmpdir()), [null, ...args], "cargo's runner changed a command outside this repository");
});

test("when a parent that is not node ends, the wrapper ends the command too", { skip }, async () => {
  // node's own children die with it (libuv keeps them in a job of its own), so the test above
  // passes through that alone; here the wrapper's parent is a Windows PowerShell, killed while the
  // test's node, whose job holds the wrapper too, lives on: only the wrapper's watch of its parent
  // ends the command.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "capped-"));
  const file = path.join(dir, "pid");
  const sleeper = path.join(dir, "sleep.js");
  fs.writeFileSync(sleeper, "require('fs').writeFileSync(process.argv[2],String(process.pid));setTimeout(()=>{},600000)\n");
  const powershell = path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
  const call = `$env:MENTE_CAPPED = (@{ command = @('${process.execPath}', '${sleeper}', '${file}'); processGB = 1; jobGB = 2; minutes = 10; parent = $PID } | ConvertTo-Json -Compress); & '${powershell}' -NoProfile -NonInteractive -ExecutionPolicy Bypass -File '${path.join(HERE, "capped.ps1")}'`;
  const parent = spawn(powershell, ["-NoProfile", "-NonInteractive", "-Command", call], { stdio: "ignore" });
  for (let k = 0; k < 200 && !fs.existsSync(file); k++) await new Promise((done) => setTimeout(done, 100));
  const command = Number(fs.readFileSync(file, "utf8"));
  assert.ok(command > 0 && alive(command), "the command did not start");
  parent.kill("SIGKILL");
  let gone = false;
  for (let k = 0; k < 100 && !gone; k++) {
    await new Promise((done) => setTimeout(done, 100));
    gone = !alive(command);
  }
  if (!gone) process.kill(command);
  assert.ok(gone, "the command outlived the PowerShell that started its wrapper by 10 s");
});

/**
 * Whether a run of cargo.mjs got past cargo's runner: cargo ran, or the tools refused for git's
 * hooks alone (local-limits.mjs, requireHooks), which they read from their own checkout after the
 * runner, so that this proof of the runner holds whatever this machine's hooks are.
 */
/// The gates' stable runner entry of this checkout, which the runner line names.
const ENTRY = path.join(gatesDir(ROOT), "entry", "capped.mjs").replace(/\\/g, "/");

const pastRunner = (r) => r.status === 0 || (r.status === 2 && /core\.hooksPath/.test(r.stderr) && !/cargo's runner|_RUNNER is set/.test(r.stderr));

test("the tools refuse to run until cargo's runner is the gates' capped one, and name the lines to add", { skip }, (t) => {
  // The review of round 4: CLAUDE.md called cargo's runner the wall, which nothing set or checked.
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "cargo-home-"));
  const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => k !== "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER"));
  const cargo = () => spawnSync(process.execPath, [path.join(HERE, "cargo.mjs"), "--version"], { cwd: ROOT, env: { ...env, CARGO_HOME: home }, encoding: "utf8", timeout: 60000 });
  const none = cargo();
  // The tools refresh the gates before they read the runner: with no copy installed, they refuse
  // for that alone, and this proof waits for the bootstrap.
  if (/no copy of main's gates is installed/.test(none.stderr)) return t.skip("no copy of main's gates is installed here: the bootstrap has not run");
  assert.equal(none.status, 2, `${none.stdout}${none.stderr}`);
  assert.match(none.stderr, /\[target\.x86_64-pc-windows-msvc\]\nrunner = \["node", ".*\/mente-gates\/entry\/capped\.mjs", "--mente-only", "--"\]/);
  assert.ok(none.stderr.includes(path.join(home, "config.toml")), none.stderr);
  for (const [runner, what] of [
    ['["node", "C:/nowhere/tools/capped.mjs", "--mente-only", "--"]', "a capped.mjs that does not exist"],
    ['["node", "' + ENTRY + '", "--"]', "no --mente-only"],
    ['"wine"', "another runner"],
  ]) {
    fs.writeFileSync(path.join(home, "config.toml"), `[build]\njobs = 12\n\n[target.x86_64-pc-windows-msvc]\nrunner = ${runner}\n`);
    const refused = cargo();
    assert.equal(refused.status, 2, `the tools ran with ${what}`);
    assert.match(refused.stderr, /cargo's runner on this laptop is/, `the tools refused ${what} for another reason than the runner`);
  }
  // The capped runner, but for another target: cargo would not use it here.
  fs.writeFileSync(path.join(home, "config.toml"), `[target.aarch64-pc-windows-msvc]\nrunner = ["node", "${ENTRY}", "--mente-only", "--"]\n`);
  const elsewhere = cargo();
  assert.equal(elsewhere.status, 2, "the tools ran with the capped runner set for another target");
  assert.match(elsewhere.stderr, /cargo's runner on this laptop is/, "the tools refused the runner of another target for another reason");
  fs.writeFileSync(path.join(home, "config.toml"), `[build]\njobs = 12\n\n[target.x86_64-pc-windows-msvc]\nrunner = ["node", "${ENTRY}", "--mente-only", "--"]\n`);
  const held = cargo();
  assert.ok(pastRunner(held), `${held.stdout}${held.stderr}`);
  if (held.status === 0) assert.match(held.stdout, /^cargo \d/);
  // The variable cargo reads before every file: the capped runner by it alone is held, and another
  // runner by it is refused whatever the file says, as cargo would run that one.
  const variable = (value, config) => {
    fs.writeFileSync(path.join(home, "config.toml"), config);
    return spawnSync(process.execPath, [path.join(HERE, "cargo.mjs"), "--version"], { cwd: ROOT, env: { ...env, CARGO_HOME: home, CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER: value }, encoding: "utf8", timeout: 60000 });
  };
  const capped = `node ${ENTRY} --mente-only --`;
  assert.ok(pastRunner(variable(capped, "")), "the capped runner set by CARGO_TARGET_..._RUNNER was refused");
  const good = `[target.x86_64-pc-windows-msvc]\nrunner = ["node", "${ENTRY}", "--mente-only", "--"]\n`;
  const over = variable("wine", good);
  assert.equal(over.status, 2, "another runner set by CARGO_TARGET_..._RUNNER passed over a capped one in the file");
  // The refusal names the variable, which is what to change, not the file, which is right (the
  // review of PR 195's last round).
  assert.match(over.stderr, /CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER is set to "wine"/);
  assert.match(over.stderr, /\nCARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER=node .*\/mente-gates\/entry\/capped\.mjs --mente-only --\n/);
});

test("the hook's command refuses when the guard cannot load", () => {
  // The review of round 4: a guard that fails to load exits 1, which Claude Code lets pass.
  const command = JSON.parse(fs.readFileSync(path.join(ROOT, ".claude", "settings.json"), "utf8")).hooks.PreToolUse[0].hooks[0].command;
  const nowhere = fs.mkdtempSync(path.join(os.tmpdir(), "no-guard-"));
  const run = (dir, input) => spawnSync("bash", ["-c", command], { env: { ...process.env, CLAUDE_PROJECT_DIR: dir.replace(/\\/g, "/") }, input, encoding: "utf8", timeout: 30000 });
  const event = JSON.stringify({ tool_input: { command: "node tools/cargo.mjs test" } });
  assert.equal(run(nowhere, event).status, 2, "a guard that cannot load let the command through");
  assert.equal(run(ROOT, event).status, 0, "the guard refused a capped command");
  assert.equal(run(ROOT, JSON.stringify({ tool_input: { command: "cargo test" } })).status, 2, "the guard let a raw cargo test through");
});

test("a run longer than the ceiling of minutes is refused before it starts", { skip }, () => {
  const r = spawnSync(process.execPath, [path.join(HERE, "cargo.mjs"), "--minutes", "61", "--version"], { cwd: ROOT, encoding: "utf8", timeout: 60000 });
  assert.equal(r.status, 2, `${r.stdout}${r.stderr}`);
  assert.match(r.stderr, /--minutes takes a number above 0 and at most 60/);
});

test("the raw-build hook refuses when it cannot read its event, rather than let the command through", () => {
  for (const input of ["not json", "", '{"tool_input":{"command":42}}']) {
    const r = spawnSync(process.execPath, [path.join(HERE, "raw-build-guard.mjs")], { input, encoding: "utf8", timeout: 30000 });
    assert.equal(r.status, 2, `${JSON.stringify(input)}: status ${r.status}`);
    assert.match(r.stderr, /the raw-build guard/, JSON.stringify(input));
  }
  const ok = spawnSync(process.execPath, [path.join(HERE, "raw-build-guard.mjs")], { input: JSON.stringify({ tool_input: { command: "node tools/cargo.mjs test" } }), encoding: "utf8", timeout: 30000 });
  assert.equal(ok.status, 0, ok.stderr);
});

test("a slot whose holder is killed is free at once", { skip }, async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "slots-"));
  const a = holder(dir, 60000);
  const b = holder(dir, 60000);
  await Promise.all([a.held, b.held]);
  a.child.kill("SIGKILL");
  await a.ended;
  const got = third(dir);
  b.child.kill();
  await b.ended;
  assert.equal(got.waited, false, `the third build waited ${got.ms} ms for the slot of a killed holder`);
});
