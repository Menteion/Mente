// The watch step of every instrument workflow: node .github/scripts/watch.mjs <workflow file name>.
// The logic is lib/watch.mjs. This file only runs it, unconditionally, however it is started: a
// guard comparing the path it was started by with this file's own once let a run through a
// junction skip main(), exit 0 and pass the job.
import { execFileSync } from "node:child_process";
import { main } from "./lib/watch.mjs";

/// The files changed between the merge base of `base` and `head`, and `head`, renames listed as
/// a deletion and an addition so that a file moved out of a watched path still counts.
const changed = (cwd) => (base, head) =>
  execFileSync("git", ["diff", "--name-only", "--no-renames", "-z", `${base}...${head}`], { cwd, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 })
    .split("\0")
    .filter((f) => f !== "");

try {
  main(process.argv.slice(2), process.env, process.cwd(), changed(process.cwd()));
} catch (e) {
  console.error(`error: ${e.message}`);
  process.exit(1);
}
