// Reading benchmark tasks and writing them for Mente (benchmark/README.md).
//
// A task file has front matter, then `## Task`, `## Signature`, `## Examples` and
// `## Hidden tests`. An agent is shown the first three and never the hidden tests. The
// signature is in the neutral notation, which this file maps to Mente for the tasks M1 can
// run: integer types, `bool`, `[N]T`, `mut` parameters, `T in a..b` refinements, and
// `requires` / `ensures` lines.

export interface Task {
  id: string;
  milestone: string;
  set: string;
  kind: string;
  title: string;
  description: string;
  /** Whether the signature uses only the core of the language (benchmark/README.md). */
  core: boolean;
  signature: string[];
  examples: string[];
  /** The hidden tests, which only scoring reads and no prompt ever shows. */
  hidden: string[];
}

/**
 * The public M1 solve tasks a preliminary run gives the agent: `core`, the others with
 * `non-core`, or those a comma-separated list of ids names; an id that names no such task is an
 * error.
 */
export function preliminaryTasks(tasks: Task[], which: string): Task[] {
  const m1 = tasks.filter((t) => t.milestone === "M1" && t.set === "public" && t.kind === "solve");
  if (which === "core" || which === "non-core") {
    return m1.filter((t) => t.core === (which === "core"));
  }
  const ids = which.split(",").map((id) => id.trim());
  const unknown = ids.filter((id) => !m1.some((t) => t.id === id));
  if (unknown.length > 0) throw new Error(`no public M1 solve task ${unknown.join(", ")}`);
  return m1.filter((t) => ids.includes(t.id));
}

/** Reads one task file. */
export function parseTask(text: string): Task {
  const match = /^---\n([\s\S]*?)\n---\n([\s\S]*)$/.exec(text.replace(/\r\n/g, "\n"));
  if (!match) throw new Error("a task starts with front matter between --- lines");
  const front: Record<string, string> = {};
  for (const line of match[1].split("\n")) {
    const kv = /^(\w+):\s*(.*)$/.exec(line);
    if (kv) front[kv[1]] = kv[2].trim();
  }
  const body = match[2];
  const title = (/^# (.+)$/m.exec(body) ?? ["", ""])[1].trim();
  return {
    id: front.id ?? "",
    milestone: front.milestone ?? "",
    set: front.set ?? "",
    kind: front.kind ?? "",
    core: front.core === "true",
    title,
    description: section(body, "Task").join("\n").trim(),
    signature: indented(section(body, "Signature")),
    examples: indented(section(body, "Examples")),
    hidden: indented(section(body, "Hidden tests")),
  };
}

/** The lines of a `## name` section, up to the next heading of that level. */
function section(body: string, name: string): string[] {
  const lines = body.split("\n");
  const start = lines.indexOf(`## ${name}`);
  if (start < 0) return [];
  const rest = lines.slice(start + 1);
  const end = rest.findIndex((l) => l.startsWith("## "));
  return end < 0 ? rest : rest.slice(0, end);
}

/** The code lines of a section, written indented by four spaces. */
function indented(lines: string[]): string[] {
  return lines.filter((l) => l.startsWith("    ")).map((l) => l.slice(4));
}

/** A neutral type as Mente writes it: `T in a..b` becomes `T[a..b]`. */
export function menteType(neutral: string): string {
  const refined = /^(\w+) in (-?\w+)(\.\.=?)(-?\w+)$/.exec(neutral.trim());
  if (refined) return `${refined[1]}[${refined[2]}${refined[3]}${refined[4]}]`;
  const t = neutral.trim();
  if (!/^(\[\d+\])*(i8|i16|i32|i64|i128|u8|u16|u32|u64|u128|usize|bool)$/.test(t)) {
    throw new Error(`\`${t}\` is not a type M1 has`);
  }
  return t;
}

/** A task's neutral signature as a Mente function header with its contracts. */
export function menteSignature(neutral: string[]): string {
  const head = /^(\w+)\((.*)\)(?:\s*->\s*(.+))?$/.exec(neutral[0] ?? "");
  if (!head) throw new Error(`not a signature: ${neutral[0]}`);
  const params = head[2].trim() === ""
    ? []
    : head[2].split(",").map((p) => {
        const param = /^\s*(mut\s+)?(\w+):\s*(.+)$/.exec(p);
        if (!param) throw new Error(`not a parameter: ${p}`);
        return `${param[1] ? "mut " : ""}${param[2]}: ${menteType(param[3])}`;
      });
  const result = head[3] ? ` -> ${menteType(head[3])}` : "";
  const lines = [`pub fn ${head[1]}(${params.join(", ")})${result}`];
  for (const clause of neutral.slice(1)) {
    if (!/^(requires|ensures) /.test(clause)) throw new Error(`not a contract: ${clause}`);
    lines.push(`  ${clause}`);
  }
  return lines.join("\n");
}

/** The program in an agent's reply: its first fenced block, or the whole reply. */
export function extractProgram(reply: string): string {
  const fenced = /```[a-z]*\n([\s\S]*?)```/.exec(reply);
  return (fenced ? fenced[1] : reply).trim() + "\n";
}

export interface Usage {
  input_tokens: number;
  output_tokens: number;
}

/** What a call cost, in US dollars, at the given prices per million tokens. */
export function costOf(usage: Usage, inputPerMillion: number, outputPerMillion: number): number {
  return (usage.input_tokens * inputPerMillion + usage.output_tokens * outputPerMillion) / 1e6;
}

/** The system prompt: the language card is all the agent knows of Mente. */
export function systemPrompt(card: string): string {
  return [
    "You write code in Mente, a new programming language that no model has been trained on.",
    "Everything you know about Mente is in its language card, below. Use only what it describes.",
    "",
    card.trim(),
  ].join("\n");
}

/** The prompt for one task: its description, Mente signature and examples, never its hidden tests. */
export function userPrompt(task: Task): string {
  return [
    `# ${task.title}`,
    "",
    task.description,
    "",
    "## Signature",
    "",
    "```mente",
    menteSignature(task.signature),
    "```",
    "",
    "## Examples",
    "",
    "Each example calls the function. `== v` is the value it must return; `leaves v` is the value its `mut` argument holds after the call.",
    "",
    "```text",
    ...task.examples,
    "```",
    "",
    "Write the function, with exactly this signature, in one ```mente block. Write only the function: no module line, no other functions and no explanation.",
  ].join("\n");
}

/**
 * One example or hidden test: a call, and what it must print, written as the test writes it after
 * the call: `v` for `== v`, `w` for `leaves w`, and `v leaves w` for both.
 */
export interface Case {
  call: string;
  expected: string;
  /** `== v`: the value the call returns. */
  value: boolean;
  /** `leaves w`: the values the call's `mut` arguments hold afterwards. */
  leaves: boolean;
}

/** Reads `f(args) == v`, `f(mut a) leaves w` or `f(mut a) == v leaves w`. */
export function parseCase(line: string): Case | null {
  const eq = line.indexOf(" == ");
  const lv = line.lastIndexOf(" leaves ");
  const call = (end: number) => line.slice(0, end).trim();
  if (eq > 0 && lv > eq) {
    const v = line.slice(eq + 4, lv).trim();
    const w = line.slice(lv + 8).trim();
    return { call: call(eq), expected: `${v} leaves ${w}`, value: true, leaves: true };
  }
  if (eq > 0) return { call: call(eq), expected: line.slice(eq + 4).trim(), value: true, leaves: false };
  if (lv > 0) return { call: call(lv), expected: line.slice(lv + 8).trim(), value: false, leaves: true };
  return null;
}

/**
 * What `mn run --call` printed, in the case's own form: its result, what its `mut` arguments
 * hold, or both; or the trap.
 */
export function outcome(c: Case, status: number | null, stdout: string, stderr: string): string {
  if (status !== 0) {
    const last = stderr.trim().split(/\r?\n/).pop() ?? "";
    // Drop the file name `mn run` puts first.
    return last.replace(/^.*?: (?=trap\[|no function|the run|an array)/, "");
  }
  const lines = stdout.trim().split(/\r?\n/);
  const left = lines
    .filter((l) => l.startsWith("leaves "))
    .map((l) => l.slice("leaves ".length))
    .join(", ");
  const result = lines[0] ?? "";
  if (c.value && c.leaves) return `${result} leaves ${left}`;
  return c.leaves ? left : result;
}

/** One case's result, as the agent's feedback shows it. */
export interface CaseResult extends Case {
  got: string;
  pass: boolean;
}

/** The feedback after a program: what `mn check` reported, else each example's result. */
export function feedbackPrompt(diagnostics: string[], results: CaseResult[]): string {
  const lines: string[] = [];
  if (diagnostics.length > 0) {
    lines.push("`mn check` rejects the program:", "", "```text", ...diagnostics, "```");
  } else {
    lines.push("The program checks. Its examples ran on the reference interpreter:", "", "```text");
    for (const r of results) {
      const verb = r.value ? "==" : "leaves";
      lines.push(`${r.pass ? "PASS" : "FAIL"} ${r.call} ${verb} ${r.expected}${r.pass ? "" : `, got ${r.got}`}`);
    }
    lines.push("```");
  }
  lines.push(
    "",
    "Revise the function and reply with all of it, with exactly the same signature, in one ```mente block.",
  );
  return lines.join("\n");
}

/** Whether a program still holds a `todo`, which scores as unsolved (benchmark/README.md). */
export function hasTodo(program: string): boolean {
  return /\btodo\s*\(/.test(program);
}
