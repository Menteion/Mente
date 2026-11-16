// The Commits check (.github/workflows/commits.yml): node .github/scripts/commits.mjs. The logic is
// lib/commits.mjs; this file only runs it, unconditionally, however it is started.
import { execFileSync } from "node:child_process";
import { main, parseLog } from "./lib/commits.mjs";

/// The commits reachable from `head` and not from `base`, oldest first.
const between = (cwd) => (base, head) => parseLog(execFileSync("git", ["log", "--reverse", "--format=%H%x1f%s%x1f%B%x1e", `${base}..${head}`], { cwd, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }));

try {
  process.exitCode = await main(process.env, between(process.cwd()));
} catch (e) {
  console.error(`error: ${e.message}`);
  process.exit(1);
}
