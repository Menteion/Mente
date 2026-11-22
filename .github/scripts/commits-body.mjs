// The Commits check's rules on a pull request's text, run before a session creates or edits a pull
// request (root-cause-first; the escape pull-request-body-unchecked-locally):
//   node .github/scripts/commits-body.mjs <body file> [--title <title>]
// The logic is bodyMain and bodyProblems in lib/commits.mjs, which the Commits check runs on
// GitHub; this file only runs it, unconditionally, however it is started.
import fs from "node:fs";
import { bodyMain } from "./lib/commits.mjs";

try {
  const { lines, code } = bodyMain(process.argv.slice(2), (f) => fs.readFileSync(f, "utf8"));
  for (const line of lines) console.log(line);
  process.exitCode = code;
} catch (e) {
  console.error(`error: ${e.message}`);
  process.exit(2);
}
