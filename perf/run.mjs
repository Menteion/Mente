// The CLI of the tight-loop kernels (perf/README.md): node perf/run.mjs --mn <mn> ... The logic,
// its usage and its tests are perf/lib.mjs and perf/run.test.mjs. This file only runs it,
// unconditionally, however it is started: a guard comparing the path it was started by with the
// module's own skips main() when the two spell one file differently, and the job then passes.
import { main } from "./lib.mjs";

try {
  main(process.argv.slice(2));
} catch (e) {
  process.stderr.write(`error: ${e.message}\n`);
  process.exit(2);
}
