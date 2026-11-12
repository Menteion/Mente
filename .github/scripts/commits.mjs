// The Commits check (.github/workflows/commits.yml): node .github/scripts/commits.mjs. The logic is
// lib/commits.mjs; this file only runs it, unconditionally, however it is started.
import { main } from "./lib/commits.mjs";

try {
  process.exitCode = main(process.env, process.cwd());
} catch (e) {
  console.error(`error: ${e.message}`);
  process.exit(1);
}
