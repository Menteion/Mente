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
//   node benchmark/harness/prelim.ts --sandbox benchmark/harness/sandbox.sh --mn <mn> --out <dir>
//     --broker <socket> [--tasks core|non-core|<id>,<id>] [--runs 5] [--budget 5]
//
// It holds no API key, and refuses to start with one in its environment: the broker at --broker
// (benchmark/harness/broker.ts), the one process that holds the key, sends each call. Every `mn`
// call on an agent's program runs through the sandbox (benchmark/harness/sandbox.sh), with no secret
// in its environment: agent-written code never runs where the key is. The job proves the sandbox with sandbox-check.ts first
// (.github/workflows/prelim.yml). The sandbox hides the repository, so --mn and --out lie outside it.
//
// Each turn sends the conversation so far as it was, the model's earlier replies unchanged, and the run stops
// before a call that could take its spending past the budget, in US dollars.
import { spawnSync } from "node:child_process";
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { REPOSITORY, checkUnderRuns, childEnv, insideAny, namedCommand, readSandboxRoots, sandboxed } from "./build.ts";
import {
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
import { API_VERSION, budgetedCall, KNOWN_MODELS, MAX_TOKENS, type Budget, type Message, type Post } from "./api.ts";
import { connectBroker } from "./brokered.ts";
import { own } from "../own.mjs";

const MAX_REPAIRS = 10;

// The models, a mid tier and a weaker one, with their prices from broker-limits.json (api.ts).
const MODELS = [own(KNOWN_MODELS, "claude-sonnet-5-5"), own(KNOWN_MODELS, "claude-haiku-5-5")];

function option(name: string, fallback: string): string {
  const at = process.argv.indexOf(`--${name}`);
  return at > 0 && at + 1 < process.argv.length ? own(process.argv, at + 1) : fallback;
}

/** The public tasks of `dir`, in the order of their ids. */
function readTasks(dir: string): Task[] {
  return readdirSync(dir)
    .filter((f) => f.endsWith(".md"))
    .sort()
    .map((f) => parseTask(readFileSync(join(dir, f), "utf8")));
}

interface Reported {
  code: string;
  line: string;
}

/** The sandbox and the directory every `mn` call runs in, set once by main(). */
const confined = { sandbox: "", workdir: "" };

/**
 * Runs `mn` on an agent's program: through the benchmark's own sandbox, the one command this
 * process starts beside the key (`namedCommand`), with no secret in its environment.
 */
function mnSync(mn: string, args: string[], timeout?: number) {
  if (!confined.sandbox) throw new Error("an agent's program would run outside the sandbox");
  const [cmd, argv] = sandboxed(confined.sandbox, confined.workdir, mn, args);
  if (!namedCommand(cmd, argv)) throw new Error(`${confined.sandbox} is not the benchmark's sandbox, benchmark/harness/sandbox.sh`);
  return spawnSync(cmd, argv, { encoding: "utf8", cwd: confined.workdir, env: childEnv(), ...(timeout ? { timeout } : {}) });
}

/** What `mn check --json` reports for a module: each diagnostic's code, and its line with its fixes. */
function check(mn: string, file: string): Reported[] {
  const run = mnSync(mn, ["check", "--json", file]);
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
    const run = mnSync(mn, ["run", file, "--call", c.call], 60_000);
    const got = outcome(c, run.status, run.stdout ?? "", run.stderr ?? "");
    return { ...c, got, pass: got === c.expected };
  });
}

/** The failure class `mn explain` gives a code. */
const classes = new Map<string, string>();
function failureClass(mn: string, code: string): string {
  if (!classes.has(code)) {
    const text = mnSync(mn, ["explain", code]).stdout ?? "";
    classes.set(code, own(/Failure class: ([^.]+)\./.exec(text), 1) ?? "unknown");
  }
  return classes.get(code) ?? "unknown";
}

async function main(): Promise<void> {
  const mn = resolve(option("mn", "target/release/mn"));
  const which = option("tasks", "core");
  const runs = Number(option("runs", "5"));
  const budget = Number(option("budget", "5"));
  const out = resolve(option("out", "prelim-out"));
  const sandbox = option("sandbox", "");
  const plan = process.argv.includes("--plan");
  // The broker holds the key; this process runs agent code through the sandbox, and holds none.
  if (process.env.ANTHROPIC_API_KEY) throw new Error("ANTHROPIC_API_KEY is set: the harness holds no key, the broker does (benchmark/harness/broker.ts)");
  const brokerAt = option("broker", "");
  if (!plan && brokerAt === "") throw new Error("pass --broker <socket>: the broker sends the calls, with the key");
  if (!plan && !sandbox) throw new Error("pass --sandbox: the agents' programs run on the machine whose broker holds the API key");
  if (!plan) {
    // --out under the sandbox's runs root, whose cover leaves it alone visible; --mn outside every
    // directory the sandbox covers, or mn could not run there.
    const roots = readSandboxRoots();
    checkUnderRuns(out, roots);
    if (insideAny(mn, [REPOSITORY, ...roots.hide, roots.runs])) throw new Error(`--mn must lie outside ${[...roots.hide, roots.runs].join(" and ")}, which the sandbox covers`);
    confined.sandbox = sandbox;
    confined.workdir = out;
  }
  const card = readFileSync("docs/language-card.md", "utf8").replace(/^---\n[\s\S]*?\n---\n/, "");
  const system = systemPrompt(card);
  const tasks = preliminaryTasks(readTasks("benchmark/tasks"), which);
  // A plan: the tasks and runs a run would make, chosen as a run chooses them, then printed, with no
  // broker, no key and no agent code, before anything is written. A harness test makes it on every
  // change, so that this script's path up to its first call runs in CI (run 3 of the plan, on
  // 2026-11-16: the benchmark's choice of tasks threw before any call, with the key in reach, and no
  // test had ever run it).
  if (plan) {
    console.log(JSON.stringify({ plan: "prelim", models: MODELS.map((m) => m.id), runs, tasks: tasks.map((t) => t.id), planned_runs: MODELS.length * tasks.length * runs }));
    return;
  }
  const broker = await connectBroker(brokerAt);
  const post: Post = (body) => broker.post(body);
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
    failure: "",
  };
  const spending: Budget = { limit: budget, spent: 0, reserved: 0, stopped: false };
  let failure = "";
  try {
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
            const answered = await budgetedCall(post, spending, { model: model.id, max_tokens: MAX_TOKENS, system, messages }, model);
            if (!answered) {
              record.stopped_early = true;
              break outer;
            }
            const { reply, cost } = answered;
            record.spent = spending.spent;
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
              answered_by: reply.model,
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
    `${record.runs.length} runs, $${record.spent.toFixed(2)} of $${budget}${failure ? `; stopped by an error: ${failure}` : record.stopped_early ? ", stopped at the budget" : ""}`,
  );
  if (failure) process.exit(1);
}

await main();
