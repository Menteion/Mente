// The local limits, proved on the machine they guard (tools/local-limits.mjs, tools/capped.ps1):
// a process that allocates without end dies alone at its limit; a command past its time ends with
// every process it started; a third build waits while two hold the machine's slots; a slot whose
// holder is killed is free at once; and outside a checkout of this repository, -MenteOnly leaves a
// command untouched. Windows only: elsewhere nothing is bounded, and these are skipped.
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

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..");
const CAPPED = path.join(HERE, "capped.ps1");
const skip = process.platform !== "win32" && "the local limits bound processes on Windows alone";

/// capped.ps1 with `options`, running node on `script`, from `cwd`.
function cappedNode(options, script, { cwd = ROOT, extra = [] } = {}) {
  const started = Date.now();
  const r = spawnSync("pwsh", ["-NoProfile", "-NonInteractive", "-File", CAPPED, ...options, "--", process.execPath, "-e", script, ...extra], { cwd, encoding: "utf8", timeout: 120000 });
  return { status: r.status, out: `${r.stdout}${r.stderr}`, ms: Date.now() - started };
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
  const r = cappedNode(["-ProcessGB", "1", "-JobGB", "2"], "const a=[];let n=0;try{for(;;){if(n*64>=2048){console.log('not refused at 2048 MB');process.exit(8)}a.push(Buffer.alloc(1<<26));n++}}catch(e){console.log(`refused after ${n*64} MB`);process.exit(7)}");
  assert.equal(r.status, 7, r.out);
  const mb = Number(/refused after (\d+) MB/.exec(r.out)?.[1]);
  assert.ok(mb > 0 && mb < 1024, `allocated ${mb} MB under a limit of 1 GB: ${r.out}`);
  assert.match(r.out, /capped: a process of .* reached its limit of 1 GB committed/);
});

test("a command past its time ends, with every process it started", { skip }, () => {
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "capped-")), "child");
  const r = cappedNode(["-Minutes", "0.05"], "const c=require('child_process').spawn(process.execPath,['-e','setTimeout(()=>{},600000)'],{stdio:'ignore'});require('fs').writeFileSync(process.argv[1],String(c.pid));setTimeout(()=>{},600000)", { extra: [file] });
  assert.equal(r.status, 124, r.out);
  assert.ok(r.ms < 30000, `it took ${r.ms} ms to stop a command at 3 s`);
  assert.match(r.out, /capped: stopped .* at its limit of 0.05 minutes/);
  const child = Number(fs.readFileSync(file, "utf8"));
  assert.ok(child > 0 && !alive(child), `the command's child ${child} outlived it`);
});

test("outside a checkout of this repository, -MenteOnly leaves a command untouched", { skip }, () => {
  const wait = "setTimeout(()=>{},4000)";
  assert.equal(cappedNode(["-MenteOnly", "-Minutes", "0.02"], wait).status, 124, "-MenteOnly left a command in the repository unbounded");
  assert.equal(cappedNode(["-MenteOnly", "-Minutes", "0.02"], wait, { cwd: os.tmpdir() }).status, 0, "-MenteOnly bounded a command outside the repository");
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
