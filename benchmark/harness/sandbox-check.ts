// Proves the benchmark sandbox denies an agent program what it must not have (benchmark/README.md,
// "The harness"; benchmark/harness/sandbox.sh): the API key, the network, the workspace, the
// held-out set, and any write outside its own directory. A Rust canary tries to read a marker from
// the driver's process (through /proc), from a planted credential file and from its own
// environment; to see a network interface; to read benchmark/README.md from the workspace and a
// file standing in for the held-out set, which lives outside the workspace and is hidden by name;
// to write into a directory outside its own; and to write into its own. Three Odin canaries read
// the credential file, the workspace file and the held-out stand-in at compile time with `#load`.
// With the sandbox each attempt must fail, except the write into the canary's own directory, which
// the builds need. Without it, as a control, every attempt must succeed, so that a green result
// proves the sandbox and is not a canary that could never read anything.
//
//   node benchmark/harness/sandbox-check.ts --sandbox benchmark/harness/sandbox.sh [--rustc rustc]
//     [--rust-toolchain <toolchain>] [--odin odin] [--out sandbox-out]
//
// It needs --sandbox; without it there is nothing to prove, so it is skipped, since a developer's
// machine has no sandbox. --out must lie outside the repository, which the sandbox hides; the
// held-out stand-in and the write probe go beside it. It runs in the workflow "Harness". The
// marker stands in for the API key; the canary never prints what it read, only whether it could.
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  BUILD_TIMEOUT_MS,
  REPOSITORY,
  RUN_TIMEOUT_MS,
  RUST_FLAGS,
  childEnv,
  insideAny,
  runProcess,
  rustcInSysroot,
  sandboxed,
} from "./build.ts";

const here = dirname(fileURLToPath(import.meta.url));
const MARKER = "BENCH-DRIVER-MARKER-7f3a91c2";
const ROUTES = ["env", "proc", "file", "net", "ws", "held", "write", "work"] as const;
type Route = (typeof ROUTES)[number];

function option(name: string, fallback: string): string {
  const at = process.argv.indexOf(`--${name}`);
  return at > 0 && at + 1 < process.argv.length ? process.argv[at + 1] : fallback;
}

interface Run {
  flags: Record<Route, boolean>;
  /** Whether the canary ran and printed every flag, so that a crash is not read as a block. */
  ran: boolean;
  status: number | null;
  raw: string;
  stderr: string;
}

async function main(): Promise<void> {
  const sandbox = option("sandbox", "");
  if (!sandbox) {
    console.log("sandbox-check skipped: pass --sandbox to run it");
    return;
  }
  const pins = JSON.parse(readFileSync(join(here, "toolchains.json"), "utf8"));
  const rustToolchain = option("rust-toolchain", pins.rust.toolchain);
  const rustc = await rustcInSysroot(option("rustc", "rustc"), rustToolchain);
  const odin = option("odin", "odin");
  const out = resolve(option("out", "sandbox-out"));
  if (insideAny(out, [REPOSITORY])) throw new Error(`--out must lie outside ${REPOSITORY}, which the sandbox hides`);
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
  // A stand-in for the held-out set, outside the workspace, hidden by name as the run hides the
  // held-out checkout wherever it is.
  const heldOutDir = join(beside, "held-out-stand-in");
  const heldOutFile = join(heldOutDir, "tasks", "held-out-task.md");
  mkdirSync(dirname(heldOutFile), { recursive: true });
  writeFileSync(heldOutFile, `${MARKER}\n`);
  // A directory outside the canary's own, which the read-only root keeps from being written.
  const outside = join(beside, "write-probe");
  mkdirSync(outside, { recursive: true });
  // A path inside the repository is hidden too, as the measured run hides the held-out checkout
  // inside the workspace; the sandbox must take it in its stride, or no canary would run.
  const hide = [REPOSITORY, join(REPOSITORY, "benchmark"), heldOutDir];
  // The driver carries the marker in its own environment, so /proc reaches it only unconfined.
  process.env.BENCH_CANARY_DRIVER = MARKER;

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
      const [cmd, args] = sandboxed(confined ? sandbox : "", rustDir, exe, [credential, workspaceFile, heldOutFile, outside], hide);
      // The needle lets the canary find the marker in /proc; a non-secret name, so it is forwarded.
      const env = childEnv({ BENCH_CANARY_NEEDLE: MARKER, BENCH_CANARY_ENV: "set" });
      const p = await runProcess(cmd, args, { cwd: rustDir, env, timeoutMs: RUN_TIMEOUT_MS });
      const printed = Object.fromEntries(p.stdout.trim().split(/\s+/).map((kv) => kv.split("=")));
      const flags = Object.fromEntries(ROUTES.map((r) => [r, printed[r] === "true"])) as Record<Route, boolean>;
      return { flags, ran: ROUTES.every((r) => r in printed), status: p.status, raw: p.stdout.trim(), stderr: p.stderr.trim() };
    };

    const control = await run(false);
    const confined = await run(true);
    console.log(`rust control  : ${control.raw}`);
    console.log(`rust sandboxed: ${confined.raw}`);
    // Each run must have produced its flags, so a canary the sandbox crashed is not read as blocked.
    if (!control.ran || control.status !== 0) failures.push(`the control canary did not run: status ${control.status}, ${control.raw} ${control.stderr}`);
    if (!confined.ran || confined.status !== 0) failures.push(`the sandboxed canary did not run: status ${confined.status}, ${confined.raw} ${confined.stderr}`);
    // Unconfined, every route is open, or the canary proves nothing.
    if (control.ran) {
      for (const route of ROUTES) {
        if (!control.flags[route]) failures.push(`the control canary could not reach ${route}, so its sandboxed run proves nothing: ${control.raw}`);
      }
    }
    // Confined, every route is closed but the canary's own directory, which builds write into.
    if (confined.ran) {
      for (const route of ROUTES) {
        const open = confined.flags[route];
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
    ];
    for (const [i, [what, path]] of targets.entries()) {
      const odinDir = join(out, `odin-${i + 1}`);
      mkdirSync(odinDir, { recursive: true });
      const program = `package main\n\nMARKER :: #load(${JSON.stringify(path)}, string)\n\nmain :: proc() {\n\tif len(MARKER) == 0 {\n\t\tpanic("empty")\n\t}\n}\n`;
      writeFileSync(join(odinDir, "load.odin"), program);
      const build = async (confined: boolean) => {
        const [cmd, args] = sandboxed(confined ? sandbox : "", odinDir, odin, ["build", ".", "-out:load"], hide);
        return runProcess(cmd, args, { cwd: odinDir, env: childEnv(), timeoutMs: BUILD_TIMEOUT_MS });
      };
      const odinControl = await build(false);
      const odinConfined = await build(true);
      console.log(`odin #load ${what}: control ${odinControl.status === 0 ? "read it" : "could not read it"}, sandboxed ${odinConfined.status === 0 ? "read it" : "could not read it"}`);
      if (odinControl.status !== 0) failures.push(`the Odin control did not read the ${what} with #load: ${odinControl.stderr.trim()}`);
      if (odinConfined.status === 0) failures.push(`the sandbox let Odin read the ${what} at compile time with #load`);
    }
  } finally {
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
    "\nthe sandbox denies the driver's marker, a credential file, the environment, the network, the workspace, the held-out set and writes outside the working directory, and #load of each at build time",
  );
}

await main();
