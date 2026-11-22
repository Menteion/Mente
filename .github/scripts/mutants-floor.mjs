// The floor of work of "Mutants in the diff": node .github/scripts/mutants-floor.mjs <command> ...
// The logic is lib/mutants-floor.mjs; this file only runs it, unconditionally, however it is
// started, and prints what it returns: nothing at all when that is empty, so that a file of its
// output holds a line only for a finding (the review of 560ccd9: a line feed alone made every
// shard read its listing as wrong).
import { main } from "./lib/mutants-floor.mjs";

try {
  const out = main(process.argv.slice(2));
  if (out !== "") process.stdout.write(`${out}\n`);
} catch (e) {
  console.error(`error: ${e.message}`);
  process.exit(1);
}
