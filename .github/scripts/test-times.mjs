// The CLI of lib/test-times.mjs: node .github/scripts/test-times.mjs. The required Rust jobs run it
// after their tests, over nextest's JUnit report, to judge each test's time against main's own
// green runs on the same runner.
import fs from "node:fs";
import zlib from "node:zlib";
import { main } from "./lib/test-times.mjs";

process.exitCode = await main(process.argv.slice(2), process.env, {
  read: (file) => fs.readFileSync(file, "utf8"),
  exists: (file) => fs.existsSync(file),
  out: (text) => console.log(text),
  err: (text) => console.error(text),
  summary: (text) => {
    if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
  },
  fetch: (url, options) => fetch(url, options),
  sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  inflateRaw: (bytes) => zlib.inflateRawSync(bytes),
});
