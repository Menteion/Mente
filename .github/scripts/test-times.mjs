// The CLI of lib/test-times.mjs: node .github/scripts/test-times.mjs. The required Rust jobs run it
// after their tests, over nextest's JUnit report, to hold each test well within its stop.
import fs from "node:fs";
import { main } from "./lib/test-times.mjs";

process.exitCode = main(process.argv.slice(2), process.env, {
  read: (file) => fs.readFileSync(file, "utf8"),
  exists: (file) => fs.existsSync(file),
  out: (text) => console.log(text),
  err: (text) => console.error(text),
  summary: (text) => {
    if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
  },
});
