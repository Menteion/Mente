// The prompts of the measured benchmark run, in each language (benchmark/README.md, "Method").
//
// The Mente agent's prompts are the preliminary runs' (tasks.ts): the language card as the system
// prompt, the task with its Mente signature and its examples in the neutral notation, and after
// each program what `mn check` reports or each example's result. The Rust and Odin agents get the
// same task with the signature, types and examples written in their language, and after each
// program what the compiler reports or each example's result. The Odin agent's system prompt
// carries its control document, of the language card's size; the Rust agent's carries none.

import {
  examplesBlock,
  expectedParts,
  odinDecl,
  odinSignature,
  parseSignature,
  parseTypes,
  rustDecl,
  rustSignature,
  type Lang,
  type Signature,
} from "./languages.ts";
import { feedbackPrompt, parseCase, systemPrompt, userPrompt, type Case, type CaseResult, type Task } from "./tasks.ts";

export interface Pins {
  rust: string;
  odin: string;
}

/** A document's text without its front matter. */
export function withoutFrontMatter(text: string): string {
  return text.replace(/\r\n/g, "\n").replace(/^---\n[\s\S]*?\n---\n/, "");
}

/** The system prompt of each language: its document, if it has one, is all that changes. */
export function systemPromptFor(lang: Lang, doc: string, pins: Pins): string {
  if (lang === "mente") return systemPrompt(doc);
  if (lang === "rust") {
    return `You write code in Rust, edition 2024, built by rustc ${pins.rust} with overflow checks on, as in a debug build, so an integer overflow panics.`;
  }
  return [
    `You write code in Odin, release ${pins.odin}, a programming language you may have seen little of. Its documentation follows.`,
    "",
    doc.trim(),
  ].join("\n");
}

const NOUN: Record<"rust" | "odin", string> = { rust: "function", odin: "procedure" };

/** The prompt for one task in a language: never its hidden tests, nor an adversarial task's probe. */
export function userPromptFor(task: Task, lang: Lang): string {
  if (lang === "mente") return userPrompt(task);
  const sig = parseSignature(task.signature);
  const decls = parseTypes(task.types).map((d) => (lang === "rust" ? rustDecl(d) : odinDecl(d)));
  const examples = task.examples.map(parseCase).filter((c): c is Case => c !== null);
  const lines = [`# ${task.title}`, "", task.description, ""];
  if (decls.length > 0) {
    lines.push("## Types", "", "The harness declares these types; use them as they are.", "", "```" + lang, decls.join("\n\n"), "```", "");
  }
  lines.push("## Signature", "", "```" + lang, lang === "rust" ? rustSignature(sig) : odinSignature(sig), "```", "");
  if (examples.length > 0) {
    const how =
      lang === "rust"
        ? "Each example calls the function, and every `assert_eq!` in it must hold."
        : "Each example calls the procedure, and every `assert` in it must hold.";
    lines.push("## Examples", "", how, "", "```" + lang, examplesBlock(lang, sig, examples), "```", "");
  }
  lines.push(
    lang === "rust"
      ? "Write the function, with exactly this signature, in one ```rust block. Write only the function: no `main`, no other functions and no explanation; put any `use` lines it needs above it."
      : "Write the procedure, with exactly this signature, in one ```odin block. Write only the procedure: no `package` line, no `main`, no other procedures and no explanation; put any `import` lines it needs above it.",
  );
  return lines.join("\n");
}

/** What went wrong in one Rust or Odin example, from what it returned and left. */
export function failureText(sig: Signature, r: CaseResult): string {
  if (r.fault === "trap" || r.fault === "timeout") return `the call ${r.got}`;
  const want = expectedParts(r);
  const got = expectedParts({ ...r, expected: r.got });
  const parts: string[] = [];
  if (want.value !== null && got.value !== want.value) parts.push(`it returned ${got.value}, expected ${want.value}`);
  const muts = sig.params.filter((p) => p.mut);
  want.leaves.forEach((w, i) => {
    if (got.leaves[i] !== w) parts.push(`it left \`${muts[i]?.name}\` as ${got.leaves[i] ?? "nothing"}, expected ${w}`);
  });
  return parts.length > 0 ? parts.join("; ") : `it gave ${r.got}, expected ${r.expected}`;
}

/** The feedback after a program: what the compiler reported, else each example's result. */
export function feedbackFor(lang: Lang, sig: Signature, diagnostics: string[], results: CaseResult[]): string {
  if (lang === "mente") return feedbackPrompt(diagnostics, results);
  const lines: string[] = [];
  if (diagnostics.length > 0) {
    lines.push(`\`${lang === "rust" ? "rustc" : "odin build"}\` rejects the program:`, "", "```text", ...diagnostics, "```");
  } else {
    lines.push("The program builds. Its examples ran:", "", "```text");
    results.forEach((r, i) => lines.push(r.pass ? `PASS example ${i + 1}` : `FAIL example ${i + 1}: ${failureText(sig, r)}`));
    lines.push("```");
  }
  lines.push("", `Revise the ${NOUN[lang]} and reply with all of it, with exactly the same signature, in one \`\`\`${lang} block.`);
  return lines.join("\n");
}

/** Whether a program holds the language's mark of unfinished code, which scores as unsolved. */
export function hasTodoIn(lang: Lang, program: string): boolean {
  if (lang === "mente") return /\btodo\s*\(/.test(program);
  if (lang === "rust") return /\b(todo|unimplemented)!\s*\(/.test(program);
  return /\bunimplemented\s*\(/.test(program);
}
