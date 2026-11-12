// The watch step of every instrument workflow: node .github/scripts/watch.mjs <workflow file name>.
// The logic is lib/watch.mjs. This file only runs it, unconditionally, however it is started: a
// guard comparing the path it was started by with this file's own once let a run through a
// junction skip main(), exit 0 and pass the job.
import { main } from "./lib/watch.mjs";

try {
  main(process.argv.slice(2), process.env, process.cwd());
} catch (e) {
  console.error(`error: ${e.message}`);
  process.exit(1);
}
