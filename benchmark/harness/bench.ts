// The measured runs of the agent benchmark (benchmark/README.md, "Method"; ROADMAP.md, "Agent
// benchmark").
//
// Every task of the milestone, the public solve and adversarial tasks and, given `--held-out`,
// the held-out solve tasks, runs 5 times per language and model. In a run the agent submits a
// program; the harness builds it and runs the visible examples, and shows the agent what the
// compiler reported or each example's result; the agent revises the whole program, at most 10
// times. A run ends once its program builds and passes every example (an adversarial task's, once
// it builds), since the agent cannot see more, or after 10 repairs. The hidden tests then score
// its last program; an adversarial task's probe gives its verdict. The Mente agent's system prompt
// is the language card, the Odin agent's the control document, and the Rust agent's names the
// language only.
//
//   node benchmark/harness/bench.ts --budget <dollars> [--milestone M1] [--languages mente,rust,odin]
//     [--models claude-sonnet-5-5,claude-haiku-5-5] [--runs 5] [--tasks all|<id>,<id>]
//     [--held-out <dir>] [--concurrency 8] [--mn target/release/mn] [--native] [--rustc rustc]
//     [--odin odin] [--sandbox benchmark/harness/sandbox.sh] [--out bench-out] --broker <socket>
//
// Agent programs can read files and open sockets, so it runs only with --sandbox, which runs every
// agent build and agent program with least privilege (benchmark/harness/sandbox.sh): no network,
// and the API key out of reach. It refuses to run without it.
//
// It holds no API key, and refuses to start with one in its environment: it sends each call of the
// Claude Messages API to the broker at --broker (benchmark/harness/broker.ts), the one process that
// holds the key, which spends within the run's limits too. It stops before any call that could take
// its spending past the budget, in US dollars: each call reserves the most it can cost before it is
// sent. It writes `results.json` after every run, with the public runs in full
// and the held-out set only in the totals. The held-out runs' prompts, programs and records go to
// `held-out/` under the output directory, which the workflow never uploads, and the log names a
// held-out task only by its place in the set, so that its results leave the runner only as totals.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  API_VERSION,
  budgetedCall,
  MAX_IN_FLIGHT,
  MAX_TOKENS,
  parseModels,
  requestBody,
  type Budget,
  type Message,
  type Model,
  type Post,
  type Reply,
} from "./api.ts";
import { connectBroker } from "./brokered.ts";
import {
  BUILD_MODES,
  buildProgram,
  checkUnderRuns,
  childEnv,
  dropExecutable,
  failureClass,
  insideAny,
  readSandboxRoots,
  runCases,
  runProcess,
  rustcInSysroot,
  verdictOf,
  type Build,
  type Tools,
  type Verdict,
} from "./build.ts";
import { LANGUAGES, odinProgram, parseSignature, rustProgram, type Lang, type Signature } from "./languages.ts";
import { feedbackFor, hasTodoIn, systemPromptFor, userPromptFor, withoutFrontMatter } from "./prompts.ts";
import { firstFailure, summarize, type RunRecord } from "./report.ts";
import { benchmarkTasks, extractProgram, heldOutTasks, parseCase, parseTask, type Case, type CaseResult, type Task } from "./tasks.ts";
import { own, setOwn } from "../own.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const MAX_REPAIRS = 10;
const DOCUMENTS: Partial<Record<Lang, string>> = { mente: "docs/language-card.md", odin: "benchmark/control/odin.md" };

function option(name: string, fallback: string): string {
  const at = process.argv.indexOf(`--${name}`);
  return at > 0 && at + 1 < process.argv.length ? own(process.argv, at + 1) : fallback;
}

function readTasks(dir: string): Task[] {
  return readdirSync(dir)
    .filter((f) => f.endsWith(".md"))
    .sort()
    .map((f) => parseTask(readFileSync(join(dir, f), "utf8")));
}


interface Spec {
  task: Task;
  /** The run's name in the log and its directory: a held-out task is named by its place. */
  label: string;
  name: string;
  set: "public" | "held-out";
  lang: Lang;
  model: Model;
  run: number;
  sig: Signature;
  examples: Case[];
  scored: Case[];
  probe: Case[];
}

interface Context {
  post: Post;
  budget: Budget;
  systems: Partial<Record<Lang, string>>;
  tools: Tools;
  out: string;
}

/** One call within the budget, with the measured run's cache breakpoints: null when the budget stops it. */
function call(ctx: Context, spec: Spec, system: string, messages: Message[]): Promise<{ reply: Reply; cost: number } | null> {
  return budgetedCall(ctx.post, ctx.budget, requestBody(spec.model.id, system, messages), spec.model);
}

/** The failure classes of one program: its codes' classes or a build failure, else its cases' faults. */
async function classesOf(lang: Lang, build: Build, results: CaseResult[], tools: Tools): Promise<string[]> {
  if (!build.ok) {
    if (lang !== "mente") return ["build failure"];
    return [...new Set(await Promise.all(build.codes.map((c) => failureClass(tools, c))))];
  }
  return [...new Set(results.filter((r) => !r.pass).map((r) => r.fault ?? "wrong result"))];
}

interface Detailed extends RunRecord {
  refused: boolean;
  failed_cases: CaseResult[];
  attempts: object[];
}

/** One run of one task in one language with one model, or null when the budget stopped it. */
async function runOne(ctx: Context, spec: Spec): Promise<Detailed | null> {
  const system = own(ctx.systems, spec.lang) ?? "";
  const dir = join(ctx.out, spec.set === "public" ? "runs" : "held-out", spec.name);
  mkdirSync(dir, { recursive: true });
  const messages: Message[] = [{ role: "user", content: userPromptFor(spec.task, spec.lang) }];
  writeFileSync(join(dir, "prompt.md"), String(own(messages, 0).content));
  const solve = spec.task.kind === "solve";
  const attempts: object[] = [];
  const programs: { classes: string[] }[] = [];
  const tokens = { input: 0, output: 0 };
  let cost = 0;
  let program = "";
  let refused = false;
  let finished = false;
  for (let attempt = 0; attempt <= MAX_REPAIRS && !finished; attempt++) {
    const answered = await call(ctx, spec, system, messages);
    if (!answered) return null;
    const { reply } = answered;
    cost += answered.cost;
    tokens.input += reply.usage.input_tokens + (reply.usage.cache_creation_input_tokens ?? 0) + (reply.usage.cache_read_input_tokens ?? 0);
    tokens.output += reply.usage.output_tokens;
    // The reply goes back unchanged, so the transcript stays append-only.
    messages.push({ role: "assistant", content: reply.content });
    writeFileSync(join(dir, `${attempt}.reply.md`), reply.text);
    program = extractProgram(reply.text);
    refused = reply.stop_reason === "refusal";
    let build: Build | null = null;
    let results: CaseResult[] = [];
    if (!refused) {
      build = await buildProgram(spec.lang, join(dir, String(attempt)), program, spec.sig, solve ? spec.examples : [], ctx.tools);
      if (build.ok && solve) results = await runCases(build, spec.sig, spec.examples, ctx.tools);
      dropExecutable(build);
    }
    finished = refused || (build !== null && build.ok && results.every((r) => r.pass));
    const classes = build ? await classesOf(spec.lang, build, results, ctx.tools) : ["refusal"];
    programs.push({ classes });
    attempts.push({
      attempt,
      answered_by: reply.model,
      stop_reason: reply.stop_reason,
      usage: reply.usage,
      cost: answered.cost,
      builds: build?.ok ?? false,
      diagnostics: build?.diagnostics ?? [],
      classes,
      examples: results,
    });
    if (!finished && attempt < MAX_REPAIRS) {
      messages.push({ role: "user", content: feedbackFor(spec.lang, spec.sig, build?.diagnostics ?? [], results) });
    }
  }
  let solved: boolean | null = null;
  let verdict: Verdict | null = null;
  let failed: CaseResult[] = [];
  if (refused) {
    if (solve) solved = false;
  } else if (solve) {
    const scoring = await buildProgram(spec.lang, join(dir, "scored"), program, spec.sig, spec.scored, ctx.tools);
    const results = scoring.ok ? await runCases(scoring, spec.sig, spec.scored, ctx.tools) : [];
    dropExecutable(scoring);
    const todo = hasTodoIn(spec.lang, program);
    solved = scoring.ok && !todo && results.every((r) => r.pass);
    failed = results.filter((r) => !r.pass);
    const classes = await classesOf(spec.lang, scoring, results, ctx.tools);
    setOwn(programs, programs.length - 1, { classes: todo && classes.length === 0 ? ["todo"] : classes });
  } else {
    const probing = await buildProgram(spec.lang, join(dir, "probe"), program, spec.sig, spec.probe, ctx.tools);
    const results = probing.ok ? await runCases(probing, spec.sig, spec.probe, ctx.tools) : [];
    dropExecutable(probing);
    verdict = verdictOf(probing.ok, results);
    failed = results;
  }
  return {
    model: spec.model.id,
    language: spec.lang,
    task: spec.label,
    set: spec.set,
    kind: solve ? "solve" : "adversarial",
    core: spec.task.core,
    run: spec.run,
    solved,
    verdict,
    refused,
    repairs: Math.max(attempts.length - 1, 0),
    tokens,
    cost,
    programs,
    failed_cases: failed,
    attempts,
  };
}

/** Checks before any call that every task can be written and run in every language. */
function checkWritable(spec: Spec): void {
  userPromptFor(spec.task, spec.lang);
  const all = [...spec.scored, ...spec.probe];
  if (spec.lang === "rust") rustProgram(spec.sig, all);
  if (spec.lang === "odin") odinProgram(spec.sig, all);
}

function sha256(text: string): string {
  return createHash("sha256").update(text).digest("hex");
}

async function main(): Promise<void> {
  const budgetText = option("budget", "");
  if (!/^\d+(\.\d+)?$/.test(budgetText)) throw new Error("--budget <dollars> is required");
  const milestone = option("milestone", "M1");
  const languages = option("languages", LANGUAGES.join(","))
    .split(",")
    .map((l) => l.trim()) as Lang[];
  for (const l of languages) if (!LANGUAGES.includes(l)) throw new Error(`no language ${l}`);
  const models = parseModels(option("models", "claude-sonnet-5-5,claude-haiku-5-5"));
  const runs = Number(option("runs", "5"));
  const concurrency = Number(option("concurrency", "8"));
  if (!Number.isInteger(concurrency) || concurrency < 1 || concurrency > MAX_IN_FLIGHT) throw new Error(`--concurrency ${concurrency}: from 1 to ${MAX_IN_FLIGHT}, the most requests the broker sends at once (broker-limits.json)`);
  const which = option("tasks", "all");
  const heldOutDir = option("held-out", "");
  const out = option("out", "bench-out");
  const plan = process.argv.includes("--plan");
  if (plan && heldOutDir !== "") throw new Error("--plan never opens the held-out set: drop --held-out");
  const pins = JSON.parse(readFileSync(join(here, "toolchains.json"), "utf8"));
  const tools: Tools = {
    mn: option("mn", "target/release/mn"),
    native: process.argv.includes("--native"),
    rustc: option("rustc", "rustc"),
    rustToolchain: option("rust-toolchain", pins.rust.toolchain),
    odin: option("odin", "odin"),
    sandbox: option("sandbox", ""),
    // The output directory, under the sandbox's runs root, which it covers all but each build's own
    // directory, so no build reads another run's folder: a public program never sees a held-out
    // run's prompts, programs or records.
    out: resolve(out),
  };
  if (tools.sandbox) {
    const roots = readSandboxRoots();
    checkUnderRuns(out, roots);
    // The held-out checkout lies in a directory the sandbox hides, the workspace, so that a program
    // written for a public task cannot read a held-out task and print it.
    if (heldOutDir && !insideAny(heldOutDir, roots.hide)) throw new Error(`--held-out must lie in a directory the sandbox hides, ${roots.hide.join(" or ")}; ${resolve(heldOutDir)} does not`);
  }
  // The broker holds the key; this process starts agent code, and holds none.
  if (process.env.ANTHROPIC_API_KEY) throw new Error("ANTHROPIC_API_KEY is set: the harness holds no key, the broker does (benchmark/harness/broker.ts)");
  const brokerAt = option("broker", "");
  if (!plan && brokerAt === "") throw new Error("pass --broker <socket>: the broker sends the calls, with the key");
  // Agent programs run beside the broker, so never outside the sandbox: an empty --sandbox once meant
  // every build ran unconfined.
  if (!plan && !tools.sandbox) throw new Error("pass --sandbox: the agents' programs run on the machine whose broker holds the API key");

  // The toolchains, checked against their pins and recorded; a plan starts none.
  const versions: Partial<Record<Lang, string>> = {};
  if (!plan && languages.includes("mente")) versions.mente = `mn at ${process.env.GITHUB_SHA ?? "a local build"}`;
  if (!plan && languages.includes("rust")) {
    tools.rustc = await rustcInSysroot(tools.rustc, tools.rustToolchain);
    const env = childEnv(tools.rustToolchain ? { RUSTUP_TOOLCHAIN: tools.rustToolchain } : {});
    versions.rust = (await runProcess(tools.rustc, ["--version"], { env, timeoutMs: 60_000 })).stdout.trim();
    if (!versions.rust.startsWith(`rustc ${pins.rust.toolchain} `)) throw new Error(`rustc is ${versions.rust}, not ${pins.rust.toolchain}`);
  }
  if (!plan && languages.includes("odin")) {
    versions.odin = (await runProcess(tools.odin, ["version"], { timeoutMs: 60_000 })).stdout.trim();
    if (!versions.odin.includes(pins.odin.release)) throw new Error(`odin is ${versions.odin}, not ${pins.odin.release}`);
  }

  // The documents and system prompts.
  const systems: Partial<Record<Lang, string>> = {};
  const documents: Record<string, { path: string; sha256: string; characters: number } | null> = {};
  for (const lang of languages) {
    const path = own(DOCUMENTS, lang);
    if (path && !existsSync(path)) throw new Error(`the ${lang} agent's document ${path} is missing`);
    const doc = path ? withoutFrontMatter(readFileSync(path, "utf8")) : "";
    setOwn(systems, lang, systemPromptFor(lang, doc, { rust: pins.rust.toolchain, odin: pins.odin.release }));
    setOwn(documents, lang, path ? { path, sha256: sha256(readFileSync(path, "utf8")), characters: doc.length } : null);
  }

  // The tasks: the milestone's public solve and adversarial tasks, then its held-out solve tasks.
  const publicDir = join(here, "..", "tasks");
  const publicTasks = benchmarkTasks(readTasks(publicDir), milestone, which);
  const heldTasks = heldOutDir ? heldOutTasks(readTasks(join(heldOutDir, "tasks")), milestone) : [];
  const specs: Spec[] = [];
  for (let run = 1; run <= runs; run++) {
    const all: [Task, "public" | "held-out", string][] = [
      ...publicTasks.map((t): [Task, "public", string] => [t, "public", t.id]),
      ...heldTasks.map((t, k): [Task, "held-out", string] => [t, "held-out", `held-out-${k + 1}`]),
    ];
    for (const [task, set, label] of all) {
      for (const model of models) {
        for (const lang of languages) {
          const cases = (lines: string[]) => lines.map(parseCase).filter((c): c is Case => c !== null);
          const spec: Spec = {
            task,
            label,
            name: `${label}-${lang}-${model.id}-${run}`,
            set,
            lang,
            model,
            run,
            sig: parseSignature(task.signature),
            examples: cases(task.examples),
            scored: cases([...task.examples, ...task.hidden]),
            probe: task.probe.map((call) => ({ call, expected: "", value: true, leaves: false })),
          };
          try {
            checkWritable(spec);
          } catch (e) {
            // A held-out task's text never reaches the log.
            throw new Error(set === "public" ? `${task.id} in ${lang}: ${(e as Error).message}` : `${label} cannot be written in ${lang}`);
          }
          specs.push(spec);
        }
      }
    }
  }

  // A plan: the tasks and runs a run would make, chosen as a run chooses them, then printed, with no
  // broker, no key and no agent code, before anything is written. A harness test makes it on every
  // change, so that this script's path up to its first call runs in CI (run 3 of the plan, on
  // 2026-11-16: the benchmark's choice of tasks threw before any call, with the key in reach, and no
  // test had ever run it).
  if (plan) {
    console.log(JSON.stringify({ plan: "bench", milestone, models: models.map((m) => m.id), languages, runs, tasks: publicTasks.map((t) => t.id), planned_runs: specs.length }));
    return;
  }

  // The runs' folders exist before the first build, so that the sandbox has them to cover.
  mkdirSync(join(out, "runs"), { recursive: true });
  mkdirSync(join(out, "held-out"), { recursive: true });
  const budget: Budget = { limit: Number(budgetText), spent: 0, reserved: 0, stopped: false };
  const broker = await connectBroker(brokerAt);
  const ctx: Context = { post: (body) => broker.post(body), budget, systems, tools, out };
  const done: Detailed[] = [];
  let failure = "";
  let written = 0;
  const write = (force: boolean) => {
    // The whole record is written at most every 30 seconds, and at the end.
    if (!force && Date.now() - written < 30_000) return;
    written = Date.now();
    const record = {
      benchmark: "agent benchmark, measured run",
      date: new Date().toISOString(),
      commit: process.env.GITHUB_SHA ?? "local",
      milestone,
      languages,
      models,
      runs_per_task: runs,
      max_repairs: MAX_REPAIRS,
      max_tokens: MAX_TOKENS,
      api_version: API_VERSION,
      thinking: "adaptive, each model's default effort; display omitted",
      sampling: "the defaults; API version 2026-10-21 accepts no temperature",
      fallback: "none: every program comes from the model named, and a refusal is recorded as one",
      prompt_caching: "the system prompt and the conversation, 5-minute entries; it changes the cost, not the prompts",
      toolchains: versions,
      build_modes: Object.fromEntries(languages.map((l) => [l, own(BUILD_MODES, l)])),
      documents,
      system_prompts: systems,
      budget: budget.limit,
      spent: budget.spent,
      stopped_at_budget: budget.stopped && failure === "",
      failure,
      tasks: { public: publicTasks.map((t) => t.id), held_out: heldTasks.length },
      planned_runs: specs.length,
      finished_runs: done.length,
      summary: summarize(done, runs),
      runs: done.filter((r) => r.set === "public"),
    };
    writeFileSync(join(out, "results.json"), JSON.stringify(record, null, 2));
    if (heldTasks.length > 0) {
      mkdirSync(join(out, "held-out"), { recursive: true });
      writeFileSync(join(out, "held-out", "results.json"), JSON.stringify(done.filter((r) => r.set === "held-out"), null, 2));
    }
  };
  console.log(`${specs.length} runs planned: ${publicTasks.length} public and ${heldTasks.length} held-out tasks, ${languages.length} languages, ${models.length} models, ${runs} runs each; budget $${budget.limit}`);
  let next = 0;
  const worker = async () => {
    while (!budget.stopped && next < specs.length) {
      const spec = own(specs, next++);
      try {
        const record = await runOne(ctx, spec);
        if (!record) break;
        done.push(record);
        writeFileSync(join(out, spec.set === "public" ? "runs" : "held-out", spec.name, "record.json"), JSON.stringify(record, null, 2));
        write(false);
        const outcome = record.kind === "solve" ? (record.solved ? "solved" : "unsolved") : (record.verdict ?? "refused");
        if (spec.set === "public") {
          console.log(`[${done.length}/${specs.length}] ${spec.name}: ${outcome} after ${record.repairs} repairs, $${record.cost.toFixed(4)}`);
        } else console.log(`[${done.length}/${specs.length}] a held-out run finished`);
      } catch (e) {
        // The first failure stands; a held-out run's names only the run's anomaly, the API status or
        // the error's kind, never the task.
        failure = firstFailure(failure, e, spec.set, spec.name);
        budget.stopped = true;
      }
    }
  };
  await Promise.all(Array.from({ length: Math.max(concurrency, 1) }, worker));
  broker.close();
  write(true);
  console.log(
    `${done.length} of ${specs.length} runs, $${budget.spent.toFixed(2)} of $${budget.limit}` +
      (failure ? `; stopped by an error: ${failure}` : budget.stopped ? ", stopped at the budget" : ""),
  );
  if (failure) process.exit(1);
}

await main();
