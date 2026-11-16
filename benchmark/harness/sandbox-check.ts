// Proves the benchmark sandbox denies an agent program what it must not have (benchmark/README.md,
// "The harness"; benchmark/harness/sandbox.sh): the API key, the network, the workspace, the
// held-out set, and any write outside its own directory. A Rust canary tries to read a marker from
// the driver's process (through /proc), from a planted credential file and from its own
// environment; to see a network interface; to read benchmark/README.md from the workspace and a
// file standing in for the held-out set, which a measured run checks out in the workspace; to read
// a file planted under <out>/held-out/, standing in for a held-out run's record beside the canary's
// own folder, as a public run's build sits beside the held-out runs' in a measured run; to connect
// to a socket listening where the broker listens, $RUNNER_TEMP/broker/socket, whose requests the
// broker sends on with the API key (benchmark/harness/broker.ts); to reach any of these through a
// descriptor it did not open (both runs start from a shell that holds descriptors on the
// workspace, the masked credential directory and the broker's directory, as the sandbox's root
// half once leaked them: the final pass of 05fe2f6); to write into a directory outside its own; to
// leave a mark on every mount it sees as writable, which this looks for from outside, where one
// found is a channel between runs (the final pass of 7cf871c: /dev/shm); and to write into its own. It also checks that the kernel lets no
// process trace another (Yama's ptrace_scope at 2 or 3, which benchmark/harness/lock-sudo.sh sets
// before this runs), so that no process of the runner's user reads the key out of the broker's memory. Four Odin canaries read the
// credential file, the workspace file, the held-out stand-in and the held-out run's file at compile
// time with `#load`.
// With the sandbox each attempt must fail, except the write into the canary's own directory, which
// the builds need. Without it, as a control, every attempt must succeed, so that a green result
// proves the sandbox and is not a canary that could never read anything.
//
//   node benchmark/harness/sandbox-check.ts --sandbox benchmark/harness/sandbox.sh [--rustc rustc]
//     [--rust-toolchain <toolchain>] [--odin odin] [--out sandbox-out]
//
// It needs --sandbox; without it there is nothing to prove, so it is skipped on a developer's
// machine, which has no sandbox, and fails in CI (CI set), where a measured run stands on it.
// --out must lie under the sandbox's runs root (/etc/mente-sandbox.conf), whose cover leaves only
// the canary's own folders visible; the write probe goes beside it, under the same cover. It runs
// in the workflow "Harness". The marker stands in for the API key; the canary never prints what it
// read, only whether it could.
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import net from "node:net";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  BASH,
  BUILD_TIMEOUT_MS,
  REPOSITORY,
  RUN_TIMEOUT_MS,
  RUST_FLAGS,
  checkUnderRuns,
  insideAny,
  childEnv,
  readSandboxRoots,
  runProcess,
  rustcInSysroot,
  sandboxed,
} from "./build.ts";
import { own } from "../own.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const MARKER = "BENCH-DRIVER-MARKER-7f3a91c2";
const ROUTES = ["env", "proc", "file", "net", "ws", "held", "run", "broker", "fd", "write", "work"] as const;
type Route = typeof ROUTES extends readonly (infer R)[] ? R : never;

function option(name: string, fallback: string): string {
  const at = process.argv.indexOf(`--${name}`);
  return at > 0 && at + 1 < process.argv.length ? own(process.argv, at + 1) : fallback;
}

interface Run {
  flags: Record<Route, boolean>;
  /** Whether the canary ran and printed every flag, so that a crash is not read as a block. */
  ran: boolean;
  status: number | null;
  /** The markers it left on writable mounts that the runner, outside the sandbox, could see. */
  seen: string[];
  raw: string;
  stderr: string;
}

async function main(): Promise<void> {
  const sandbox = option("sandbox", "");
  if (!sandbox) {
    // In CI the check is the proof a measured run stands on: without a sandbox it fails, so that an
    // empty SANDBOX can never read as a sandbox proven.
    if (process.env.CI) throw new Error("no --sandbox: in CI the sandbox must be proven, not skipped");
    console.log("sandbox-check skipped: pass --sandbox to run it");
    return;
  }
  const pins = JSON.parse(readFileSync(join(here, "toolchains.json"), "utf8"));
  const rustToolchain = option("rust-toolchain", pins.rust.toolchain);
  const rustc = await rustcInSysroot(option("rustc", "rustc"), rustToolchain);
  const odin = option("odin", "odin");
  const out = resolve(option("out", "sandbox-out"));
  checkUnderRuns(out, readSandboxRoots());
  const beside = dirname(out);
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });

  const home = process.env.HOME ?? "";
  if (!home) throw new Error("HOME is not set; the sandbox check needs it to plant a file");
  // A credential file under a directory the sandbox masks.
  const configDir = join(home, ".config");
  mkdirSync(configDir, { recursive: true });
  const credential = join(configDir, "bench-sandbox-canary-marker");
  writeFileSync(credential, `${MARKER}\n`);
  // A file of the workspace, which the sandbox covers.
  const workspaceFile = join(REPOSITORY, "benchmark", "README.md");
  // A stand-in for the held-out set, in the workspace, where a measured run checks it out.
  const heldOutDir = join(REPOSITORY, ".held-out-stand-in");
  const heldOutFile = join(heldOutDir, "tasks", "held-out-task.md");
  mkdirSync(dirname(heldOutFile), { recursive: true });
  writeFileSync(heldOutFile, `${MARKER}\n`);
  // A directory outside the canary's own, which the read-only root keeps from being written.
  const outside = join(beside, "write-probe");
  mkdirSync(outside, { recursive: true });
  // Another run's folder under --out, as a held-out run's record lies beside a public run's build.
  // The output directory is covered all but each build's own folder, as the measured run covers it.
  const otherRun = join(out, "held-out", "held-out-1-rust-model-1", "record.json");
  mkdirSync(dirname(otherRun), { recursive: true });
  writeFileSync(otherRun, `${MARKER}\n`);
  // The driver carries the marker in its own environment, so /proc reaches it only unconfined.
  process.env.BENCH_CANARY_DRIVER = MARKER;
  // A socket where the broker listens, which the sandbox covers by default: the check runs before
  // the broker starts, and refuses to stand in for one that is there.
  const runnerTemp = process.env.RUNNER_TEMP ?? "";
  if (runnerTemp === "") throw new Error("RUNNER_TEMP is not set; the sandbox check needs it to plant the broker's socket");
  const brokerDir = join(runnerTemp, "broker");
  if (existsSync(brokerDir)) throw new Error(`${brokerDir} exists: the sandbox check runs before the broker starts`);
  mkdirSync(brokerDir, { recursive: true });
  const brokerSocket = join(brokerDir, "socket");
  const listener = net.createServer((conn) => conn.end());
  await new Promise<void>((done, failed) => {
    listener.on("error", failed);
    listener.listen(brokerSocket, () => done());
  });

  const failures: string[] = [];
  try {
    // The Rust canary, built once (our own code), then run with and without the sandbox.
    const rustDir = join(out, "rust");
    mkdirSync(rustDir, { recursive: true });
    writeFileSync(join(rustDir, "leak.rs"), readFileSync(join(here, "fixtures", "canary", "leak.rs"), "utf8"));
    const exe = join(rustDir, "leak");
    const buildEnv = childEnv(rustToolchain ? { RUSTUP_TOOLCHAIN: rustToolchain } : {});
    const built = await runProcess(rustc, [...RUST_FLAGS, "-o", exe, join(rustDir, "leak.rs")], { env: buildEnv, timeoutMs: BUILD_TIMEOUT_MS });
    if (built.status !== 0) throw new Error(`the canary did not build: ${built.stderr}`);

    const run = async (confined: boolean): Promise<Run> => {
      const tag = `${confined ? "confined" : "control"}-${Date.now()}`;
      const [inner, innerArgs] = sandboxed(confined ? sandbox : "", rustDir, exe, [credential, workspaceFile, heldOutFile, outside, otherRun, brokerSocket, tag]);
      // Started from a shell that holds descriptors 3, 4 and 5 on the workspace, the credential's
      // masked directory and the broker's directory: the control reaches the planted files through
      // them, and the sandbox must start the canary with none of them, nor any other.
      const [cmd, args] = [BASH, ["-c", 'exec 3<"$1" 4<"$2" 5<"$3" && shift 3 && exec "$@"', "bash", REPOSITORY, configDir, brokerDir, inner, ...innerArgs]];
      // The needle lets the canary find the marker in /proc; a non-secret name, so it is forwarded.
      const env = childEnv({ BENCH_CANARY_NEEDLE: MARKER, BENCH_CANARY_ENV: "set" });
      const p = await runProcess(cmd, args, { cwd: rustDir, env, timeoutMs: RUN_TIMEOUT_MS });
      const printed = Object.fromEntries(p.stdout.trim().split(/\s+/).map((kv) => kv.split("=")));
      const flags = Object.fromEntries(ROUTES.map((r) => [r, own(printed, r) === "true"])) as Record<Route, boolean>;
      // Where the canary left a marker on a mount it saw as writable, looked for from outside the
      // sandbox: one found here, outside its own directory, is a place another run could find it.
      const list = join(rustDir, "canary-wrote.txt");
      const wrote = existsSync(list) ? readFileSync(list, "utf8").split("\n").filter((l) => l !== "") : [];
      rmSync(list, { force: true });
      const seen = wrote.filter((w) => !insideAny(w, [rustDir]) && existsSync(w));
      for (const w of seen) rmSync(w, { force: true });
      return { flags, ran: ROUTES.every((r) => r in printed), status: p.status, raw: p.stdout.trim(), stderr: p.stderr.trim(), seen };
    };

    const control = await run(false);
    const confined = await run(true);
    console.log(`rust control  : ${control.raw}`);
    console.log(`rust sandboxed: ${confined.raw}`);
    console.log(`writes seen outside: control ${control.seen.length} (${control.seen.join(" ")}), sandboxed ${confined.seen.length}`);
    // A write the runner sees from a run's sandbox, outside its own directory, is a channel from
    // that run to the next, a held-out run's to a public one's: the sandbox leaves none, and the
    // control, writing as the runner does, finds one, or the probe proves nothing.
    if (control.seen.length === 0) failures.push("the control canary left no marker the runner could see, so its sandboxed run proves nothing of shared mounts");
    if (confined.seen.length > 0) failures.push(`the sandbox let the canary write where the runner, and so another run, sees it: ${confined.seen.join(", ")}`);
    // Each run must have produced its flags, so a canary the sandbox crashed is not read as blocked.
    if (!control.ran || control.status !== 0) failures.push(`the control canary did not run: status ${control.status}, ${control.raw} ${control.stderr}`);
    if (!confined.ran || confined.status !== 0) failures.push(`the sandboxed canary did not run: status ${confined.status}, ${confined.raw} ${confined.stderr}`);
    // Unconfined, every route is open, or the canary proves nothing.
    if (control.ran) {
      for (const route of ROUTES) {
        if (!own(control.flags, route)) failures.push(`the control canary could not reach ${route}, so its sandboxed run proves nothing: ${control.raw}`);
      }
    }
    // Confined, every route is closed but the canary's own directory, which builds write into.
    if (confined.ran) {
      for (const route of ROUTES) {
        const open = own(confined.flags, route);
        if (route === "work" ? !open : open) {
          failures.push(route === "work" ? `the sandbox kept the canary from writing its own directory: ${confined.raw}` : `the sandbox let the canary reach ${route}: ${confined.raw} ${confined.stderr}`);
        }
      }
    }

    // The Odin canaries read each file at compile time with `#load`.
    const targets: [string, string][] = [
      ["credential file", credential],
      ["workspace file", workspaceFile],
      ["held-out stand-in", heldOutFile],
      ["held-out run's file", otherRun],
    ];
    for (const [i, [what, path]] of targets.entries()) {
      const odinDir = join(out, `odin-${i + 1}`);
      mkdirSync(odinDir, { recursive: true });
      const program = `package main\n\nMARKER :: #load(${JSON.stringify(path)}, string)\n\nmain :: proc() {\n\tif len(MARKER) == 0 {\n\t\tpanic("empty")\n\t}\n}\n`;
      writeFileSync(join(odinDir, "load.odin"), program);
      const build = async (confined: boolean) => {
        const [cmd, args] = sandboxed(confined ? sandbox : "", odinDir, odin, ["build", ".", "-out:load"]);
        return runProcess(cmd, args, { cwd: odinDir, env: childEnv(), timeoutMs: BUILD_TIMEOUT_MS });
      };
      const odinControl = await build(false);
      const odinConfined = await build(true);
      console.log(`odin #load ${what}: control ${odinControl.status === 0 ? "read it" : "could not read it"}, sandboxed ${odinConfined.status === 0 ? "read it" : "could not read it"}`);
      if (odinControl.status !== 0) failures.push(`the Odin control did not read the ${what} with #load: ${odinControl.stderr.trim()}`);
      if (odinConfined.status === 0) failures.push(`the sandbox let Odin read the ${what} at compile time with #load`);
    }
    // No process of the runner's user may trace another, which only a process with CAP_SYS_PTRACE
    // may once lock-sudo.sh has run: Yama's scope 2 or 3.
    let scope = "";
    try {
      scope = readFileSync("/proc/sys/kernel/yama/ptrace_scope", "utf8").trim();
    } catch {
      scope = "absent";
    }
    console.log(`yama ptrace_scope: ${scope}`);
    if (!/^[23]$/.test(scope)) failures.push(`Yama's ptrace_scope is ${scope}, so a process of the runner's user could read another's memory, the broker's and the key in it among them: lock-sudo.sh sets 2`);
  } finally {
    await new Promise((done) => listener.close(() => done(undefined)));
    rmSync(brokerDir, { recursive: true, force: true });
    rmSync(credential, { force: true });
    rmSync(heldOutDir, { recursive: true, force: true });
    rmSync(outside, { recursive: true, force: true });
    delete process.env.BENCH_CANARY_DRIVER;
  }

  if (failures.length > 0) {
    console.error(`\n${failures.length} sandbox failures:\n${failures.join("\n")}`);
    process.exit(1);
  }
  console.log(
    "\nthe sandbox denies the driver's marker, a credential file, the environment, the network, the workspace, the held-out set, another run's folder, the broker's socket, every descriptor it did not open, writes outside the working directory and a mark on any mount another run could see, and #load of each at build time; no process traces another",
  );
}

await main();
