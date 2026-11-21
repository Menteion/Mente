// The step summary of a run of mn fuzz (fuzz.yml and fuzz-nightly.yml): the campaign's summary, then
// each failure's description and minimized input. GitHub refuses a step summary past 1024k and then
// shows none of it, so every part is capped by lines and by bytes, the whole by bytes, and each cut
// is said with what it left out (2026-11-21: a minimized input of 3 MB on one line, which a cap of
// 40 lines let through, aborted the nightly's summary). A part is read as bytes: cut only where a
// UTF-8 character ends, with a NUL, a control byte and a byte of no character shown escaped, and
// every count in true bytes (the review of 4ae3c27). The whole of every file is in the run's
// artifact. The run tests feed it each of those inputs.
//
//   node .github/scripts/fuzz-summary.mjs <fuzz output> <title> <mn fuzz's status> >> "$GITHUB_STEP_SUMMARY"
import fs from "node:fs";
import path from "node:path";

// What one part may show, and what the whole may, well under GitHub's 1024k for a step's summary.
const PART_LINES = 40;
const PART_BYTES = 4096;
const TOTAL_BYTES = 512 * 1024;

/// The length of the UTF-8 character that starts at `b[k]`, or 0 when no character starts there or
/// it is cut short at `end`: overlong forms, surrogates and code points past U+10FFFF are none.
function charLength(b, k, end) {
  const x = b[k];
  if (x < 0x80) return 1;
  const [n, lo, hi] = x >= 0xc2 && x <= 0xdf ? [2, 0x80, 0xbf] : x === 0xe0 ? [3, 0xa0, 0xbf] : x === 0xed ? [3, 0x80, 0x9f] : x >= 0xe1 && x <= 0xef ? [3, 0x80, 0xbf] : x === 0xf0 ? [4, 0x90, 0xbf] : x >= 0xf1 && x <= 0xf3 ? [4, 0x80, 0xbf] : x === 0xf4 ? [4, 0x80, 0x8f] : [0];
  if (n === 0 || k + n > end) return 0;
  if (b[k + 1] < lo || b[k + 1] > hi) return 0;
  for (let j = 2; j < n; j++) if (b[k + j] < 0x80 || b[k + j] > 0xbf) return 0;
  return n;
}

/// A file's first PART_LINES lines and at most PART_BYTES bytes of them, as text a reader can read,
/// and the bytes of the file it showed: never half of a character at the cut.
function part(file) {
  const size = fs.statSync(file).size;
  const fd = fs.openSync(file, "r");
  const b = Buffer.alloc(Math.min(size, PART_BYTES + 3));
  const read = fs.readSync(fd, b, 0, b.length, 0);
  fs.closeSync(fd);
  const limit = Math.min(read, PART_BYTES);
  let text = "";
  let shown = 0;
  let lines = 0;
  for (let k = 0; k < limit && lines < PART_LINES; ) {
    const x = b[k];
    if (x === 0x0a) {
      text += "\n";
      lines++;
      k++;
    } else if (x === 0x09) {
      text += "\t";
      k++;
    } else if (x < 0x20 || x === 0x7f) {
      text += `\\x${x.toString(16).padStart(2, "0")}`;
      k++;
    } else if (x < 0x80) {
      text += String.fromCharCode(x);
      k++;
    } else {
      // Three bytes past the cap are read, so a character that starts before it is read whole unless
      // the file itself ends inside it.
      const n = charLength(b, k, read);
      if (n === 0) {
        // A byte that starts no character: shown escaped, never as a reader's replacement mark.
        text += `\\x${x.toString(16)}`;
        k++;
      } else if (k + n > limit) {
        // A whole character the cap would split: left out with the rest, never half shown.
        break;
      } else {
        const ch = b.toString("utf8", k, k + n);
        const cp = ch.codePointAt(0);
        text += cp >= 0x80 && cp <= 0x9f ? `\\u{${cp.toString(16)}}` : ch;
        k += n;
      }
    }
    shown = k;
  }
  return { text, shown, size };
}

const say = (() => {
  let written = 0;
  const f = (line) => {
    process.stdout.write(`${line}\n`);
    written += Buffer.byteLength(line) + 1;
  };
  f.written = () => written;
  return f;
})();

/// A file shown in a fence no line of it can close, and what was cut of it.
function show(file) {
  const { text, shown, size } = part(file);
  const fence = "~".repeat(Math.max(8, ...[...text.matchAll(/~+/g)].map((m) => m[0].length + 1)));
  say(`${fence}text`);
  say(text.endsWith("\n") ? text.slice(0, -1) : text);
  if (shown < size) say(`--- cut here: ${size - shown} of its ${size} bytes are past ${PART_LINES} lines or ${PART_BYTES} bytes, and in the run's artifact`);
  say(fence);
}

const [out, title, status, ...more] = process.argv.slice(2);
if (out === undefined || title === undefined || status === undefined || more.length > 0) {
  console.error("usage: fuzz-summary.mjs <fuzz output> <title> <mn fuzz's status>");
  process.exit(2);
}
say(`## ${title}`);
const summary = path.join(out, "summary.txt");
if (fs.existsSync(summary)) show(summary);
else say(`no summary: mn fuzz exited with ${status}`);
const failures = (fs.existsSync(out) ? fs.readdirSync(out) : [])
  .filter((f) => f.endsWith(".txt") && f !== "summary.txt" && fs.statSync(path.join(out, f)).isFile())
  .sort();
for (const [k, f] of failures.entries()) {
  if (say.written() >= TOTAL_BYTES) {
    say(`--- ${failures.length - k} more failures are not shown here, past ${TOTAL_BYTES} bytes of this summary, and in the run's artifact`);
    break;
  }
  say(`### ${f}`);
  show(path.join(out, f));
  say("minimized input:");
  const input = path.join(out, f.replace(/\.txt$/, ".mn"));
  if (fs.existsSync(input)) show(input);
  else say("(none)");
}
