// The syntax probe of M1's checkpoint (a) (docs/design-review.md, syntax-probe).
//
// Agents write the public M1 solve tasks from the draft language card, one program per run,
// with no repairs and no compiler feedback; `mn check` then reads every program, and the
// diagnostics show which constructs agents get wrong. Nothing is scored.
//
//   node benchmark/harness/probe.ts [--runs 3] [--budget 5] [--out probe-out]
//   node benchmark/harness/probe.ts --check probe-out --mn target/release/mn
//
// It runs in two jobs (.github/workflows/probe.yml): the first, which holds the API key, has the
// agents write the programs and runs none of them; the second, which holds no secret, runs
// `mn check` on each and records its diagnostics in results.json. Agent-written code never runs
// where the key is.
//
// It calls the Claude Messages API over HTTP with the key in ANTHROPIC_API_KEY, records the
// model, its parameters, every prompt and reply and the tokens used, and stops before a call
// that could take its spending past the budget, in US dollars.

import { spawnSync } from "node:child_process";
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  costOf,
  extractProgram,
  parseTask,
  systemPrompt,
  userPrompt,
  type Task,
} from "./tasks.ts";

const MODEL = "claude-sonnet-5-5";
const API_VERSION = "2026-10-21";
const MAX_TOKENS = 16000;
// US dollars per million tokens for MODEL, as Anthropic lists them on 2026-11-09.
const PRICE_INPUT = 2;
const PRICE_OUTPUT = 10;

function option(name: string, fallback: string): string {
  const at = process.argv.indexOf(`--${name}`);
  return at > 0 && at + 1 < process.argv.length ? process.argv[at + 1] : fallback;
}

/** The public M1 solve tasks, in the order of their ids. */
function m1Tasks(dir: string): Task[] {
  return readdirSync(dir)
    .filter((f) => f.endsWith(".md"))
    .sort()
    .map((f) => parseTask(readFileSync(join(dir, f), "utf8")))
    .filter((t) => t.milestone === "M1" && t.set === "public" && t.kind === "solve");
}

interface Reply {
  text: string;
  stop_reason: string;
  usage: { input_tokens: number; output_tokens: number };
}

async function ask(apiKey: string, system: string, user: string): Promise<Reply> {
  for (let attempt = 0; ; attempt++) {
    const response = await fetch("https://api.anthropic.com/v1/messages", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-api-key": apiKey,
        "anthropic-version": API_VERSION,
      },
      body: JSON.stringify({
        model: MODEL,
        max_tokens: MAX_TOKENS,
        system,
        messages: [{ role: "user", content: user }],
      }),
    });
    if (response.ok) {
      const body = await response.json();
      const text = (body.content ?? [])
        .filter((b: { type: string }) => b.type === "text")
        .map((b: { text: string }) => b.text)
        .join("\n");
      return { text, stop_reason: body.stop_reason, usage: body.usage };
    }
    const retryable = response.status === 429 || response.status >= 500;
    if (!retryable || attempt >= 3) {
      throw new Error(`the API answered ${response.status}: ${await response.text()}`);
    }
    const after = Number(response.headers.get("retry-after") ?? "0");
    await new Promise((done) => setTimeout(done, after > 0 ? after * 1000 : 2000 * 2 ** attempt));
  }
}

/** What `mn check --json` reports for a module, one `line:column code message` each. */
function check(mn: string, file: string): string[] {
  const run = spawnSync(mn, ["check", "--json", file], { encoding: "utf8" });
  const doc = JSON.parse(run.stdout);
  return doc.diagnostics.map(
    (d: { code: string; message: string; primary: { line: number; column: number } }) =>
      `${d.primary.line}:${d.primary.column} ${d.code} ${d.message}`,
  );
}

/** The second job: `mn check` on every program the first wrote, with no key in reach. */
function checkPrograms(dir: string, mn: string): void {
  if (process.env.ANTHROPIC_API_KEY) throw new Error("--check runs agent-written programs: run it without ANTHROPIC_API_KEY");
  const record = JSON.parse(readFileSync(join(dir, "results.json"), "utf8"));
  for (const p of record.programs) {
    p.diagnostics = p.stop_reason === "refusal" ? [] : check(mn, join(dir, p.file));
    console.log(`${p.file}: ${p.diagnostics.length} diagnostics`);
  }
  writeFileSync(join(dir, "results.json"), JSON.stringify(record, null, 2));
}

async function main(): Promise<void> {
  const checkDir = option("check", "");
  if (checkDir !== "") return checkPrograms(checkDir, option("mn", "target/release/mn"));
  const runs = Number(option("runs", "3"));
  const budget = Number(option("budget", "5"));
  const out = option("out", "probe-out");
  const apiKey = process.env.ANTHROPIC_API_KEY ?? "";
  if (apiKey === "") throw new Error("set ANTHROPIC_API_KEY");

  const card = readFileSync("docs/language-card.md", "utf8").replace(/^---\n[\s\S]*?\n---\n/, "");
  const system = systemPrompt(card);
  const tasks = m1Tasks("benchmark/tasks");
  mkdirSync(out, { recursive: true });

  const record = {
    model: MODEL,
    api_version: API_VERSION,
    max_tokens: MAX_TOKENS,
    thinking: "adaptive, the model's default; display omitted",
    effort: "high, the model's default",
    sampling: "the defaults; API version 2026-10-21 accepts no temperature",
    prices_per_million: { input: PRICE_INPUT, output: PRICE_OUTPUT },
    commit: process.env.GITHUB_SHA ?? "local",
    budget,
    system_prompt: system,
    programs: [] as object[],
    spent: 0,
    stopped_early: false,
  };
  const worstCall = (system.length / 3 + 2000) * PRICE_INPUT / 1e6 + MAX_TOKENS * PRICE_OUTPUT / 1e6;

  for (const task of tasks) {
    const user = userPrompt(task);
    for (let run = 1; run <= runs; run++) {
      if (record.spent + worstCall > budget) {
        record.stopped_early = true;
        break;
      }
      const reply = await ask(apiKey, system, user);
      const cost = costOf(reply.usage, PRICE_INPUT, PRICE_OUTPUT);
      record.spent += cost;
      const name = `${task.id}-${run}`;
      const module = `module probe\n\n${extractProgram(reply.text)}`;
      writeFileSync(join(out, `${name}.mn`), module);
      writeFileSync(join(out, `${name}.reply.md`), reply.text);
      record.programs.push({
        task: task.id,
        run,
        prompt: user,
        stop_reason: reply.stop_reason,
        usage: reply.usage,
        cost,
        file: `${name}.mn`,
        diagnostics: null,
      });
      console.log(`${name}: written, ${cost.toFixed(4)}`);
    }
    if (record.stopped_early) break;
  }

  writeFileSync(join(out, "results.json"), JSON.stringify(record, null, 2));
  console.log(
    `${record.programs.length} programs, $${record.spent.toFixed(2)} of $${budget}${record.stopped_early ? ", stopped at the budget" : ""}`,
  );
}

await main();
