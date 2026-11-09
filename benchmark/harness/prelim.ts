// The preliminary runs of the M1 tasks (docs/design-review.md, early-core-benchmark and
// benchmark-difficulty).
//
// Agents write the public M1 solve tasks, the core ones or the others, from the language card,
// with repairs: after each
// program the harness runs `mn check` and, when it passes, the task's examples on the reference
// interpreter, and shows the agent what failed. A run ends when the program checks and passes
// every example, or after 10 repairs. The hidden tests then score the last program. It is a
// preliminary run, not a benchmark result, and it never opens the held-out set.
//
//   node benchmark/harness/prelim.ts --mn target/release/mn [--tasks core|non-core|<id>,<id>] [--runs 5]
//     [--budget 5] [--out prelim-out]
//
// It calls the Claude Messages API over HTTP with the key in ANTHROPIC_API_KEY. Each turn sends
// the conversation so far as it was, the model's earlier replies unchanged, and the run stops
// before a call that could take its spending past the budget, in US dollars.
import { spawnSync } from "node:child_process";
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  costOf,
  extractProgram,
  feedbackPrompt,
  hasTodo,
  outcome,
  parseCase,
  parseTask,
  preliminaryTasks,
  systemPrompt,
  userPrompt,
  type Case,
  type CaseResult,
  type Task,
} from "./tasks.ts";

const API_VERSION = "2026-10-21";
const MAX_TOKENS = 16000;
const MAX_REPAIRS = 10;

// The models, a mid tier and a weaker one, with their prices in US dollars per million tokens
// as Anthropic lists them on 2026-11-09 (Haiku's for prompts of 100K tokens or fewer).
const MODELS = [
  { id: "claude-sonnet-5-5", input: 2, output: 10 },
  { id: "claude-haiku-5-5", input: 0.1, output: 0.5 },
];

function option(name: string, fallback: string): string {
  const at = process.argv.indexOf(`--${name}`);
  return at > 0 && at + 1 < process.argv.length ? process.argv[at + 1] : fallback;
}

/** The public tasks of `dir`, in the order of their ids. */
function readTasks(dir: string): Task[] {
  return readdirSync(dir)
    .filter((f) => f.endsWith(".md"))
    .sort()
    .map((f) => parseTask(readFileSync(join(dir, f), "utf8")));
}

type Message = { role: "user" | "assistant"; content: unknown };

interface Reply {
  content: unknown[];
  text: string;
  stop_reason: string;
  usage: { input_tokens: number; output_tokens: number };
}

async function ask(apiKey: string, model: string, system: string, messages: Message[]): Promise<Reply> {
  for (let attempt = 0; ; attempt++) {
    const response = await fetch("https://api.anthropic.com/v1/messages", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-api-key": apiKey,
        "anthropic-version": API_VERSION,
      },
      body: JSON.stringify({ model, max_tokens: MAX_TOKENS, system, messages }),
    });
    if (response.ok) {
      const body = await response.json();
      const content = body.content ?? [];
      const text = content
        .filter((b: { type: string }) => b.type === "text")
        .map((b: { text: string }) => b.text)
        .join("\n");
      return { content, text, stop_reason: body.stop_reason, usage: body.usage };
    }
    const retryable = response.status === 429 || response.status >= 500;
    if (!retryable || attempt >= 3) {
      throw new Error(`the API answered ${response.status}: ${await response.text()}`);
    }
    const after = Number(response.headers.get("retry-after") ?? "0");
    await new Promise((done) => setTimeout(done, after > 0 ? after * 1000 : 2000 * 2 ** attempt));
  }
}

interface Reported {
  code: string;
  line: string;
}

/** What `mn check --json` reports for a module: each diagnostic's code, and its line with its fixes. */
function check(mn: string, file: string): Reported[] {
  const run = spawnSync(mn, ["check", "--json", file], { encoding: "utf8" });
  const doc = JSON.parse(run.stdout);
  return doc.diagnostics.map(
    (d: {
      code: string;
      message: string;
      primary: { line: number; column: number };
      fixes: { message: string }[];
    }) => ({
      code: d.code,
      line:
        `${d.primary.line}:${d.primary.column} ${d.code} ${d.message}` +
        d.fixes.map((f) => ` (fix: ${f.message})`).join(""),
    }),
  );
}

/** Runs each case on the reference interpreter. */
function runCases(mn: string, file: string, cases: Case[]): CaseResult[] {
  return cases.map((c) => {
    const run = spawnSync(mn, ["run", file, "--call", c.call], { encoding: "utf8", timeout: 60_000 });
    const got = outcome(c, run.status, run.stdout ?? "", run.stderr ?? "");
    return { ...c, got, pass: got === c.expected };
  });
}

/** The failure class `mn explain` gives a code. */
const classes = new Map<string, string>();
function failureClass(mn: string, code: string): string {
  if (!classes.has(code)) {
    const text = spawnSync(mn, ["explain", code], { encoding: "utf8" }).stdout ?? "";
    classes.set(code, /Failure class: ([^.]+)\./.exec(text)?.[1] ?? "unknown");
  }
  return classes.get(code) ?? "unknown";
}

async function main(): Promise<void> {
  const mn = option("mn", "target/release/mn");
  const which = option("tasks", "core");
  const runs = Number(option("runs", "5"));
  const budget = Number(option("budget", "5"));
  const out = option("out", "prelim-out");
  const apiKey = process.env.ANTHROPIC_API_KEY ?? "";
  if (apiKey === "") throw new Error("set ANTHROPIC_API_KEY");
  const card = readFileSync("docs/language-card.md", "utf8").replace(/^---\n[\s\S]*?\n---\n/, "");
  const system = systemPrompt(card);
  const tasks = preliminaryTasks(readTasks("benchmark/tasks"), which);
  mkdirSync(out, { recursive: true });
  const record = {
    models: MODELS,
    api_version: API_VERSION,
    max_tokens: MAX_TOKENS,
    max_repairs: MAX_REPAIRS,
    thinking: "adaptive, each model's default; display omitted",
    sampling: "the defaults; API version 2026-10-21 accepts no temperature",
    commit: process.env.GITHUB_SHA ?? "local",
    budget,
    tasks: which,
    system_prompt: system,
    runs: [] as object[],
    spent: 0,
    stopped_early: false,
  };
  outer: for (const model of MODELS) {
    for (const task of tasks) {
      const examples = task.examples.map(parseCase).filter((c): c is Case => c !== null);
      const hidden = task.hidden.map(parseCase).filter((c): c is Case => c !== null);
      for (let run = 1; run <= runs; run++) {
        const messages: Message[] = [{ role: "user", content: userPrompt(task) }];
        const attempts: object[] = [];
        let spent = 0;
        let program = "";
        let file = "";
        let finished = false;
        for (let attempt = 0; attempt <= MAX_REPAIRS && !finished; attempt++) {
          const chars = system.length + JSON.stringify(messages).length;
          const worst = ((chars / 3) * model.input + MAX_TOKENS * model.output) / 1e6;
          if (record.spent + worst > budget) {
            record.stopped_early = true;
            break outer;
          }
          const reply = await ask(apiKey, model.id, system, messages);
          const cost = costOf(reply.usage, model.input, model.output);
          record.spent += cost;
          spent += cost;
          // The reply goes back unchanged, so the transcript stays append-only.
          messages.push({ role: "assistant", content: reply.content });
          const name = `${model.id}-${task.id}-${run}-${attempt}`;
          program = extractProgram(reply.text);
          file = join(out, `${name}.mn`);
          writeFileSync(file, `module prelim\n\n${program}`);
          writeFileSync(join(out, `${name}.reply.md`), reply.text);
          const refused = reply.stop_reason === "refusal";
          const reported = refused ? [] : check(mn, file);
          const results = refused || reported.length > 0 ? [] : runCases(mn, file, examples);
          finished = refused || (reported.length === 0 && results.every((r) => r.pass));
          attempts.push({
            attempt,
            stop_reason: reply.stop_reason,
            usage: reply.usage,
            cost,
            file: `${name}.mn`,
            diagnostics: reported.map((d) => d.line),
            classes: reported.map((d) => failureClass(mn, d.code)),
            examples: results,
          });
          if (!finished && attempt < MAX_REPAIRS) {
            messages.push({
              role: "user",
              content: feedbackPrompt(
                reported.map((d) => d.line),
                results,
              ),
            });
          }
        }
        const builds = file !== "" && check(mn, file).length === 0;
        const scored = builds ? runCases(mn, file, [...examples, ...hidden]) : [];
        const solved = builds && !hasTodo(program) && scored.every((r) => r.pass);
        record.runs.push({
          model: model.id,
          task: task.id,
          run,
          solved,
          repairs: Math.max(attempts.length - 1, 0),
          builds,
          failed_cases: scored.filter((r) => !r.pass),
          cost: spent,
          attempts,
        });
        console.log(
          `${model.id} ${task.id}-${run}: ${solved ? "solved" : "unsolved"} after ${Math.max(attempts.length - 1, 0)} repairs, $${spent.toFixed(4)}`,
        );
      }
    }
  }
  writeFileSync(join(out, "results.json"), JSON.stringify(record, null, 2));
  console.log(
    `${record.runs.length} runs, $${record.spent.toFixed(2)} of $${budget}${record.stopped_early ? ", stopped at the budget" : ""}`,
  );
}

await main();
