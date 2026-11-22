// The workflow policy: the local tools (README.md in this directory). The quick checks before a
// push and what each change calls for, the limits every local build, test and probe runs under on
// the laptop, and the hook that refuses a raw build.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { CJK_RANGES, badSubject, baseChangedSincePush, cjkLines, problems, pullRequestOf } from "../lib/commits.mjs";
import { CARGO_HARMLESS, decide as rawBuildDecide, message as rawBuildMessage, refusal as rawBuildRefusal } from "../../../tools/raw-build.mjs";
import { blockString, bodyRefusal, decide as bodyDecide, graphqlTexts, requestPath } from "../../../tools/pr-body-guard.mjs";
import { SYSTEM_POWERSHELL, storePowerShell, withoutStoreApps } from "../../../tools/store-apps.mjs";
import { holds, jsImports, jsTokens } from "./js.mjs";
import { GATE_FILES, QUICK_MAP, owns } from "./pins.mjs";
import { read, strings, tracked } from "./repo.mjs";

/// Every check of this module counts itself when it runs, and the module fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// each module's FLOOR equal to the checks it declares.
const FLOOR = 6;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks of local-tools.test.mjs ran: a skip or a filter left the rest out`);
});
// ------------------------------------------------------------------------- the required checks

check("tools/quick.mjs runs this policy before a push for a change to any file, since the policy reads files across the whole tree", () => {
  // PR 161: a change to the benchmark's harness alone failed this policy in CI, where tools/quick.mjs
  // had run it only for the gates' files. The policy reads far more than the gates: every module a
  // job that holds a secret runs, the compiler's sources, the conformance programs, the corpus, the
  // spec and the docs, read on 2026-11-16 as some 2,300 paths in every part of the tree. The one
  // trigger that cannot fall behind what it reads is every path, tracked or new.
  // The map is data, read here and never run (the quick check of d471f7e: imported, it could have
  // turned this policy's assertions off).
  const map = JSON.parse(read(QUICK_MAP));
  const policy = map.checks.filter((c) => Array.isArray(c.run) && c.run.join(" ") === "npm run test:ci");
  assert.equal(policy.length, 1, `${QUICK_MAP} runs the workflow policy in one check`);
  const source = Object.hasOwn(map.patterns, policy[0].when) ? map.patterns[policy[0].when] : null;
  assert.equal(typeof source, "string", `${QUICK_MAP}: the workflow policy's check names no pattern the map defines`);
  const when = new RegExp(source);
  const missed = [...tracked(), "a-new-directory/a-new-file.txt", "a-new-file"].filter((f) => !when.test(f));
  assert.deepEqual(missed.slice(0, 20), [], `${missed.length} files whose change tools/quick.mjs would not check against this policy`);
});

/// The local limits' files: the module that defines them, the job object's script and the type it
/// shares with CI, the entry points, cargo's runner, the raw-build hook, and their proofs
/// (tools/local-limits.mjs).
const LOCAL_LIMITS = ["tools/local-limits.mjs", "tools/capped.ps1", "tools/capped.mjs", "tools/cargo.mjs", "tools/mn.mjs", "tools/raw-build.mjs", "tools/raw-build-guard.mjs", "tools/store-apps.mjs", "tools/local-limits.test.mjs", ".github/scripts/lib/job-object.cs", ".github/scripts/job-limits.ps1"];
/// The most the local limits may give: builds at once, GB committed by a process and by a tree, and
/// minutes, those --minutes may ask for among them.
const LOCAL_CEILING = { slots: 2, processGB: 8, jobGB: 16, minutes: 60 };

/// The arguments of each call of `name` in `toks`, each argument its tokens, split at the commas of
/// the call's own depth.
function callArguments(toks, name) {
  const calls = [];
  for (let i = 0; i < toks.length; i++) {
    if (toks[i].t !== "id" || toks[i].v !== name || toks[i + 1]?.v !== "(" || toks[i - 1]?.v === ".") continue;
    const args = [[]];
    let depth = 0;
    for (let j = i + 2; j < toks.length; j++) {
      const v = toks[j].v;
      if (depth === 0 && v === ")") break;
      if (depth === 0 && v === ",") {
        args.push([]);
        continue;
      }
      if (["(", "[", "{"].includes(v)) depth++;
      if ([")", "]", "}"].includes(v)) depth--;
      args.at(-1).push(toks[j]);
    }
    calls.push(args);
  }
  return calls;
}

/// What keeps the quick checks from running every command in a build slot and under the local
/// limits, from the texts of tools/quick.mjs (`quick`), tools/local-limits.mjs (`limits`),
/// tools/capped.ps1 (`capped`), tools/mn.mjs (`mn`) and tools/cargo.mjs (`cargo`), and the quick
/// map (`map`):
/// - quick.mjs starts no process but git's and runCapped's: it imports nothing else that could, it
///   calls execFileSync only on "git", it takes a slot, and runs each check by runCapped with BUILD;
/// - local-limits.mjs gives no more than LOCAL_CEILING, --minutes included; it starts capped.ps1 by
///   Windows PowerShell's path in System32, never pwsh, and hands it the command as JSON in
///   MENTE_CAPPED, never as arguments (the review of 8f03074); it starts one process, and a command
///   untouched only off Windows or, with menteOnly, outside a checkout of this repository;
/// - capped.ps1 reads its call from MENTE_CAPPED alone, and starts the command only through the job
///   object's Run, after Confine;
/// - mn.mjs and cargo.mjs hold their slot from before their first run to after their last;
/// - quick.mjs, cargo.mjs and mn.mjs require cargo's runner to be tools/capped.mjs before they take
///   a slot, as local-limits.mjs's requireRunner reads it (the review of round 4);
/// - the quick map runs the proofs for a change to any file of LOCAL_LIMITS.
export function localLimitsProblems({ quick, limits, capped, mn, cargo, map }) {
  const problems = [];
  const q = jsTokens(quick);
  const allowed = new Set(["node:child_process", "node:fs", "./local-limits.mjs"]);
  for (const s of jsImports(q).specifiers) if (!allowed.has(s.v)) problems.push(`tools/quick.mjs imports ${s.v}, which could start a process past the limits`);
  const starters = ["spawn", "spawnSync", "exec", "execSync", "execFile", "fork"];
  for (const t of q) if (t.t === "id" && starters.includes(t.v)) problems.push(`tools/quick.mjs:${t.line}: ${t.v} starts a process past the limits`);
  for (const args of callArguments(q, "execFileSync")) {
    if (!(args[0]?.length === 1 && args[0][0].t === "str" && args[0][0].v === "git")) problems.push("tools/quick.mjs calls execFileSync on something other than git");
  }
  if (q.some((t, i) => t.t === "id" && t.v === "execFileSync" && !["(", "}", ","].includes(q[i + 1]?.v))) problems.push("tools/quick.mjs passes execFileSync around, where no reading follows it");
  if (callArguments(q, "acquireSlot").length !== 1) problems.push("tools/quick.mjs takes no build slot, or more than one");
  const runs = callArguments(q, "runCapped");
  if (runs.length === 0) problems.push("tools/quick.mjs runs no check through runCapped");
  for (const args of runs) {
    if (!(args[1]?.length === 1 && args[1][0].v === "BUILD")) problems.push("tools/quick.mjs runs a check under other limits than BUILD");
  }

  const l = jsTokens(limits);
  const number = (name, key) => {
    const at = l.findIndex((t, i) => t.t === "id" && t.v === name && l[i + 1]?.v === "=");
    if (at < 0) return NaN;
    if (key === null) return l[at + 2]?.t === "num" ? Number(l[at + 2].v) : NaN;
    for (let j = at; j < Math.min(l.length, at + 30); j++) if (l[j].t === "id" && l[j].v === key && l[j + 1]?.v === ":" && l[j + 2]?.t === "num") return Number(l[j + 2].v);
    return NaN;
  };
  const slots = number("SLOTS", null);
  if (!(slots >= 1 && slots <= LOCAL_CEILING.slots)) problems.push(`tools/local-limits.mjs lets ${slots} builds run at once, past ${LOCAL_CEILING.slots}`);
  for (const set of ["BUILD", "PROBE"]) {
    for (const key of ["processGB", "jobGB", "minutes"]) {
      const v = number(set, key);
      if (!(v > 0 && v <= LOCAL_CEILING[key])) problems.push(`tools/local-limits.mjs gives ${set}.${key} ${v}, past ${LOCAL_CEILING[key]}`);
    }
  }
  const most = number("MAX_MINUTES", null);
  if (!(most > 0 && most <= LOCAL_CEILING.minutes)) problems.push(`tools/local-limits.mjs lets --minutes ask for ${most}, past ${LOCAL_CEILING.minutes}`);
  if (!holds(l, ["minutes", "<", "=", "MAX_MINUTES"])) problems.push("tools/local-limits.mjs does not hold --minutes to MAX_MINUTES");
  if (l.some((t) => /pwsh/i.test(t.v))) problems.push("tools/local-limits.mjs names pwsh, which takes the processes it starts out of the job");
  const strings = new Set(l.filter((t) => t.t === "str").map((t) => t.v));
  for (const s of ["System32", "WindowsPowerShell", "v1.0", "powershell.exe", "capped.ps1", "-File"]) if (!strings.has(s)) problems.push(`tools/local-limits.mjs does not name ${s} in the way it starts capped.ps1`);
  if (!holds(l, ["MENTE_CAPPED", ":", "JSON", ".", "stringify", "(", "call", ")"]) || !holds(l, ["command", ":", "run"])) problems.push("tools/local-limits.mjs does not hand capped.ps1 the command as JSON in MENTE_CAPPED");
  const at = l.findIndex((t, i) => t.v === "args" && l[i + 1]?.v === ":" && l[i + 2]?.v === "[");
  let close = at;
  while (close >= 0 && close < l.length && l[close].v !== "]") close++;
  if (at < 0 || l.slice(at + 3, close).some((t) => !(t.t === "str" || t.v === "CAPPED" || t.v === ","))) problems.push("tools/local-limits.mjs hands capped.ps1 arguments past its own options, which PowerShell's parser would change");
  // runCapped's process, and the gates' refresh entry (refreshGates), which the tools run before
  // their slot, by this node, with the gates' environment, under the refresher's own time limit
  // (briefs/gate-tools-from-main.md, "Who writes it").
  const spawns = callArguments(l, "spawnSync").map((args) => args.map((arg) => (arg ?? []).map((t) => t.v).join("")));
  const bounded = spawns.filter((args) => args[0] === "spawned.cmd" && args[1] === "spawned.args");
  const refreshed = spawns.filter((args) => args[0] === "process.execPath" && args[1] === "[entry]" && /env:gateEnv\(\)/.test(args[2] ?? ""));
  if (spawns.length !== 2 || bounded.length !== 1 || refreshed.length !== 1) problems.push("tools/local-limits.mjs starts a process other than the one runCapped chose and the gates' refresh entry");
  if (!holds(l, ["const", "{", "cmd", ",", "args", ",", "env", "}", "=", "capped", "(", "run", ",", "limits", ")"])) problems.push("tools/local-limits.mjs runs a command other than capped() gives");
  // The Store's pwsh, started in a job, takes every process it starts out of it (Norman,
  // 2026-11-22): a capped run refuses it as its program, and its PATH holds no WindowsApps
  // directory, where it is found by name.
  if (!jsImports(l).specifiers.some((s) => s.v === "./store-apps.mjs")) problems.push("tools/local-limits.mjs does not take the Store's apps from tools/store-apps.mjs");
  if (!holds(l, ["if", "(", "storePowerShell", "(", "run", "[", "0", "]", ")", ")"])) problems.push("tools/local-limits.mjs does not refuse the Store's pwsh as a capped command");
  if (!holds(l, ["env", ":", "{", ".", ".", ".", "withoutStoreApps", "(", "process", ".", "env", ")"])) problems.push("tools/local-limits.mjs leaves the WindowsApps directories on a capped command's PATH");
  const untouched = [...limits.matchAll(/spawned = \{ cmd: run\[0\]/g)].length;
  const gated = [...limits.matchAll(/(?:if|else if) \(([^\n]*)\) \{\n(?:\s*console\.error\([^\n]*\);\n)?\s*spawned = \{ cmd: run\[0\]/g)].map((m) => m[1]);
  if (gated.length !== untouched || gated.some((c) => !["menteOnly && !inMente(cwd ?? process.cwd())", "!WINDOWS"].includes(c))) problems.push("tools/local-limits.mjs runs a command untouched where it should bound it");

  const code = capped.split(/\r?\n/).filter((line) => !/^\s*#/.test(line)).join("\n");
  const confine = code.indexOf("[JobLimits]::Confine(");
  const run = code.indexOf("[JobLimits]::Run(");
  if (!/Add-Type -Path \(Join-Path \$PSScriptRoot '\.\.\/\.github\/scripts\/lib\/job-object\.cs'\)/.test(code)) problems.push("tools/capped.ps1 does not load the job object CI's Windows jobs use");
  if (confine < 0 || run < 0 || run < confine) problems.push("tools/capped.ps1 does not start the command in the job, after setting the job up");
  if (/Start-Process|Diagnostics\.Process\]::Start|Invoke-Expression|\biex\b|&\s*\$|pwsh/i.test(code)) problems.push("tools/capped.ps1 starts a process outside the job object's Run");
  if (!code.includes("$env:MENTE_CAPPED") || /\$args\b/i.test(code)) problems.push("tools/capped.ps1 reads its call from somewhere other than MENTE_CAPPED");

  for (const [file, text] of [["tools/quick.mjs", quick], ["tools/mn.mjs", mn], ["tools/cargo.mjs", cargo]]) {
    const t = jsTokens(text);
    const first = (name) => t.findIndex((x, i) => x.t === "id" && x.v === name && t[i + 1]?.v === "(" && t[i - 1]?.v !== ".");
    const required = first("requireRunner");
    if (required < 0 || required > first("acquireSlot")) problems.push(`${file} does not require cargo's runner before it takes a slot`);
    // The runner line and the hooks name the gates' stable entries, which a first refresh writes
    // (briefs/gate-tools-from-main.md, "Who writes it").
    const refreshed = first("refreshGates");
    if (refreshed < 0 || refreshed > required) problems.push(`${file} does not refresh the gates before it requires cargo's runner`);
  }
  for (const s of ["capped.mjs", "x86_64-pc-windows-msvc", "config.toml"]) if (!strings.has(s)) problems.push(`tools/local-limits.mjs does not name ${s} in the runner it requires`);
  // The line it tells the laptop to add, and what it takes for that runner, both with --mente-only.
  if (!/^export const runnerLine = [^\n]*"capped\.mjs"[^\n]*, "--mente-only", "--"\]`;$/m.test(limits)) problems.push("tools/local-limits.mjs tells the laptop to add a runner line other than tools/capped.mjs --mente-only --");
  if (!holds(l, ["words", "[", "2", "]", "=", "=", "=", "--mente-only"]) || !holds(l, ["words", "[", "3", "]", "=", "=", "=", "--"])) problems.push("tools/local-limits.mjs takes a runner without --mente-only -- for the capped one");
  for (const [file, text] of [["tools/mn.mjs", mn], ["tools/cargo.mjs", cargo]]) {
    const t = jsTokens(text);
    const calls = (name, dotted) => t.map((x, i) => (x.t === "id" && x.v === name && t[i + 1]?.v === "(" && (t[i - 1]?.v === ".") === dotted ? i : -1)).filter((i) => i >= 0);
    const slot = calls("acquireSlot", false);
    const ran = calls("runCapped", false);
    const released = calls("release", true);
    if (slot.length !== 1 || ran.length === 0 || slot[0] > ran[0] || released.length === 0 || released.some((r) => r < ran.at(-1))) problems.push(`${file} does not hold its slot from before its first run to after its last`);
  }

  const proofs = (map.checks ?? []).filter((c) => Array.isArray(c.run) && c.run.join(" ") === "node --test tools/local-limits.test.mjs");
  const when = proofs.map((c) => (Object.hasOwn(map.patterns ?? {}, c.when) && typeof map.patterns[c.when] === "string" ? new RegExp(map.patterns[c.when]) : null)).filter(Boolean);
  for (const f of LOCAL_LIMITS) if (!when.some((w) => w.test(f))) problems.push(`no quick check runs the local limits' proofs for a change to ${f}`);
  return problems;
}

check("the quick checks run every command in one of the machine's build slots, under the local limits", () => {
  // On 2026-11-22 a test binary committed 107 GB on the laptop, where several sessions build and
  // test at once, and the machine reset: nothing bounded a local process. tools/quick.mjs runs each
  // check in a Windows job object that bounds each process's memory, the tree's and the time, in one
  // of two build slots for the machine (tools/local-limits.mjs). Read as text, never run.
  const texts = { quick: read("tools/quick.mjs"), limits: read("tools/local-limits.mjs"), capped: read("tools/capped.ps1"), mn: read("tools/mn.mjs"), cargo: read("tools/cargo.mjs"), map: JSON.parse(read(QUICK_MAP)) };
  for (const f of LOCAL_LIMITS) assert.ok(tracked().has(f), `${f} is not in the repository`);
  // Each is a gate file, so that no edit lifts a limit without its owner's review (the review of
  // 8f03074's round: cargo's runner, tools/capped.mjs, came in without an owner).
  for (const f of LOCAL_LIMITS) assert.ok(GATE_FILES.some((p) => owns(p, f)), `${f}, one of the local limits, has no owner in .github/CODEOWNERS`);
  assert.deepEqual(localLimitsProblems(texts), []);
  // Planted, one at a time: each must be refused.
  const plants = [
    ["quick", 'import { execFileSync } from "node:child_process";', 'import { execFileSync, spawnSync } from "node:child_process";', "a direct spawnSync"],
    ["quick", "  results.push([check.name, runCapped(check.run, BUILD) === 0]);", "  results.push([check.name, runCapped(check.run, { processGB: 64, jobGB: 64, minutes: 600 }) === 0]);", "other limits"],
    ["quick", "const slot = acquireSlot(`tools/quick.mjs: ${due.length} check(s)`);", "const slot = { release() {} };", "no slot"],
    ["quick", 'execFileSync("git", args', 'execFileSync("cargo", args', "execFileSync on cargo"],
    ["quick", 'import { BUILD, acquireSlot, requireRunner, runCapped } from "./local-limits.mjs";', 'import { BUILD, acquireSlot, requireRunner, runCapped } from "./local-limits.mjs";\nimport { run } from "./other.mjs";', "another module"],
    ["limits", "export const SLOTS = 2;", "export const SLOTS = 4;", "four slots"],
    ["limits", "export const BUILD = Object.freeze({ processGB: 8,", "export const BUILD = Object.freeze({ processGB: 64,", "64 GB a process"],
    ["limits", "export const PROBE = Object.freeze({ processGB: 4, jobGB: 8, minutes: 5 });", "export const PROBE = Object.freeze({ processGB: 4, jobGB: 80, minutes: 5 });", "80 GB a probe's tree"],
    ["limits", "export const MAX_MINUTES = 60;", "export const MAX_MINUTES = 600;", "600 minutes"],
    ["limits", "  if (!(minutes > 0 && minutes <= MAX_MINUTES)) {", "  if (!(minutes > 0)) {", "--minutes with no ceiling"],
    ["limits", 'export const WINDOWS_POWERSHELL = path.join(process.env.SystemRoot || "C:\\\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe");', 'export const WINDOWS_POWERSHELL = "pwsh";', "the Store's pwsh"],
    ["limits", '"-File", CAPPED]', '"-File", CAPPED, ...run]', "the command as arguments"],
    ["limits", "  const { cmd, args, env } = capped(run, limits);", "  const { cmd, args, env } = { cmd: run[0], args: run.slice(1), env: {} };", "no capped()"],
    ["limits", "  if (menteOnly && !inMente(cwd ?? process.cwd())) {", "  if (true) {", "always untouched"],
    ["limits", "    if (storePowerShell(run[0])) {", "    if (false) {", "the Store's pwsh let through"],
    ["limits", "env: { ...withoutStoreApps(process.env),", "env: { ...process.env,", "the WindowsApps directories left on PATH"],
    ["limits", "  const r = spawnSync(spawned.cmd, spawned.args, { stdio: \"inherit\", env: spawned.env, cwd });", "  spawnSync(run[0], run.slice(1));\n  const r = spawnSync(spawned.cmd, spawned.args, { stdio: \"inherit\", env: spawned.env, cwd });", "a second process"],
    ["capped", "$status = [JobLimits]::Run($job, $application, $line, $milliseconds, [int]$call.parent, [ref] $end)", "$status = (Start-Process $application -PassThru -Wait).ExitCode", "Start-Process"],
    ["capped", "$status = [JobLimits]::Run($job, $application, $line, $milliseconds, [int]$call.parent, [ref] $end)", "& $program @rest; $status = $LASTEXITCODE", "the call operator"],
    ["capped", "Add-Type -Path (Join-Path $PSScriptRoot '../.github/scripts/lib/job-object.cs')", "Add-Type -Path (Join-Path $PSScriptRoot 'other.cs')", "another job object"],
    ["capped", "  $call = $env:MENTE_CAPPED | ConvertFrom-Json", "  $call = $args | ConvertFrom-Json", "the call from its arguments"],
    ["mn", 'let status = runCapped(["cargo", "build", "--quiet", "--bin", "mn"], BUILD, { cwd: ROOT });', 'let status = runCapped(["cargo", "build", "--quiet", "--bin", "mn"], BUILD, { cwd: ROOT });\nslot.release();', "the probe's slot let go before its run"],
    ["cargo", 'const slot = acquireSlot(`cargo ${args.join(" ")}`);', "const slot = { release() {} };", "cargo with no slot"],
    ["quick", "if (!requireRunner({ fallback })) process.exit(2);\n", "", "quick.mjs with no runner required"],
    ["cargo", "if (!requireRunner({ fallback })) process.exit(2);\n", "", "cargo.mjs with no runner required"],
    ["mn", "if (!requireRunner({ fallback })) process.exit(2);\n", "", "mn.mjs with no runner required"],
    ["quick", "const gates = refreshGates({ cwd: process.cwd() });\n", "const gates = \"copy\";\n", "quick.mjs with no refresh of the gates"],
    ["cargo", "const gates = refreshGates({ cwd: ROOT });\n", "const gates = \"copy\";\n", "cargo.mjs with no refresh of the gates"],
    ["mn", "const gates = refreshGates({ cwd: ROOT });\n", "if (!requireRunner()) process.exit(2);\nconst gates = refreshGates({ cwd: ROOT });\n", "mn.mjs requiring the runner before it refreshes the gates"],
    ["limits", '"--mente-only", "--"]`;\n\n/// The runner a configuration file', '"--"]`;\n\n/// The runner a configuration file', "a runner line without --mente-only"],
    ["limits", "const r = spawnSync(process.execPath, [entry], { stdio: \"inherit\", env: gateEnv(), cwd, windowsHide: true });", "const r = spawnSync(process.execPath, [entry], { stdio: \"inherit\", env: process.env, cwd, windowsHide: true });", "the refresh entry with this process's environment"],
    ["limits", "const r = spawnSync(process.execPath, [entry], { stdio: \"inherit\", env: gateEnv(), cwd, windowsHide: true });", "const r = spawnSync(process.execPath, [entry], { stdio: \"inherit\", env: gateEnv(), cwd, windowsHide: true });\n  spawnSync(\"git\", [\"gc\"]);", "a third process"],
  ];
  for (const [part, from, to, what] of plants) {
    assert.equal(texts[part].split(from).length, 2, `the plant "${what}" finds no place in ${part}`);
    assert.ok(localLimitsProblems({ ...texts, [part]: texts[part].replace(from, () => to) }).length > 0, `the plant "${what}" passed`);
  }
  // What the Store's pwsh is, and the PATH a capped command gets.
  for (const p of ["pwsh", "PWSH.EXE", "C:\\Users\\x\\AppData\\Local\\Microsoft\\WindowsApps\\pwsh.exe", "C:\\Program Files\\WindowsApps\\Microsoft.PowerShell_7.7.0.0_x64__8wekyb3d8bbwe\\pwsh.exe", "C:/Users/x/AppData/Local/Microsoft/WindowsApps/winget.exe"]) assert.ok(storePowerShell(p), `${p} was not taken for a Store app`);
  for (const p of [SYSTEM_POWERSHELL, "powershell", "cargo", "C:\\Windows\\System32\\cmd.exe", "C:\\tools\\mywindowsappsdemo\\x.exe"]) assert.ok(!storePowerShell(p), `${p} was taken for a Store app`);
  const trimmed = withoutStoreApps({ Path: "C:\\Windows\\System32;C:\\Users\\x\\AppData\\Local\\Microsoft\\WindowsApps;C:\\Users\\x\\.cargo\\bin;C:\\Program Files\\WindowsApps\\Microsoft.PowerShell_7.7.0.0_x64__8wekyb3d8bbwe\\;D:\\x\\WINDOWSAPPS\\", OTHER: "kept" });
  assert.deepEqual(trimmed, { Path: "C:\\Windows\\System32;C:\\Users\\x\\.cargo\\bin", OTHER: "kept" });
  const noProofs = { ...texts.map, checks: texts.map.checks.filter((c) => c.run.join(" ") !== "node --test tools/local-limits.test.mjs") };
  assert.ok(localLimitsProblems({ ...texts, map: noProofs }).length > 0, "a map that runs no proof of the local limits passed");
});

/// The project's settings, and the command they run the guard by.
const GUARD_SETTINGS = ".claude/settings.json";
// `|| exit 2`: a guard that fails to load exits 1, which Claude Code lets pass; this refuses.
const GUARD_COMMAND = 'node "$CLAUDE_PROJECT_DIR/tools/raw-build-guard.mjs" || exit 2';
/// The push guard's command, which refuses a push of a tree tools/quick.mjs has not passed
/// (tools/push.mjs; Norman approved it, 2026-11-22).
const PUSH_GUARD_COMMAND = 'node "$CLAUDE_PROJECT_DIR/tools/push-guard.mjs" || exit 2';
/// The tools whose commands both guards read before they run.
const GUARDED_TOOLS = ["Bash", "PowerShell", "Monitor", "mcp__terminal__run_in_terminal"];

/// What keeps `settings`, the project's Claude Code settings as parsed, from running each guard,
/// the raw-build guard and the push guard, before every command of a tool that runs one: Bash,
/// PowerShell, Monitor and the terminal's run_in_terminal (the second review of #203). A PreToolUse
/// hook whose matcher takes each tool's name, with the guard's command among its hooks.
export function guardSettingsProblems(settings) {
  const takes = (matcher, name) => {
    try {
      return new RegExp(`^(?:${matcher})$`).test(name);
    } catch {
      return false;
    }
  };
  const before = Array.isArray(settings?.hooks?.PreToolUse) ? settings.hooks.PreToolUse : [];
  const problems = [];
  for (const command of [GUARD_COMMAND, PUSH_GUARD_COMMAND]) {
    for (const tool of GUARDED_TOOLS) {
      const runs = before.some((h) => typeof h?.matcher === "string" && takes(h.matcher, tool) && Array.isArray(h.hooks) && h.hooks.some((x) => x?.type === "command" && x.command === command));
      if (!runs) problems.push(`${GUARD_SETTINGS} does not run \`${command}\` before each ${tool} command`);
    }
  }
  return problems;
}

check("the raw-build guard refuses a build outside the local limits and names the capped command to run instead", () => {
  // The hook a session runs before each shell command (tools/raw-build-guard.mjs, which
  // tools/raw-build.mjs decides for): a raw `cargo test`, or a binary run from target/, would run
  // past the job object and the slots, as the test binary of 2026-11-22 did. The refusal names the
  // command to run instead, as a diagnostic names its fix. Each case: the command, and what the
  // guard must name instead, or null to let it through.
  const cases = [
    ["cargo test", "node tools/cargo.mjs test"],
    ["cargo test -p mente-cli", "node tools/cargo.mjs test -p mente-cli"],
    ["cd compiler && cargo build --release", "node tools/cargo.mjs build --release"],
    ["CARGO_BUILD_JOBS=4 cargo t", "node tools/cargo.mjs t"],
    ["cargo +nightly run --bin mn -- spec", "node tools/cargo.mjs +nightly run --bin mn -- spec"],
    ["& cargo clippy --all-targets", "node tools/cargo.mjs clippy --all-targets"],
    ['cargo test -p x -- "a b" && echo done', 'node tools/cargo.mjs test -p x -- "a b"'],
    ["if true; then cargo test; fi", "node tools/cargo.mjs test"],
    ["C:/Users/Dancr/.cargo/bin/cargo.exe check", "node tools/cargo.mjs check"],
    ["target/debug/mn.exe check x.mn", "node tools/mn.mjs check x.mn"],
    ["C:\\Repos\\Mente\\target\\release\\mn.exe run a.mn", "node tools/mn.mjs run a.mn"],
    ["./target/debug/deps/mn-0e02cc27629dbce7.exe --list", "node tools/cargo.mjs test -- --list"],
    ["node tools/cargo.mjs test -p mente-cli", null],
    ["node tools/mn.mjs check examples/a.mn", null],
    ["node tools/quick.mjs -- -p mente-cli x", null],
    ["cargo fmt --all --check", null],
    ["cargo metadata --format-version 1", null],
    ['git commit -m "run cargo test before"', null],
    ['git commit -m "fix the floor; cargo test passes"', null],
    ["ls target/debug/", null],
    ["grep -r cargo test.txt", null],
    ["echo cargo test", null],
    // The review of 8f03074: forms the regular expressions missed, each now read as a command.
    ["cmd /c cargo test", "node tools/cargo.mjs test"],
    ["cargo -q test", "node tools/cargo.mjs -q test"],
    ["cargo --locked test", "node tools/cargo.mjs --locked test"],
    ["foreach ($x in 1..2) { cargo test }", "node tools/cargo.mjs test"],
    ["timeout 600 cargo test", "node tools/cargo.mjs test"],
    ["env cargo test", "node tools/cargo.mjs test"],
    ["Cargo test", "node tools/cargo.mjs test"],
    ["cargo llvm-cov", "node tools/cargo.mjs llvm-cov"],
    ["cargo fix", "node tools/cargo.mjs fix"],
    ['bash -c "cargo build"', "node tools/cargo.mjs build"],
    ["cargo test 2>&1 | tail -5", "node tools/cargo.mjs test 2>&1"],
    // A cargo command not known as harmless is refused, so a new one is too.
    ["cargo testing-tool", "node tools/cargo.mjs testing-tool"],
    // And what the regular expressions refused wrongly: a heredoc's body and a comment.
    ["git commit -F - <<'EOF'\ncargo test passes now\nEOF", null],
    ["cat <<-EOF > notes.txt\n\tcargo build\n\tEOF", null],
    ["# then cargo test", null],
    ["echo done # cargo test", null],
    ["echo done # then; cargo test", null],
    ["cargo -q fmt --check", null],
    ["cargo --locked metadata --format-version 1", null],
    ["$m = @'\ncargo test\n'@\ngit commit -m $m", null],
    ["cargo --version", null],
    ["cargo", null],
    // tools/capped.mjs asked to run the Store's pwsh, by name or by path, through either separator,
    // gets Windows PowerShell in its place; its command is the word after its first `--`, and
    // cargo.mjs and mn.mjs run cargo and mn whatever their arguments name (the review of 55c40e7).
    ["node tools/capped.mjs -- pwsh -NoProfile -c x", `node tools/capped.mjs -- ${SYSTEM_POWERSHELL} -NoProfile -c x`],
    ["node tools\\capped.mjs -- pwsh -c x", `node tools\\capped.mjs -- ${SYSTEM_POWERSHELL} -c x`],
    ["node tools/capped.mjs --mente-only -- pwsh.exe", `node tools/capped.mjs --mente-only -- ${SYSTEM_POWERSHELL}`],
    ["node tools/capped.mjs -- C:\\Users\\x\\AppData\\Local\\Microsoft\\WindowsApps\\pwsh.exe -c 1", `node tools/capped.mjs -- ${SYSTEM_POWERSHELL} -c 1`],
    ["node tools/capped.mjs -- node probe.js pwsh", null],
    ["node tools/cargo.mjs test -- --skip pwsh", null],
    ["node tools/cargo.mjs run --bin probe -- pwsh", null],
    ["node tools/capped.mjs -- powershell.exe -c x", null],
    ["node tools/mn.mjs check a.mn", null],
    ["pwsh -c 1", null],
  ];
  const wrong = cases.filter(([command, instead]) => (rawBuildRefusal(command)?.instead ?? null) !== instead).map(([command, instead]) => `${command}: ${rawBuildRefusal(command)?.instead ?? null}, not ${instead}`);
  assert.deepEqual(wrong, []);
  // The words a session reads name the command found and the one to run.
  const planted = rawBuildRefusal("cargo test");
  assert.match(rawBuildMessage(planted), /^"cargo test" would run outside the local limits\. Run this instead: node tools\/cargo\.mjs test\n/);
  for (const name of ["build", "test", "run", "clippy", "nextest", "check", "llvm-cov", "fix", "bench", "doc", "mutants"]) assert.ok(!CARGO_HARMLESS.includes(name), `the guard lets cargo ${name} through`);
  // The hook fails closed: an event it cannot read, or one with no command, refuses the command.
  for (const input of ["not json", "", '{"tool_input":{"command":42}}', "{}"]) {
    const answer = rawBuildDecide(input);
    assert.equal(answer.status, 2, `the hook lets a command through on ${JSON.stringify(input)}`);
    assert.match(answer.words, /^the raw-build guard /);
  }
  assert.deepEqual(rawBuildDecide(JSON.stringify({ tool_input: { command: "node tools/cargo.mjs test" } })), { status: 0, words: "" });
  assert.deepEqual(rawBuildDecide(JSON.stringify({ tool_name: "Monitor", tool_input: { ws: { url: "wss://example.invalid" } } })), { status: 0, words: "" }, "a Monitor of a WebSocket runs no command");
  assert.equal(rawBuildDecide(JSON.stringify({ tool_name: "mcp__terminal__run_in_terminal", tool_input: { command: "cargo test", cwd: "C:/x" } })).status, 2);
  assert.equal(rawBuildDecide(JSON.stringify({ tool_name: "Monitor", tool_input: {} })).status, 2, "an event with neither command nor WebSocket refuses");
  assert.equal(rawBuildDecide(JSON.stringify({ tool_input: { command: "cargo test" } })).status, 2);
  // The project's settings, which every checkout and every session of it reads, run the guard
  // before each Bash and PowerShell command (Norman, 2026-11-22).
  const settings = JSON.parse(read(GUARD_SETTINGS));
  assert.deepEqual(guardSettingsProblems(settings), []);
  const hook = (matcher, command) => ({ hooks: { PreToolUse: [{ matcher, hooks: [{ type: "command", command }] }] } });
  const ours = settings.hooks.PreToolUse.find((h) => h.hooks?.some((x) => x.command === GUARD_COMMAND));
  for (const [plant, what] of [
    [hook("Bash", GUARD_COMMAND), "the guard for Bash alone"],
    [hook(ours.matcher, GUARD_COMMAND), "the raw-build guard without the push guard"],
    [hook(ours.matcher, PUSH_GUARD_COMMAND), "the push guard without the raw-build guard"],
    [{ hooks: { PreToolUse: [{ matcher: "Bash|PowerShell", hooks: ours.hooks }] } }, "both guards before Bash and PowerShell alone"],
    [hook(ours.matcher, 'node "$CLAUDE_PROJECT_DIR/tools/other.mjs"'), "another script"],
    [{ hooks: { PostToolUse: settings.hooks.PreToolUse } }, "the guard after the command, not before"],
    [{}, "no hook"],
  ]) {
    assert.ok(guardSettingsProblems(plant).length > 0, `settings with ${what} passed`);
  }
});

/// The program generator, its differential runs and their coverage: modules of mn's own binary.
const GENERATOR_DIR = "compiler/mente-cli/src/mir_run/";
const GENERATOR_RUN = ["cargo", "test", "-p", "mente-cli", "--bin", "mn", "--"];

/// What keeps the quick map (`map`, as parsed) from running the program generator's own tests for a
/// change to any of `files`, the tracked files of GENERATOR_DIR, with `main`, the text of mn's
/// main.rs: one check whose pattern takes every one of them runs `cargo test` on mn's binary with,
/// for each module a file of the directory is, a filter every test of that module's path contains,
/// and no option among them (`--exact` or `--skip` would turn the filters into something else); and
/// main.rs declares mir_run, so that the binary holds those tests at all. A filter that matched no
/// test would pass with none run.
export function generatorQuickProblems(map, files, main) {
  const problems = [];
  if (!/^mod mir_run;$/m.test(main)) problems.push("compiler/mente-cli/src/main.rs declares no `mod mir_run;`, so mn's binary holds none of the generator's tests");
  const modules = files.filter((f) => f.endsWith(".rs")).map((f) => `mir_run::${f.slice(GENERATOR_DIR.length, -".rs".length).replace(/\/mod$/, "").replace(/\//g, "::")}::`);
  const runs = (map.checks ?? []).filter((c) => Array.isArray(c.run) && GENERATOR_RUN.every((a, k) => c.run[k] === a));
  const covering = runs.filter((c) => {
    const source = Object.hasOwn(map.patterns ?? {}, c.when) ? map.patterns[c.when] : null;
    if (typeof source !== "string") return false;
    const when = new RegExp(source);
    const filters = c.run.slice(GENERATOR_RUN.length);
    return files.every((f) => when.test(f)) && !filters.some((a) => a.startsWith("-")) && modules.every((m) => filters.some((a) => m.includes(a)));
  });
  if (covering.length === 0) {
    problems.push(`no check of ${QUICK_MAP} runs \`${GENERATOR_RUN.join(" ")}\` with a filter for each of ${modules.join(", ")}, for a change to any file of ${GENERATOR_DIR}`);
  }
  return problems;
}

check("tools/quick.mjs runs the program generator's own tests for a change to it", () => {
  // A change to the program generator can break what only its own tests hold: every construct of
  // the grammar reached, every generated program well typed and replayable, the three paths
  // agreeing on them. No other quick check runs them, so such a change passed the quick checks and
  // failed only on CI (compiler-fixes, 2026-11-22). A module added to the directory needs its
  // filter in the map too.
  const files = [...tracked()].filter((f) => f.startsWith(GENERATOR_DIR));
  assert.ok(files.some((f) => f.endsWith("/generate.rs")), `${GENERATOR_DIR} holds no generate.rs: the generator moved, and this check with it`);
  const main = read("compiler/mente-cli/src/main.rs");
  const map = JSON.parse(read(QUICK_MAP));
  assert.deepEqual(generatorQuickProblems(map, files, main), []);
  // Planted: a module added to the directory, a pattern that misses a file, a filter turned exact,
  // one module's filter alone, the filters run on another test target, the check gone, and mir_run
  // no longer a module of the binary.
  const generator = map.checks.find((c) => generatorQuickProblems({ ...map, checks: [c] }, files, main).length === 0);
  const withRun = (run) => ({ ...map, checks: map.checks.map((c) => (c === generator ? { ...c, run } : c)) });
  assert.equal(generatorQuickProblems(map, [...files, `${GENERATOR_DIR}shrink.rs`], main).length, 1, "a module added to the generator's directory passed without its filter");
  assert.equal(generatorQuickProblems({ ...map, patterns: { ...map.patterns, [generator.when]: `^${GENERATOR_DIR}generate\\.rs$` } }, files, main).length, 1, "a pattern that misses a file of the directory passed");
  assert.equal(generatorQuickProblems(withRun([...generator.run, "--exact"]), files, main).length, 1, "filters turned exact passed");
  assert.equal(generatorQuickProblems(withRun([...GENERATOR_RUN, "mir_run::generate::"]), files, main).length, 1, "a run of one module's tests passed");
  assert.equal(generatorQuickProblems(withRun(["cargo", "test", "-p", "mente-cli", "--test", "cli", "--", ...generator.run.slice(GENERATOR_RUN.length)]), files, main).length, 1, "the filters run against a test target that holds none of the generator's tests passed");
  assert.equal(generatorQuickProblems({ ...map, checks: map.checks.filter((c) => c !== generator) }, files, main).length, 1, "a map without the generator's check passed");
  assert.equal(generatorQuickProblems(map, files, main.replace(/^mod mir_run;$/m, "")).length, 1, "mir_run outside mn's binary passed");
  assert.deepEqual(generatorQuickProblems(withRun([...GENERATOR_RUN, "mir_run::"]), files, main), [], "one filter that every module's tests contain was refused");
});

check("the session's hook refuses to send a pull request's text that fails the Commits check's rules, or one it cannot read", () => {
  // tools/pr-body-guard.mjs, which tools/push-guard.mjs runs before each shell command: a session
  // sends a pull request's text with the App's client, from a JSON file, and the hook reads that file
  // with requestProblems, as the Commits check reads the text on GitHub (the review of 5264a01: a
  // body sent unchecked met the rules only in the required check).
  const han = String.fromCodePoint(0x4e2d, 0x6587);
  const files = new Map([
    ["C:/s/good.json", JSON.stringify({ title: "T", body: "Not a fix: a brief." })],
    ["C:/s/bare.json", JSON.stringify({ title: "T", body: "No declaration." })],
    ["C:/s/title.json", JSON.stringify({ title: "Retitled" })],
    ["C:/s/cjk.json", JSON.stringify({ title: `T ${han}` })],
    ["C:/s/bom.json", `\uFEFF${JSON.stringify({ body: "Not a fix: a brief." })}`],
    ["C:/s/broken.json", "{"],
    ["/tmp/s/bare.json", JSON.stringify({ body: "No declaration." })],
    // GraphQL's mutations of a pull request: the text in its variables, inline as a string or a block
    // string, or in a variable the query names.
    ["C:/s/gql-vars.json", JSON.stringify({ query: "mutation($input: UpdatePullRequestInput!) { updatePullRequest(input: $input) { pullRequest { id } } }", variables: { input: { pullRequestId: "PR_1", body: "No declaration." } } })],
    ["C:/s/gql-inline.json", JSON.stringify({ query: 'mutation { updatePullRequest(input: {pullRequestId: "PR_1", body: "No declaration."}) { clientMutationId } }' })],
    ["C:/s/gql-block.json", JSON.stringify({ query: 'mutation { updatePullRequest(input: {pullRequestId: "PR_1", body: """No declaration."""}) { clientMutationId } }' })],
    ["C:/s/gql-named.json", JSON.stringify({ query: 'mutation($text: String!) { updatePullRequest(input: {pullRequestId: "PR_1", body: $text}) { clientMutationId } }', variables: { text: "No declaration." } })],
    ["C:/s/gql-create.json", JSON.stringify({ query: 'mutation { createPullRequest(input: {repositoryId: "R_1", baseRefName: "main", headRefName: "x", title: "T"}) { clientMutationId } }' })],
    ["C:/s/gql-cjk.json", JSON.stringify({ query: "mutation($input: UpdatePullRequestInput!) { updatePullRequest(input: $input) { clientMutationId } }", variables: { input: { pullRequestId: "PR_1", title: `T ${han}` } } })],
    ["C:/s/gql-unread.json", JSON.stringify({ query: 'mutation { updatePullRequest(input: {pullRequestId: "PR_1", body: SOME_ENUM}) { clientMutationId } }' })],
    ["C:/s/gql-good.json", JSON.stringify({ query: 'mutation($body: String!) { updatePullRequest(input: {pullRequestId: "PR_1", body: $body}) { pullRequest { body title } } }', variables: { body: "Not a fix: a brief." } })],
    ["C:/s/gql-title.json", JSON.stringify({ query: 'mutation { updatePullRequest(input: {pullRequestId: "PR_1", title: "Retitled"}) { clientMutationId } }' })],
    ["C:/s/gql-query.json", JSON.stringify({ query: "query { viewer { login } }", variables: { body: "No declaration." } })],
    // The review of e95f0ad: variables sent as a JSON-encoded string, and an indented block string.
    ["C:/s/gql-strvars.json", JSON.stringify({ query: 'mutation($b: String!) { updatePullRequest(input: {pullRequestId: "PR_1", body: $b}) { clientMutationId } }', variables: JSON.stringify({ b: "No declaration." }) })],
    ["C:/s/gql-indented.json", JSON.stringify({ query: 'mutation {\n  updatePullRequest(input: {pullRequestId: "PR_1", body: """\n      Not a fix: a brief.\n\n      More of it.\n    """}) { clientMutationId }\n}' })],
    ["C:/s/gql-indented-bare.json", JSON.stringify({ query: 'mutation {\n  updatePullRequest(input: {pullRequestId: "PR_1", body: """\n      No declaration.\n    """}) { clientMutationId }\n}' })],
  ]);
  const readFile = (f) => {
    if (!files.has(f)) throw Object.assign(new Error("no such file"), { code: "ENOENT" });
    return files.get(f);
  };
  const on = (platform) => (command) => bodyRefusal(command, { readFile, platform });
  const win = on("win32");
  const client = "node C:/Users/x/tools/github-app/gh-app.mjs api";
  // Each case: the command, and what the refusal must say, or null to let it through.
  const cases = [
    [`${client} PATCH repos/Menteion/Mente/pulls/214 --data-file C:/s/good.json`, null],
    [`${client} POST repos/Menteion/Mente/pulls --data-file C:/s/good.json --out C:/s/created.json`, null],
    [`${client} PATCH repos/Menteion/Mente/pulls/214 --data-file C:/s/bare.json`, /fails the Commits check's rules[\s\S]*- the pull request's body: it declares neither/],
    [`${client} patch /repos/menteion/MENTE/pulls/214/ --data-file C:/s/bare.json`, /declares neither/],
    [`cd C:/s && MSYS_NO_PATHCONV=1 node gh-app.mjs api PATCH repos/Menteion/Mente/pulls/1 --data-file /c/s/bare.json`, /not a drive's: name it as C:\/\.\.\./],
    [`bash -c "node gh-app.mjs api PATCH repos/Menteion/Mente/pulls/1 --data-file C:/s/bare.json"`, /declares neither/],
    [`pwsh -Command node gh-app.mjs api PATCH repos/Menteion/Mente/pulls/1 --data-file C:/s/bare.json`, /declares neither/],
    // An edit of the title alone is no edit of the body; a request that creates one is read whole.
    [`${client} PATCH repos/Menteion/Mente/pulls/214 --data-file C:/s/title.json`, null],
    [`${client} POST repos/Menteion/Mente/pulls --data-file C:/s/title.json`, /declares neither/],
    [`${client} PATCH repos/Menteion/Mente/pulls/214 --data-file C:/s/cjk.json`, /title: CJK text/],
    [`${client} PATCH repos/Menteion/Mente/pulls/214 --data-file C:/s/bom.json`, null],
    // A file it cannot name, read or parse refuses.
    [`S=C:/s; ${client} PATCH repos/Menteion/Mente/pulls/214 --data-file $S/bare.json`, /through the shell/],
    [`${client} PATCH repos/Menteion/Mente/pulls/214 --data-file ~/bare.json`, /through the shell/],
    [`${client} PATCH repos/Menteion/Mente/pulls/214 --data-file bare.json`, /not a drive.s/],
    [`${client} PATCH repos/Menteion/Mente/pulls/214 --data-file C:\\s\\bare.json`, /with a backslash/],
    [`${client} PATCH repos/Menteion/Mente/pulls/214 --data-file C:/s/missing.json`, /cannot read \(ENOENT\)/],
    [`${client} PATCH repos/Menteion/Mente/pulls/214 --data-file C:/s/broken.json`, /holds no JSON/],
    [`node build.mjs > C:/s/good.json && ${client} PATCH repos/Menteion/Mente/pulls/214 --data-file C:/s/good.json`, /elsewhere on the same command line/],
    // The route as the client builds its URL and the API reads it (the review of 3fc8633).
    [`${client} PATCH https://api.github.com/repos/Menteion/Mente/pulls/214 --data-file C:/s/bare.json`, /declares neither/],
    [`${client} PATCH repos/Menteion/Mente/pulls/214?x=1 --data-file C:/s/bare.json`, /declares neither/],
    [`${client} PATCH repos/Menteion/Mente/pulls/214#frag --data-file C:/s/bare.json`, /declares neither/],
    [`${client} PATCH repos/Menteion/Mente/./pulls/214 --data-file C:/s/bare.json`, /declares neither/],
    [`${client} PATCH repos/Menteion/x/../Mente/pulls/214 --data-file C:/s/bare.json`, /declares neither/],
    [`${client} PATCH repositories/1234/pulls/214 --data-file C:/s/bare.json`, /declares neither/],
    [`${client} POST repositories/1234/pulls --data-file C:/s/title.json`, /declares neither/],
    [`${client} PATCH https://api.github.com.example.com/repos/Menteion/Mente/pulls/214 --data-file C:/s/bare.json`, null],
    [`${client} PATCH C:/Program Files/Git/repos/Menteion/Mente/pulls/214 --data-file C:/s/bare.json`, null],
    // A word the shell may change, in a call that could send text, refuses.
    [`${client} PATCH $R --data-file C:/s/good.json`, /names a word of its call through the shell/],
    [`node gh-app.mjs api "$@" --data-file C:/s/good.json`, /names a word of its call through the shell/],
    [`node gh-app.mjs api $M repos/Menteion/Mente/pulls/1 --data-file C:/s/good.json`, /names a word of its call through the shell/],
    [`node gh-app.mjs api PATCH @route --data-file C:/s/good.json`, /names a word of its call through the shell/],
    ["node gh-app.mjs api PA`TCH repos/Menteion/Mente/pulls/1 --data-file C:/s/good.json", /names a word of its call through the shell/],
    [`node gh-app.mjs --% api PATCH repos/Menteion/Mente/pulls/1 --data-file C:/s/good.json`, /names a word of its call through the shell/],
    [`node gh-app.mjs api PATCH repos/Menteion/Mente/pulls/1 $D`, /names a word of its call through the shell/],
    [`node gh-app.mjs api PATCH repos/Menteion/Mente/pulls/%R% --data-file C:/s/good.json`, /names a word of its call through the shell/],
    [`${client} PATCH repos/Menteion/Mente/pulls/214 --data-file C:/s/good.json --out $S/out.json`, null],
    [`${client} GET repos/Menteion/Mente/pulls/$N --out $S/out.json`, null],
    // node's options, with a value or without, before the client or loading it.
    [`node --import ./x.mjs C:/Users/x/tools/github-app/gh-app.mjs api PATCH repos/Menteion/Mente/pulls/1 --data-file C:/s/bare.json`, /declares neither/],
    [`node -r x gh-app.mjs api PATCH repos/Menteion/Mente/pulls/1 --data-file C:/s/bare.json`, /declares neither/],
    [`node --no-warnings gh-app.mjs api PATCH repos/Menteion/Mente/pulls/1 --data-file C:/s/bare.json`, /declares neither/],
    [`node --import ./gh-app.mjs main.mjs api PATCH repos/Menteion/Mente/pulls/1 --data-file C:/s/bare.json`, /declares neither/],
    // GraphQL's mutations of a pull request, read for every title and body they send.
    [`${client} POST graphql --data-file C:/s/gql-vars.json`, /body: it declares neither/],
    [`${client} POST https://api.github.com/graphql --data-file C:/s/gql-inline.json`, /body: it declares neither/],
    [`${client} POST graphql --data-file C:/s/gql-block.json`, /body: it declares neither/],
    [`${client} POST graphql --data-file C:/s/gql-named.json`, /body: it declares neither/],
    [`${client} POST graphql --data-file C:/s/gql-create.json`, /body: it declares neither/],
    [`${client} POST graphql --data-file C:/s/gql-cjk.json`, /title: CJK text/],
    [`${client} POST graphql --data-file C:/s/gql-unread.json`, /names a pull request's body in a form this rule cannot read/],
    [`${client} POST graphql --data-file C:/s/gql-good.json`, null],
    [`${client} POST graphql --data-file C:/s/gql-title.json`, null],
    [`${client} POST graphql --data-file C:/s/gql-query.json`, null],
    [`${client} POST graphql --data-file C:/s/gql-strvars.json`, /sends its variables as no JSON object/],
    [`${client} POST graphql --data-file C:/s/gql-indented.json`, null],
    [`${client} POST graphql --data-file C:/s/gql-indented-bare.json`, /body: it declares neither/],
    // xargs, a PowerShell subexpression and bash's brace expansion give the client words from
    // elsewhere (the review of e95f0ad).
    [`xargs node gh-app.mjs < C:/s/args.txt`, /through xargs/],
    [`xargs -a C:/s/args.txt node C:/Users/x/tools/github-app/gh-app.mjs`, /through xargs/],
    [`cat C:/s/args.txt | xargs -n 9 node gh-app.mjs api GET repos/Menteion/Mente/pulls/1`, /through xargs/],
    [`git ls-files | xargs grep -l gh-app`, null],
    [`ls | xargs -a C:/s/args.txt node other.mjs`, null],
    [`node gh-app.mjs api PATCH (Get-Content C:/s/route.txt) --data-file C:/s/good.json`, /names a word of its call through the shell/],
    [`node gh-app.mjs (Get-Content C:/s/args.txt)`, /names a word of its call through the shell/],
    [`node gh-app.mjs api PATCH repos/Menteion/Mente/{pulls,x}/1 --data-file C:/s/good.json`, /names a word of its call through the shell/],
    [`node gh-app.mjs api PATCH "repos/Menteion/Mente/pulls/(1)" --data-file C:/s/good.json`, /names a word of its call through the shell/],
    [`${client} PATCH repos/Menteion/Mente/pulls/214 --data-file C:/s/good.json && (echo sent)`, null],
    [`${client} GET repos/Menteion/Mente/pulls/214 --out C:/s/out.json; { echo read; }`, null],
    // What sends no text of a pull request of this repository passes.
    [`${client} GET repos/Menteion/Mente/pulls/214 --out C:/s/bare.json`, null],
    [`${client} PATCH repos/Menteion/Mente/pulls/214`, null],
    [`${client} POST repos/Menteion/Mente/pulls/214/reviews --data-file C:/s/bare.json`, null],
    [`${client} PATCH repos/Menteion/Mente/pulls --data-file C:/s/bare.json`, null],
    [`${client} PATCH repos/Menteion/Menteion/pulls/3 --data-file C:/s/bare.json`, null],
    [`${client} PATCH repos/Menteion/Mente/issues/214 --data-file C:/s/bare.json`, null],
    [`echo node gh-app.mjs api PATCH repos/Menteion/Mente/pulls/1 --data-file C:/s/bare.json`, null],
    [`echo gh-app.mjs api PATCH repos/Menteion/Mente/pulls/1 --data-file C:/s/bare.json`, null],
    ["git status", null],
  ];
  for (const [command, says] of cases) {
    const why = win(command);
    if (says === null) assert.equal(why, null, `${command} was refused: ${why}`);
    else assert.match(why ?? "", says, `${command}: ${why}`);
  }
  // The path the client requests, as the API reads it.
  assert.equal(requestPath("repos/Menteion/Mente/pulls/214"), "/repos/menteion/mente/pulls/214");
  assert.equal(requestPath("/repos/Menteion/Mente/pulls/214/"), "/repos/menteion/mente/pulls/214/");
  assert.equal(requestPath("https://api.github.com/repos/Menteion/Mente/pulls/214?x=1#y"), "/repos/menteion/mente/pulls/214");
  assert.equal(requestPath("repos/Menteion/Mente/pull%73/214"), "/repos/menteion/mente/pulls/214", "an escape in the route was read as written");
  assert.equal(requestPath("repos/Menteion/Mente/pull%2573/214"), "/repos/menteion/mente/pulls/214", "an escape of an escape was read as written");
  assert.equal(requestPath("repos/Menteion/x/%2e%2e/Mente//pulls/214"), "/repos/menteion/mente/pulls/214");
  assert.equal(requestPath("repos/Menteion/x%2F..%2FMente/pulls/214"), "/repos/menteion/mente/pulls/214", "a slash escaped in a segment was read as written");
  assert.equal(requestPath("%2F%2Frepos/Menteion/Mente/pulls/214"), "/repos/menteion/mente/pulls/214", "escaped slashes before the path were read as a host");
  assert.equal(requestPath("https://api.github.com.example.com/repos/Menteion/Mente/pulls/214"), null);
  assert.equal(requestPath("http://api.github.com/repos/Menteion/Mente/pulls/214"), "/http:/api.github.com/repos/menteion/mente/pulls/214");
  assert.equal(requestPath("C:/Program Files/Git/repos"), null);
  // GraphQL: nothing of a request that names no mutation of a pull request.
  assert.deepEqual(graphqlTexts({ query: "query { viewer { login } }", variables: { body: "x" } }), { found: [] });
  assert.deepEqual(graphqlTexts({ query: 'mutation { updatePullRequest(input: {body: null, title: "a\\"b"}) { id } }' }), { creates: false, found: [["body", null], ["title", 'a"b']] });
  assert.deepEqual(graphqlTexts({ query: "mutation { updatePullRequest(input: {body: $missing}) { id } }" }), { creates: false, found: [] });
  assert.match(graphqlTexts({ query: "mutation($b: X) { updatePullRequest(input: {body: $b}) { id } }", variables: { b: { nested: "x" } } }).why ?? "", /cannot read/);
  assert.match(graphqlTexts({ query: 'mutation { updatePullRequest(input: {body: """unclosed}) { id } }' }).why ?? "", /cannot read/);
  assert.match(graphqlTexts({ query: 'mutation { updatePullRequest(input: {body: "unclosed}) { id } }' }).why ?? "", /cannot read/);
  for (const variables of ["{}", [], 7, true]) assert.match(graphqlTexts({ query: "mutation { updatePullRequest(input: {}) { id } }", variables }).why ?? "", /variables as no JSON object/, JSON.stringify(variables));
  assert.deepEqual(graphqlTexts({ query: "mutation { updatePullRequest(input: {}) { id } }", variables: null }), { creates: false, found: [] });
  // A block string's value, as GraphQL gives it: the common indent of its lines after the first
  // removed, its leading and trailing blank lines dropped, `\"""` unescaped.
  assert.equal(blockString("\n    Not a fix: a brief.\n\n      Indented more.\n    "), "Not a fix: a brief.\n\n  Indented more.");
  assert.equal(blockString("First line\n    second\n  third"), "First line\n  second\nthird");
  assert.equal(blockString('  \r\n\tA \\""" quote.\r\n\t'), 'A """ quote.');
  assert.equal(blockString("   "), "");
  // On Linux, an absolute path is read as written, and a relative one refuses.
  assert.match(on("linux")(`${client} PATCH repos/Menteion/Mente/pulls/1 --data-file /tmp/s/bare.json`) ?? "", /declares neither/);
  assert.match(on("linux")(`${client} PATCH repos/Menteion/Mente/pulls/1 --data-file s/bare.json`) ?? "", /relative path/);
  // The hook's answer: status 2 with the words, 0 to let it run; an event it cannot read refuses.
  const event = (command) => JSON.stringify({ tool_input: { command } });
  assert.equal(bodyDecide(event(cases[2][0]), { readFile, platform: "win32" }).status, 2);
  assert.match(bodyDecide(event(cases[2][0]), { readFile, platform: "win32" }).words, /^the pull request body guard refused it: /);
  assert.deepEqual(bodyDecide(event("git status"), { readFile, platform: "win32" }), { status: 0, words: "" });
  assert.equal(bodyDecide("{", { readFile }).status, 2);
  assert.equal(bodyDecide(JSON.stringify({ tool_input: {} }), { readFile }).status, 2);
  assert.equal(bodyDecide(JSON.stringify({ tool_input: { ws: "wss://x" } }), { readFile }).status, 0);
  // No text of the body reaches the words: only the rules' messages, which echo none.
  assert.doesNotMatch(win(`${client} PATCH repos/Menteion/Mente/pulls/214 --data-file C:/s/bare.json`), /No declaration\./);
});

/// What makes two checks of the quick map (`map`, as parsed) one: the same name, or the same
/// command. A merge that kept both sides of a check, as two pull requests that each add the same
/// one leave it to, would run its command twice, or show two lines under one name, and nothing
/// else would say so.
export function duplicateQuickProblems(map) {
  const problems = [];
  const seen = new Map();
  for (const c of map.checks ?? []) {
    for (const [what, key] of [
      ["name", `name ${JSON.stringify(c.name)}`],
      ["command", `command ${JSON.stringify(Array.isArray(c.run) ? c.run.join(" ") : c.run)}`],
    ]) {
      if (seen.has(key)) problems.push(`${QUICK_MAP} has two checks of the ${what} ${key.slice(what.length + 1)}`);
      else seen.set(key, true);
    }
  }
  return problems;
}

check("no two quick checks share a name or a command", () => {
  // Pull requests 214 and 215 each added the check "records", byte for byte, so that whichever
  // merged second would resolve to one entry; a resolution that kept both would have run the
  // records' tests twice before every push, unnoticed.
  const map = JSON.parse(read(QUICK_MAP));
  assert.deepEqual(duplicateQuickProblems(map), []);
  // Planted: a check kept twice, one renamed beside its twin, and two of one name and other
  // commands.
  const first = map.checks[0];
  assert.equal(duplicateQuickProblems({ ...map, checks: [...map.checks, first] }).length, 2, "a check kept twice passed");
  assert.equal(duplicateQuickProblems({ ...map, checks: [...map.checks, { ...first, name: `${first.name} again` }] }).length, 1, "one command under two names passed");
  assert.equal(duplicateQuickProblems({ ...map, checks: [...map.checks, { ...first, run: ["true"] }] }).length, 1, "one name over two commands passed");
});
