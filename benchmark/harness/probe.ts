// The syntax probe of M1's checkpoint (a) (docs/design-review.md, syntax-probe).
//
// Agents write the public M1 solve tasks from the draft language card, one program per run,
// with no repairs and no compiler feedback; `mn check` then reads every program, and the
// diagnostics show which constructs agents get wrong. Nothing is scored.
//
//   node benchmark/harness/probe.ts --broker <socket> [--runs 3] [--budget 5] [--out probe-out]
//   node benchmark/harness/probe.ts --check probe-out --mn target/release/mn
//
// It runs in two jobs (.github/workflows/probe.yml): the first, whose broker holds the API key, has
// the agents write the programs and runs none of them; the second, which holds no secret, runs
// `mn check` on each and records its diagnostics in results.json. Agent-written code never runs
// where the key is.
//
// The writer holds no API key, and refuses to start with one in its environment: the broker at
// --broker (benchmark/harness/broker.ts), the one process that holds the key, sends each call. It
// records the model, its parameters, every prompt and reply and the tokens used, and stops before a call
// that could take its spending past the budget, in US dollars.

import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { runProcess } from "./build.ts";
import {
  extractProgram,
  parseTask,
  systemPrompt,
  userPrompt,
  type Task,
} from "./tasks.ts";
import { API_VERSION, budgetedCall, KNOWN_MODELS, MAX_TOKENS, type Budget, type Post } from "./api.ts";
import { connectBroker } from "./brokered.ts";
import { own } from "../own.mjs";

const MODEL = "claude-sonnet-5-5";
// MODEL's prices, in US dollars per million tokens, from broker-limits.json (api.ts).
const PRICES = own(KNOWN_MODELS, MODEL);

function option(name: string, fallback: string): string {
  const at = process.argv.indexOf(`--${name}`);
  return at > 0 && at + 1 < process.argv.length ? own(process.argv, at + 1) : fallback;
}

/** The public M1 solve tasks, in the order of their ids. */
function m1Tasks(dir: string): Task[] {
  return readdirSync(dir)
    .filter((f) => f.endsWith(".md"))
    .sort()
    .map((f) => parseTask(readFileSync(join(dir, f), "utf8")))
    .filter((t) => t.milestone === "M1" && t.set === "public" && t.kind === "solve");
}

/**
 * What `mn check --json` reports for a module, one `line:column code message` each, through the
 * harness's one way to start a process, which keeps every secret from it.
 */
async function check(mn: string, file: string): Promise<string[]> {
  const run = await runProcess(mn, ["check", "--json", file], { timeoutMs: 120_000 });
  const doc = JSON.parse(run.stdout);
  return doc.diagnostics.map(
    (d: { code: string; message: string; primary: { line: number; column: number } }) =>
      `${d.primary.line}:${d.primary.column} ${d.code} ${d.message}`,
  );
}

/** The second job: `mn check` on every program the first wrote, with no key in reach. */
async function checkPrograms(dir: string, mn: string): Promise<void> {
  if (process.env.ANTHROPIC_API_KEY) throw new Error("--check runs agent-written programs: run it without ANTHROPIC_API_KEY");
  const record = JSON.parse(readFileSync(join(dir, "results.json"), "utf8"));
  for (const p of record.programs) {
    p.diagnostics = p.stop_reason === "refusal" ? [] : await check(mn, join(dir, p.file));
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
  // The broker holds the key; this process holds none.
  if (process.env.ANTHROPIC_API_KEY) throw new Error("ANTHROPIC_API_KEY is set: the harness holds no key, the broker does (benchmark/harness/broker.ts)");
  const brokerAt = option("broker", "");
  if (brokerAt === "") throw new Error("pass --broker <socket>: the broker sends the calls, with the key");
  const broker = await connectBroker(brokerAt);
  const post: Post = (body) => broker.post(body);

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
    prices_per_million: { input: PRICES.input, output: PRICES.output },
    commit: process.env.GITHUB_SHA ?? "local",
    budget,
    system_prompt: system,
    programs: [] as object[],
    spent: 0,
    stopped_early: false,
    failure: "",
  };
  const spending: Budget = { limit: budget, spent: 0, reserved: 0, stopped: false };

  let failure = "";
  try {
    for (const task of tasks) {
      const user = userPrompt(task);
      for (let run = 1; run <= runs; run++) {
        const answered = await budgetedCall(post, spending, { model: MODEL, max_tokens: MAX_TOKENS, system, messages: [{ role: "user", content: user }] }, PRICES);
        if (!answered) {
          record.stopped_early = true;
          break;
        }
        const { reply, cost } = answered;
        record.spent = spending.spent;
        const name = `${task.id}-${run}`;
        const source = `module probe\n\n${extractProgram(reply.text)}`;
        writeFileSync(join(out, `${name}.mn`), source);
        writeFileSync(join(out, `${name}.reply.md`), reply.text);
        record.programs.push({
          task: task.id,
          run,
          prompt: user,
          answered_by: reply.model,
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
  } catch (e) {
    // A failed call, an answer the broker refused as another model's among them, ends the run, and
    // its record says so: no result stands for a model that did not write it.
    failure = (e as Error).message;
  }
  broker.close();
  record.spent = spending.spent;
  record.failure = failure;
  writeFileSync(join(out, "results.json"), JSON.stringify(record, null, 2));
  console.log(
    `${record.programs.length} programs, $${record.spent.toFixed(2)} of $${budget}${failure ? `; stopped by an error: ${failure}` : record.stopped_early ? ", stopped at the budget" : ""}`,
  );
  if (failure) process.exit(1);
}

await main();
