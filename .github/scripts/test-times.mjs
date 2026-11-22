// The CLI of lib/test-times.mjs. The required Rust jobs run `node .github/scripts/test-times.mjs
// calibrate` after their tests, to time a fixed workload, and then `node
// .github/scripts/test-times.mjs` over nextest's JUnit report, to judge each test's time against its
// stop and, on the Linux runners, against main's own runs on the same runner.
import fs from "node:fs";
import zlib from "node:zlib";
import { performance } from "node:perf_hooks";
import { main } from "./lib/test-times.mjs";

process.exitCode = await main(process.argv.slice(2), process.env, {
  read: (file) => fs.readFileSync(file, "utf8"),
  write: (file, text) => fs.writeFileSync(file, text),
  exists: (file) => fs.existsSync(file),
  now: () => performance.now(),
  out: (text) => console.log(text),
  err: (text) => console.error(text),
  summary: (text) => {
    if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
  },
  fetch: (url, options) => fetch(url, options),
  sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  inflateRaw: (bytes) => zlib.inflateRawSync(bytes),
});
