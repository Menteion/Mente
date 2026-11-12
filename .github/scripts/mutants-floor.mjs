// The floor of work of "Mutants in the diff": node .github/scripts/mutants-floor.mjs <command> ...
// The logic is lib/mutants-floor.mjs; this file only runs it, unconditionally, however it is
// started, and prints the one count the workflow reads.
import { main } from "./lib/mutants-floor.mjs";

try {
  console.log(main(process.argv.slice(2)));
} catch (e) {
  console.error(`error: ${e.message}`);
  process.exit(1);
}
