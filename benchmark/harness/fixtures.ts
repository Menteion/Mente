// Checks the harness end to end in Rust and Odin (benchmark/README.md, "The harness"): each
// fixture under fixtures/<language>/ is a solution of a public M1 task, built through the harness
// as an agent's program would be. A solve task's fixture must build and pass every example and
// hidden test. An adversarial task's fixture starts with a line `// expect: <verdict>`, and its
// probe must end with that verdict. Every public M1 solve task needs a fixture in each language.
//
//   node benchmark/harness/fixtures.ts [--languages rust,odin] [--rustc rustc] [--rust-toolchain <toolchain>]
//     [--odin odin] [--out fixtures-out]
//
// It runs in the workflow "Harness", which installs the toolchains toolchains.json pins; the Rust
// toolchain defaults to that pin, and `--rust-toolchain ""` takes `rustc` as it is.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { buildProgram, childEnv, dropExecutable, runCases, runProcess, verdictOf, type Tools } from "./build.ts";
import { parseSignature, type Lang } from "./languages.ts";
import { parseCase, parseTask, type Case } from "./tasks.ts";

const here = dirname(fileURLToPath(import.meta.url));
const EXTENSIONS: Record<string, string> = { rust: "rs", odin: "odin" };

function option(name: string, fallback: string): string {
  const at = process.argv.indexOf(`--${name}`);
  return at > 0 && at + 1 < process.argv.length ? process.argv[at + 1] : fallback;
}

async function main(): Promise<void> {
  const pins = JSON.parse(readFileSync(join(here, "toolchains.json"), "utf8"));
  const languages = option("languages", "rust,odin").split(",").map((l) => l.trim()) as Lang[];
  const out = option("out", "fixtures-out");
  const tools: Tools = {
    mn: "",
    native: false,
    rustc: option("rustc", "rustc"),
    rustToolchain: option("rust-toolchain", pins.rust.toolchain),
    odin: option("odin", "odin"),
  };
  for (const lang of languages) {
    if (!(lang in EXTENSIONS)) throw new Error(`no fixtures for ${lang}`);
  }
  if (languages.includes("rust")) {
    const env = childEnv(tools.rustToolchain ? { RUSTUP_TOOLCHAIN: tools.rustToolchain } : {});
    console.log((await runProcess(tools.rustc, ["--version"], { env, timeoutMs: 60_000 })).stdout.trim());
  }
  if (languages.includes("odin")) {
    console.log((await runProcess(tools.odin, ["version"], { timeoutMs: 60_000 })).stdout.trim());
  }
  const tasks = readdirSync(join(here, "..", "tasks"))
    .filter((f) => f.endsWith(".md"))
    .sort()
    .map((f) => parseTask(readFileSync(join(here, "..", "tasks", f), "utf8")))
    .filter((t) => t.milestone === "M1" && t.set === "public");
  const failures: string[] = [];
  let checked = 0;
  for (const lang of languages) {
    for (const task of tasks) {
      const path = join(here, "fixtures", lang, `${task.id}.${EXTENSIONS[lang]}`);
      if (!existsSync(path)) {
        if (task.kind === "solve") failures.push(`${lang} ${task.id}: no fixture at ${path}`);
        continue;
      }
      const program = readFileSync(path, "utf8").replace(/\r\n/g, "\n");
      const sig = parseSignature(task.signature);
      const dir = join(out, lang, task.id);
      checked++;
      if (task.kind === "adversarial") {
        const expected = /^\/\/ expect: (.+)$/m.exec(program)?.[1]?.trim();
        if (!expected) {
          failures.push(`${lang} ${task.id}: an adversarial fixture starts with // expect: <verdict>`);
          continue;
        }
        const probe: Case[] = task.probe.map((call) => ({ call, expected: "", value: true, leaves: false }));
        const build = await buildProgram(lang, dir, program, sig, probe, tools);
        const results = build.ok ? await runCases(build, sig, probe, tools) : [];
        dropExecutable(build);
        const verdict = verdictOf(build.ok, results);
        const detail = build.ok ? results.map((r) => `${r.call}: ${r.got}`).join("; ") : build.diagnostics.join("\n");
        if (verdict !== expected) failures.push(`${lang} ${task.id}: ${verdict}, expected ${expected}\n${detail}`);
        console.log(`${lang} ${task.id}: ${verdict} (${detail})`);
        continue;
      }
      const cases = [...task.examples, ...task.hidden].map(parseCase).filter((c): c is Case => c !== null);
      const build = await buildProgram(lang, dir, program, sig, cases, tools);
      if (!build.ok) {
        failures.push(`${lang} ${task.id}: does not build\n${build.diagnostics.join("\n")}`);
        continue;
      }
      const results = await runCases(build, sig, cases, tools);
      dropExecutable(build);
      const failed = results.filter((r) => !r.pass);
      for (const r of failed) failures.push(`${lang} ${task.id}: ${r.call} should give ${r.expected}, gave ${r.got}`);
      console.log(`${lang} ${task.id}: ${results.length - failed.length} of ${results.length} cases pass`);
    }
  }
  if (failures.length > 0) {
    console.error(`\n${failures.length} failures:\n${failures.join("\n")}`);
    process.exit(1);
  }
  console.log(`\n${checked} fixtures build through the harness and give what their tasks expect`);
}

await main();
