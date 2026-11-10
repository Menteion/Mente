// Proves the benchmark sandbox denies an agent program the API key and the network
// (benchmark/README.md, "The harness"; benchmark/harness/sandbox.sh). It runs a Rust canary, which
// tries to read a marker from the driver's process (through /proc), from a planted file and from
// its own environment, and to see a network interface; and it builds an Odin canary that reads the
// planted file at compile time with `#load`. With the sandbox each attempt must fail. Without it,
// as a control, the attempts on the driver's process and the network must succeed, so that a green
// result proves the sandbox and is not a canary that could never read anything.
//
//   node benchmark/harness/sandbox-check.ts --sandbox benchmark/harness/sandbox.sh [--rustc rustc]
//     [--rust-toolchain <toolchain>] [--odin odin] [--out sandbox-out]
//
// It needs --sandbox; without it there is nothing to prove, so it is skipped, since a developer's
// machine has no sandbox. It runs in the workflow "Harness". The marker stands in for the API key;
// the canary never prints a secret, only whether it could reach one.
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { BUILD_TIMEOUT_MS, RUN_TIMEOUT_MS, RUST_FLAGS, childEnv, runProcess, sandboxed } from "./build.ts";

const here = dirname(fileURLToPath(import.meta.url));
const MARKER = "BENCH-DRIVER-MARKER-7f3a91c2";

function option(name: string, fallback: string): string {
  const at = process.argv.indexOf(`--${name}`);
  return at > 0 && at + 1 < process.argv.length ? process.argv[at + 1] : fallback;
}

interface Flags {
  env: boolean;
  proc: boolean;
  file: boolean;
  net: boolean;
  /** Whether the canary ran and printed its four flags, so that a crash is not read as a block. */
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
  const rustc = option("rustc", "rustc");
  const rustToolchain = option("rust-toolchain", pins.rust.toolchain);
  const odin = option("odin", "odin");
  const out = option("out", "sandbox-out");
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });

  const home = process.env.HOME ?? "";
  if (!home) throw new Error("HOME is not set; the sandbox check needs it to plant a file");
  // A file under a credential directory the sandbox hides. The run restores the directory after.
  const configDir = join(home, ".config");
  mkdirSync(configDir, { recursive: true });
  const planted = join(configDir, "bench-sandbox-canary-marker");
  writeFileSync(planted, `${MARKER}\n`);
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

    const run = async (confined: boolean): Promise<Flags> => {
      const [cmd, args] = sandboxed(confined ? sandbox : "", rustDir, exe, [planted]);
      // The needle lets the canary find the marker in /proc; a non-secret name, so it is forwarded.
      const env = childEnv({ BENCH_CANARY_NEEDLE: MARKER, BENCH_CANARY_ENV: "set" });
      const p = await runProcess(cmd, args, { cwd: rustDir, env, timeoutMs: RUN_TIMEOUT_MS });
      const flags = Object.fromEntries(p.stdout.trim().split(/\s+/).map((kv) => kv.split("=")));
      const ran = ["env", "proc", "file", "net"].every((k) => k in flags);
      return {
        env: flags.env === "true",
        proc: flags.proc === "true",
        file: flags.file === "true",
        net: flags.net === "true",
        ran,
        status: p.status,
        raw: p.stdout.trim(),
        stderr: p.stderr.trim(),
      };
    };

    const control = await run(false);
    const confined = await run(true);
    console.log(`rust control  : ${control.raw}`);
    console.log(`rust sandboxed: ${confined.raw}`);
    // Each run must have actually produced its flags, so that a canary the sandbox crashed is not
    // read as one the sandbox blocked.
    if (!control.ran || control.status !== 0) failures.push(`the control canary did not run: status ${control.status}, ${control.raw} ${control.stderr}`);
    if (!confined.ran || confined.status !== 0) failures.push(`the sandboxed canary did not run: status ${confined.status}, ${confined.raw} ${confined.stderr}`);
    // The control must reach the driver's environment and the network, or the canary proves nothing.
    if (control.ran && !control.proc) failures.push(`control did not read the driver's environment through /proc: ${control.raw}`);
    if (control.ran && !control.net) failures.push(`control saw no network interface: ${control.raw}`);
    // The sandbox must block every route.
    if (confined.ran) {
      for (const route of ["env", "proc", "file", "net"] as const) {
        if (confined[route]) failures.push(`the sandbox let the canary reach ${route}: ${confined.raw} ${confined.stderr}`);
      }
    }

    // The Odin canary reads the planted file at compile time with `#load`.
    const odinDir = join(out, "odin");
    mkdirSync(odinDir, { recursive: true });
    const program = `package main\n\nMARKER :: #load(${JSON.stringify(planted)}, string)\n\nmain :: proc() {\n\tif len(MARKER) == 0 {\n\t\tpanic("empty")\n\t}\n}\n`;
    writeFileSync(join(odinDir, "load.odin"), program);
    const build = async (confined: boolean) => {
      const [cmd, args] = sandboxed(confined ? sandbox : "", odinDir, odin, ["build", ".", "-out:load"]);
      return runProcess(cmd, args, { cwd: odinDir, env: childEnv(), timeoutMs: BUILD_TIMEOUT_MS });
    };
    const odinControl = await build(false);
    const odinConfined = await build(true);
    console.log(`odin #load control  : ${odinControl.status === 0 ? "read the file" : "could not read the file"}`);
    console.log(`odin #load sandboxed: ${odinConfined.status === 0 ? "read the file" : "could not read the file"}`);
    if (odinControl.status !== 0) failures.push(`the Odin control did not read the planted file with #load: ${odinControl.stderr.trim()}`);
    if (odinConfined.status === 0) failures.push("the sandbox let Odin read the planted file at compile time with #load");
  } finally {
    rmSync(planted, { force: true });
    delete process.env.BENCH_CANARY_DRIVER;
  }

  if (failures.length > 0) {
    console.error(`\n${failures.length} sandbox failures:\n${failures.join("\n")}`);
    process.exit(1);
  }
  console.log("\nthe sandbox denies the driver's marker, a planted file, the environment and the network, and #load at build time");
}

await main();
