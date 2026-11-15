// No code of the change can run where cargo builds, and every crate is an approved one, as cargo
// reads the manifests and the lock:
//   node .github/scripts/build-time-code.mjs --before-toolchain
//   node .github/scripts/build-time-code.mjs
// The logic is lib/build-time-code.mjs; this file only runs it, unconditionally, however it is
// started.
import { main } from "./lib/build-time-code.mjs";

try {
  const { problems, summary } = main(process.cwd(), undefined, { beforeToolchain: process.argv.includes("--before-toolchain") });
  if (problems.length > 0) {
    for (const p of problems) console.log(`::error::${p}`);
    process.exit(1);
  }
  console.log(summary);
} catch (e) {
  console.error(`error: ${e.message}`);
  process.exit(1);
}
