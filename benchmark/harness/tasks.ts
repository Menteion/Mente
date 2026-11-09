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
  signature: string[];
  examples: string[];
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
    title,
    description: section(body, "Task").join("\n").trim(),
    signature: indented(section(body, "Signature")),
    examples: indented(section(body, "Examples")),
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
