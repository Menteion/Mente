// No code of the change can run where cargo builds, and every crate is an approved one, as cargo
// reads the manifests and the lock:
//   node .github/scripts/build-time-code.mjs --before-toolchain
//   node .github/scripts/build-time-code.mjs
// The logic is lib/build-time-code.mjs; this file only runs it, unconditionally, however it is
// started.
import { execFileSync } from "node:child_process";
import { main } from "./lib/build-time-code.mjs";

/// What cargo reports of the workspace, with every feature on, so that no optional dependency is
/// left out, and with the lock as it stands. cargo reads the manifests and fetches the crates the
/// lock names; it compiles nothing and runs no build script.
const cargoMetadata = (root) => execFileSync("cargo", ["metadata", "--format-version", "1", "--locked", "--all-features"], { cwd: root, encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] });
/// The files git tracks in the repository at `root`.
const tracked = (root) => execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }).split("\0").filter(Boolean);

try {
  const root = process.cwd();
  const { problems, summary } = main(root, cargoMetadata, { beforeToolchain: process.argv.includes("--before-toolchain"), toolchain: { tracked: tracked(root) } });
  if (problems.length > 0) {
    for (const p of problems) console.log(`::error::${p}`);
    process.exit(1);
  }
  console.log(summary);
} catch (e) {
  console.error(`error: ${e.message}`);
  process.exit(1);
}
