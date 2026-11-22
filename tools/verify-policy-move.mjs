// Shows the split of the workflow policy to be a move. Every top-level statement of the old
// .github/scripts/policy.test.mjs at <base> is looked up, by the name it declares, among the
// modules of .github/scripts/policy/ at <head>; each found once with its text unchanged, but for
// `export` before a shared declaration, counts as moved unchanged, and every other one is printed
// with its changed lines, as are statements missing, doubled or added. Run from the repository:
//
//   node tools/verify-policy-move.mjs <base> <head>
//
// PR 200 ran it as `node tools/verify-policy-move.mjs 60f2e52 <the split's commit>`.
import { execFileSync } from "node:child_process";
import { jsTokens } from "../.github/scripts/policy/js.mjs";

const [base, head] = process.argv.slice(2);
if (!base || !head) {
  console.error("usage: node tools/verify-policy-move.mjs <base> <head>");
  process.exit(2);
}
const git = (...args) => execFileSync("git", args, { encoding: "utf8", maxBuffer: 64 << 20 });
const OLD = ".github/scripts/policy.test.mjs";
const DIR = ".github/scripts/policy/";

/// The top-level statements of a module, each with the name it declares and its text, its own
/// lines with nothing before them.
function statements(text) {
  const lines = text.split("\n");
  const out = [];
  let depth = 0;
  let cur = null;
  const isFn = (s) => {
    const v = s.toks.slice(0, 3).map((t) => t.v);
    return v.includes("function") || v[0] === "class";
  };
  for (const t of jsTokens(text)) {
    if (!cur && depth === 0) cur = { toks: [] };
    cur.toks.push(t);
    if (t.t === "punct" && "{([".includes(t.v)) depth++;
    if (t.t === "punct" && "})]".includes(t.v)) depth--;
    if (depth === 0 && t.t === "punct" && (t.v === ";" || (t.v === "}" && isFn(cur)))) {
      out.push(cur);
      cur = null;
    }
  }
  return out.map((s) => {
    const textOf = lines.slice(s.toks[0].line - 1, s.toks[s.toks.length - 1].line).join("\n");
    const v = s.toks.map((t) => t.v);
    let k = v[0] === "export" ? 1 : 0;
    if (v[k] === "async") k++;
    const name =
      v[0] === "check" ? `check: ${s.toks[2].v}` : v[0] === "import" ? `import ${v.slice(1).join(" ")}` : ["const", "let", "function", "class"].includes(v[k]) ? v[k + 1] : `statement at ${s.toks[0].line}`;
    return { name, text: textOf.replace(/^export /, "") };
  });
}

/// Each module's own lines, which every module repeats and the old file held once.
const BOILERPLATE = new Set(["FLOOR", "ran", "check"]);
const counted = (s) => !s.name.startsWith("import ") && !BOILERPLATE.has(s.name) && !s.name.startsWith("statement at");

const oldStmts = statements(git("show", `${base}:${OLD}`));
const newStmts = git("ls-tree", "--name-only", `${head}:${DIR}`)
  .split("\n")
  .filter((f) => f.endsWith(".mjs"))
  .flatMap((f) => statements(git("show", `${head}:${DIR}${f}`)).map((s) => ({ ...s, file: f })));
const byName = new Map();
for (const s of newStmts) byName.set(s.name, [...(byName.get(s.name) ?? []), s]);

const strip = (t) => t.split("\n").filter((l) => !/^\s*$/.test(l)).join("\n");
let same = 0;
const changed = [];
const missing = [];
for (const o of oldStmts.filter(counted)) {
  const found = byName.get(o.name) ?? [];
  if (found.length !== 1) missing.push(`${o.name}: ${found.length} in the modules`);
  else if (strip(found[0].text) === strip(o.text)) same++;
  else changed.push({ name: o.name, file: found[0].file, old: strip(o.text).split("\n"), new: strip(found[0].text).split("\n") });
}
const oldNames = new Set(oldStmts.map((s) => s.name));
const added = newStmts.filter((s) => counted(s) && !oldNames.has(s.name));
const checks = (list) => list.filter((s) => s.name.startsWith("check: ")).length;

console.log(`checks: ${checks(oldStmts)} in the old file, ${checks(newStmts)} in the modules`);
console.log(`statements moved unchanged: ${same}`);
console.log(`statements changed: ${changed.length}`);
for (const c of changed) {
  const diff = [];
  for (let i = 0; i < Math.max(c.old.length, c.new.length); i++) if (c.old[i] !== c.new[i]) diff.push(`  - ${c.old[i] ?? ""}\n  + ${c.new[i] ?? ""}`);
  console.log(`- ${c.name} (${c.file}):\n${diff.join("\n")}`);
}
console.log(`statements missing or doubled: ${missing.length}${missing.length ? `\n  ${missing.join("\n  ")}` : ""}`);
console.log(`statements added: ${added.length}${added.length ? `\n  ${added.map((s) => `${s.name} (${s.file})`).join("\n  ")}` : ""}`);
