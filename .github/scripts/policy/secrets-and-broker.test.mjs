// The workflow policy: secrets and the broker (README.md in this directory). The jobs that hold the
// API key or the held-out token, how they start, and what they run.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { CJK_RANGES, badSubject, baseChangedSincePush, cjkLines, problems, pullRequestOf } from "../lib/commits.mjs";
import { JS_MODULE, holds, jsImports, jsTokens } from "./js.mjs";
import { GATE_FILES, PINNED_FILES, PINNED_JOBS, owns } from "./pins.mjs";
import { digest, read, tracked } from "./repo.mjs";
import { BROKER_STEP, HELD_OUT_STEP, HELD_OUT_TOKEN, SECRET_JOBS, SECRET_SHELL, isStep, secretStepsEnd } from "./secrets.mjs";
import { PERMISSION, beforeToolchainProblems } from "./toolchain.mjs";
import { PAGES_DEPLOY_JOB, allJobs, isPlain, logicalLines, repoPaths, requiredJobs, scriptTexts, stepKey, steps, triggers, workflows } from "./workflows.mjs";

/// Every check of this module counts itself when it runs, and the module fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// each module's FLOOR equal to the checks it declares.
const FLOOR = 6;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks of secrets-and-broker.test.mjs ran: a skip or a filter left the rest out`);
});
/// Whether a value hands a secret over: an expression naming `secrets.` other than a test of
/// whether one is set.
/// Whether a value, a string or anything in a step or a job, holds a secret: an expression that
/// names `secrets` in any form (`secrets.X`, `secrets['X']`, `toJSON(secrets)`), but the test
/// whether one is empty, which yields no secret. The final review of PR 137 found a secret named
/// without a dot read as none.
/// Every string of a value, keys and values, as written: a JSON string of it would turn a tab or a
/// line break inside an expression into `\t` or `\n`, past `\b` (the review of 35bbe0f).
const stringsOf = (v) => (typeof v === "string" ? [v] : Array.isArray(v) ? v.flatMap(stringsOf) : v !== null && typeof v === "object" ? [...Object.keys(v), ...Object.values(v).flatMap(stringsOf)] : []);
/// GitHub reads a context's name in any case: `SECRETS.X` is a secret too.
/// It reads from a string's first `${{` to its end, not to the first `}}`, which GitHub skips
/// inside a quoted string of the expression (the review of b2be87b: `'}}'` ended it early); the
/// one exception is a string that is the emptiness test and nothing more.
const holdsSecret = (value) =>
  stringsOf(value).some((s) => {
    const k = s.indexOf("${{");
    if (k < 0) return false;
    const rest = s.slice(k);
    return /\bsecrets\b/i.test(rest) && !/^\$\{\{\s*secrets\.\w+\s*[!=]=\s*''\s*\}\}\s*$/i.test(rest);
  });

/// The one computed load a module of a secret job may make, by its exact line, once, with the files
/// it loads, which must be owned: the corpus checker's, which loads reference.mjs from the corpus
/// and from the held-out set, whose own the private repository holds at the commit corpus.lock
/// names (the review of 35bbe0f: check.mjs loaded an unowned reference.mjs beside the held-out set
/// by a computed path).
const COMPUTED_LOADS = new Map([
  ["benchmark/check.mjs", { line: '  const reference = (await import(pathToFileURL(path.join(dir, "reference.mjs")).href)).tasks;', loads: ["benchmark/reference.mjs"] }],
]);
/// The modules of node a module of a secret job may import, each by `node:`; child_process only in
/// SPAWNERS. None of them loads or runs code: no module, vm, worker_threads or child_process here.
const NODE_MODULES = new Set(["node:fs", "node:path", "node:url", "node:crypto", "node:net"]);
/// The modules of a secret job that may start a process: each by its exact import of
/// child_process, and every use of what it imports a call on a line named or counted, so that what
/// they start is read where they start it: the harness's runProcess and the preliminary run's
/// mnSync, one call each, and the corpus checker's git, whose two calls are named whole, arguments
/// and all (the review of b2be87b: `git -c core.pager=...` would have run anything). No other module
/// of those jobs imports child_process (the review of 35bbe0f: `execSync as sh` in tasks.ts ran mn
/// beside the key).
const SPAWNERS = new Map([
  ["benchmark/harness/build.ts", { line: 'import { spawn } from "node:child_process";', word: "spawn", calls: 1 }],
  ["benchmark/harness/prelim.ts", { line: 'import { spawnSync } from "node:child_process";', word: "spawnSync", calls: 1 }],
  [
    "benchmark/check.mjs",
    {
      line: 'import { execFileSync } from "node:child_process";',
      word: "execFileSync",
      calls: ['function git(dir, ...args) { return execFileSync(process.platform === "win32" ? "git" : "/usr/bin/git", ["-C", dir, ...args], { encoding: "utf8" }).trim(); }'],
      through: { word: "git", calls: ['    const dirty = git(heldOutDir, "status", "--porcelain");', '    const commit = git(heldOutDir, "rev-parse", "HEAD");'] },
    },
  ],
]);
/// The names a module of a secret job may not use, each a way to run or load code that a reading
/// of its imports does not see, or a way to reach one: eval and Function, the loaders and their
/// hooks, the native loaders, workers, WebAssembly, and the objects that reach any global by a
/// computed name.
const REFUSED_NAMES = new Set([
  "eval",
  "Function",
  "AsyncFunction",
  "require",
  "createRequire",
  "register",
  "_load",
  "getBuiltinModule",
  "binding",
  "_linkedBinding",
  "dlopen",
  "Worker",
  "WebAssembly",
  "globalThis",
  "global",
  "Reflect",
  "Proxy",
  "getPrototypeOf",
  "setPrototypeOf",
  "prototype",
  "defineProperty",
  "defineProperties",
  "__proto__",
  "arguments",
  "module",
  "exports",
]);
/// The modules a secret job may run or import: ES modules alone, which the root package.json's
/// `"type": "module"` makes of `.js` and `.ts` as of `.mjs`.
const SECRET_MODULE = /\.(mjs|js|ts)$/;
/// The members of `process` a module of a secret job may name, each only as `process.<member>`:
/// its arguments, its environment, its end and its platform. The rest of `process` loads or runs
/// code a reading of the module does not see (`getBuiltinModule`, `binding`, `dlopen`, `execve`,
/// the review of 41a54b6), and `process` by any other spelling, an alias or a computed member,
/// would reach them.
const PROCESS_MEMBERS = new Set(["argv", "env", "exit", "platform"]);
/// The one module through which the modules of a secret job read or write a member by a computed
/// key, by its text: it reads only an object's own property and defines only an own property, so
/// that no key, however it is made, reaches what a prototype holds (`constructor`, and through it
/// Function). Every other computed member access in those modules is refused.
const OWN_FILE = "benchmark/own.mjs";
const OWN_TEXT = [
  "// The one way a module of a job that holds a secret reads or writes a member by a computed key",
  "// (docs/design-review.md, untrusted-test-isolation). It reads only an object's own property and",
  "// defines only an own property, so that no key, however it is made, reaches what a prototype",
  "// holds: `constructor`, and through it Function, above all. The key becomes a property key once,",
  "// and that one is both checked and used: a key whose `toString` answers otherwise the second time",
  "// would otherwise pass the check as one name and read another. The workflow policy refuses every",
  "// other computed member access in those modules, and holds this file's text.",
  "",
  "/** A key as the property key it names, made once. */",
  "const keyOf = (key) => (typeof key === \"symbol\" ? key : String(key));",
  "",
  "/** The own property `key` of `object`, or undefined: never one its prototypes hold. */",
  "export function own(object, key) {",
  "  const k = keyOf(key);",
  "  return object !== null && object !== undefined && Object.hasOwn(Object(object), k) ? object[k] : undefined;",
  "}",
  "",
  "/** Defines `key` as an own property of `object`, whatever its prototypes hold, and returns `value`. */",
  "export function setOwn(object, key, value) {",
  "  Object.defineProperty(object, keyOf(key), { value, writable: true, enumerable: true, configurable: true });",
  "  return value;",
  "}",
  "",
].join("\n");
/// The keywords after which `[` opens an array or a pattern, not a computed member.
const KEYWORDS = /^(return|typeof|instanceof|in|of|new|delete|void|throw|case|do|else|yield|await|extends|const|let|var)$/;
/// Whether the `[` at `i` reads a member by a computed key, or names one: after a name, a
/// literal, a call, a parenthesized expression, another member, `?.`, a block's or an object's
/// `}`, or an expression's non-null `!`; after a keyword that is a property's name (`Array.of[k]`,
/// `m.delete[k]`) or a method's (`o.catch(x)[k]`); and as the computed key of an object, a
/// pattern or a class (`{ [k]: v }`, `const { [k]: f } = x`, `[k]() {}`), which a `]` followed by
/// `:` or `(` shows. Not after a keyword, an operator or `(`, where it opens an array or a
/// pattern, nor right after the condition of `if`, `while`, `for`, `switch`, `catch` or
/// `with`, where it starts a statement. A reading of tokens, this narrows what can be spelled and
/// stays a reading aid: no reading decides what arbitrary code does.
export function computedAccessAt(toks, i) {
  const t = toks[i];
  if (!(t?.t === "punct" && t.v === "[") || toks[i + 1]?.v === "]") return false;
  const p = toks[i - 1];
  if (!p) return false;
  const afterDot = (j) => toks[j - 1]?.t === "punct" && toks[j - 1].v === ".";
  const endsExpression = (x, j) => x && ((x.t === "id" && (!KEYWORDS.test(x.v) || afterDot(j))) || x.t === "str" || x.t === "num" || (x.t === "tpl" && !x.open) || (x.t === "punct" && /^[)\]]$/.test(x.v)));
  // A computed key: `[...]` followed by `:` (a property or a pattern's) or `(` (a method's).
  let close = i + 1;
  for (let depth = 0; close < toks.length; close++) {
    if (toks[close].t === "punct" && /^[[({]$/.test(toks[close].v)) depth++;
    if (toks[close].t === "punct" && /^[\])}]$/.test(toks[close].v) && depth-- === 0) break;
  }
  if (toks[close + 1]?.t === "punct" && /^[:(]$/.test(toks[close + 1].v) && p.t === "punct" && /^[{,;}]$/.test(p.v)) return true;
  if (p.t === "id") return !KEYWORDS.test(p.v) || afterDot(i - 1);
  if (p.t === "str" || p.t === "num" || p.t === "re" || (p.t === "tpl" && !p.open)) return true;
  if (p.t !== "punct") return false;
  if (p.v === "]" || p.v === "}") return true;
  if (p.v === "." && toks[i - 2]?.v === "?") return true;
  if (p.v === "!") return endsExpression(toks[i - 2], i - 2);
  if (p.v === ")") {
    let depth = 0;
    for (let j = i - 1; j >= 0; j--) {
      if (toks[j].t === "punct" && toks[j].v === ")") depth++;
      if (toks[j].t === "punct" && toks[j].v === "(" && --depth === 0) {
        const q = toks[j - 1];
        const control = q?.t === "id" && /^(if|while|for|switch|catch|with)$/.test(q.v) && !afterDot(j - 1);
        return !control;
      }
    }
  }
  return false;
}

/// A module of a secret job, read as an allowlist over its tokens. Its imports are static, each of
/// a relative literal that is a file of the repository or of NODE_MODULES (child_process only in
/// SPAWNERS, used as named there); it uses none of REFUSED_NAMES, as a name or a string,
/// `constructor` only to define a class's, `process` only as `process.<member>` of
/// PROCESS_MEMBERS, and no computed member access, which only OWN_FILE makes; every literal and
/// comment closes, and nothing outside printable ASCII, tab and LF, and no backslash outside a
/// literal (an escaped name), appears. The one computed load allowed is COMPUTED_LOADS's line.
/// Returns the problems and the files its relative imports name. These rules narrow what such a
/// module can spell; they cannot decide what arbitrary code does, so the code owner's review holds
/// what they do not list (docs/design-review.md, trust-from-structure), and each node of a secret
/// job runs with code generation from strings disallowed, which no spelling gets past.
export function readSecretModule(file, text) {
  // OWN_FILE is held by its whole text, which no rule below need read.
  if (file === OWN_FILE) return { problems: text === OWN_TEXT ? [] : [`${file}: its text is not the one the policy holds (OWN_TEXT)`], imports: [] };
  const problems = [];
  // An ES module alone: `.cjs` and `.cts` are CommonJS, whose `arguments`, `require` and
  // `module` load any module by a computed name (the re-check of da58409).
  if (!SECRET_MODULE.test(file)) problems.push(`${file}: no .mjs, .js or .ts module, which the package's "type": "module" makes ES modules`);
  const imports = [];
  const lines = text.split("\n");
  const named = COMPUTED_LOADS.get(file);
  if (named && lines.filter((l) => l === named.line).length !== 1) problems.push(`${file}: its named load is not the one line \`${named.line.trim()}\`, once`);
  if (/[^\t\n -~]/.test(text)) problems.push(`${file}: a character outside printable ASCII, tab and LF`);
  // The named load's line is read as blank, keeping every other line's number.
  const toks = jsTokens(named ? lines.map((l) => (l === named.line ? "" : l)).join("\n") : text);
  const where = (t) => `${file}:${t.line}`;
  const prior = (i) => toks[i - 1] ?? { t: "start", v: "" };
  for (let i = 0; i < toks.length; i++) {
    const t = toks[i];
    if (t.bad) problems.push(`${where(t)}: a literal or a comment that does not close`);
    if (t.t === "punct" && t.v === "\\") problems.push(`${where(t)}: a backslash outside a literal, as in an escaped name`);
    if ((t.t === "id" || t.t === "str" || t.t === "tpl") && REFUSED_NAMES.has(t.v)) problems.push(`${where(t)}: ${t.v}, a way to run or load code that a reading of its imports does not see`);
    if ((t.t === "str" || t.t === "tpl") && t.v === "constructor") problems.push(`${where(t)}: "constructor", the way to Function`);
    if (t.t === "id" && t.v === "constructor" && !(toks[i + 1]?.v === "(" && /^[{};]$/.test(prior(i).v) && prior(i).t === "punct")) problems.push(`${where(t)}: constructor, other than defining a class's, the way to Function`);
    if (t.t === "id" && t.v === "process" && prior(i).v !== "." && !(toks[i + 1]?.v === "." && toks[i + 2]?.t === "id" && PROCESS_MEMBERS.has(toks[i + 2].v))) {
      problems.push(`${where(t)}: process other than as process.${[...PROCESS_MEMBERS].join(", process.")}`);
    }
    // Every computed member access, whatever its key: OWN_FILE's `own` and `setOwn` read and
    // write only own properties (the review of 41a54b6: `(() => 0)["constr" + "uctor"]` and
    // `p["getBuilt" + "inModule"]` after `const p = process`).
    if (computedAccessAt(toks, i)) problems.push(`${where(t)}: a member read by a computed key, which only ${OWN_FILE}'s own() and setOwn() do`);
  }
  const read = jsImports(toks);
  for (const line of read.unreadable) problems.push(`${file}:${line}: an import the policy cannot read as a static import of a literal`);
  for (const s of read.specifiers) {
    if (s.dynamic) problems.push(`${file}:${s.line}: import("${s.v}"), a load at run time, not a static import`);
    else if (/^\.{1,2}\//.test(s.v)) {
      const target = path.posix.normalize(path.posix.join(path.posix.dirname(file), s.v));
      imports.push(target);
      if (!tracked().has(target)) problems.push(`${file}:${s.line}: imports ${s.v}, which is no file of the repository`);
      if (!SECRET_MODULE.test(target)) problems.push(`${file}:${s.line}: imports ${s.v}, which is no .mjs, .js or .ts module`);
    } else if (s.v === "node:child_process") {
      if (!SPAWNERS.has(file)) problems.push(`${file}:${s.line}: imports child_process, which only the modules of SPAWNERS may`);
    } else if (!NODE_MODULES.has(s.v)) problems.push(`${file}:${s.line}: imports ${s.v}, which is neither a relative file nor one of NODE_MODULES`);
  }
  const spawner = SPAWNERS.get(file);
  // No module but SPAWNERS names child_process at all, in any form the reading does not take for
  // an import of it.
  // Read as the rest is read, decoded: the text, and every literal's value, an escape in it spelled
  // out (the review of 75e5eeb: a second import of `"node:child_process"` counted as none in
  // the text).
  const naming = toks.filter((x) => (x.t === "str" || x.t === "tpl") && x.v.includes("child_process"));
  if (!spawner && (text.includes("child_process") || naming.length > 0)) problems.push(`${file}: names child_process, which only the modules of SPAWNERS may`);
  if (spawner) {
    const lineNo = (line) => lines.flatMap((l, k) => (l === line ? [k + 1] : []));
    const importAt = lineNo(spawner.line);
    const imported = read.specifiers.filter((s) => s.v === "node:child_process");
    const alone = importAt.length === 1 && imported.length === 1 && imported[0].line === importAt[0] && naming.length === 1 && (text.match(/child_process/g) ?? []).length === 1;
    if (!alone) problems.push(`${file}: its import of child_process is not \`${spawner.line}\` alone, once, decoded`);
    // Every use of a name is the import (or the function's definition), and each call where it is
    // named or as many as counted, each a call.
    const uses = (word, definedAt, calls) => {
      const found = toks.filter((t) => t.t === "id" && t.v === word && prior(toks.indexOf(t)).v !== ".");
      const rest = found.filter((t) => !definedAt.includes(t.line));
      const callAt = typeof calls === "number" ? null : calls.map(lineNo);
      const isCall = (t) => toks[toks.indexOf(t) + 1]?.v === "(";
      const ok =
        found.length - rest.length === definedAt.length &&
        rest.every(isCall) &&
        (callAt === null ? rest.length === calls : callAt.every((at) => at.length === 1) && rest.length === callAt.length && rest.every((t) => callAt.some((at) => at[0] === t.line)));
      if (!ok) problems.push(`${file}: its uses of ${word} are not ${typeof calls === "number" ? `its import and ${calls} call` : `its import and the named calls: ${calls.map((l) => l.trim()).join(" ")}`}`);
    };
    uses(spawner.word, importAt, spawner.calls);
    if (spawner.through) uses(spawner.through.word, lineNo(spawner.calls[0]), spawner.through.calls);
  }
  return { problems, imports };
}
export const secretModuleProblems = (file, text) => readSecretModule(file, text).problems;
/// The environment that holds the secrets.
const SECRET_ENVIRONMENT = "benchmark";
/// A flag as start-broker.sh writes it: the value of a flag that names the workspace, quoted.
const quoteFlag = (f) => (f.includes("=$") ? f.replace(/=(.*)$/, '="$1"') : f);
/// The step that takes root from the runner's user for the rest of a job, but for the sandbox's
/// root-owned half, before the sandbox runs anything and before the key is in reach: so that no
/// process of that user, outside the sandbox, reads the key out of the broker's memory or the
/// runner's (benchmark/harness/lock-sudo.sh; docs/design-review.md, untrusted-test-isolation).
export const LOCK_STEP = { name: "Take root from the runner's user, but for the sandbox", run: "bash benchmark/harness/lock-sudo.sh" };
/// The last step of a secret job: it stops the broker, whatever came before, and checks that root
/// is still out of the runner's user's reach.
export const STOP_STEP = { name: "Stop the broker", if: "always()", run: "bash benchmark/harness/stop-broker.sh\nbash benchmark/harness/lock-sudo.sh --check\n" };
/// The last step of a job that runs the sandbox and holds no secret: the same check.
export const LOCKED_STEP = { name: "Root is still out of the runner's user's reach", if: "!cancelled() && steps.watch.outputs.run != 'false'", run: "bash benchmark/harness/lock-sudo.sh --check" };
/// The line of start-broker.sh that starts the broker: the key on its standard input and in no
/// environment (`env -i`, which also drops NODE_OPTIONS, whose --allow-child-process would undo
/// the permission model), under permission.json's flags, each command by its path: env, and the
/// Node of permission.json's version that setup-node put in the runner's tool cache.
export const BROKER_LINE = `printf '%s\\n' "$ANTHROPIC_API_KEY" | /usr/bin/env -i PATH="$PATH" "$RUNNER_TOOL_CACHE/node/${PERMISSION.node}/x64/bin/node" ${PERMISSION.broker.flags.map(quoteFlag).join(" ")} "$GITHUB_WORKSPACE/${PERMISSION.broker.script}" --socket "$dir/socket" --budget "$BUDGET" > "$dir/log" 2>&1 &`;
/// The lines start-broker.sh holds before BROKER_LINE: no core file at any size, a limit the
/// runner's user cannot raise again, and a mask and a directory that keep the socket, the log and
/// the pid file the runner's user's alone.
const BROKER_SETUP = ['dir="$RUNNER_TEMP/broker"', "ulimit -c 0", "umask 0177", '/usr/bin/mkdir -m 0700 "$dir"'];
/// The one write to the environment of later steps a secret job may make before its held-out step
/// and its broker's: a path of its own, which holds no secret and turns nothing on. Any other could
/// set what the shells of those steps run with (SHELLOPTS, BASH_ENV) or put a program first on PATH.
const ENV_WRITES = new Set(['echo "RUSTUP_HOME=$RUNNER_TEMP/rustup" >> "$GITHUB_ENV"']);
/// Whether a step writes the environment or the PATH of later steps other than by ENV_WRITES.
const writesEnv = (s) => /GITHUB_ENV|GITHUB_PATH/.test(String(s.run ?? "")) && !ENV_WRITES.has(String(s.run ?? "").trim());
/// What is wrong with the repository's paths for the held-out step: held-out-set, where it checks the
/// set out, tracked as a file, a link or a directory.
export const heldOutPathProblems = (paths) => [...paths].filter((f) => f === "held-out-set" || f.startsWith("held-out-set/")).map((f) => `${f}: a path of the repository where the held-out step checks the set out`);
/// The held-out repository, which no step checks out but HELD_OUT_STEP.
const HELD_OUT_REPOSITORY = "Menteion/mente-bench-heldout";

/// What is wrong with start-broker.sh: BROKER_SETUP, then BROKER_LINE, each once, and no other node.
export function startBrokerProblems(text) {
  const found = [];
  const lines = text.split("\n");
  const at = (line) => lines.indexOf(line);
  for (const line of [...BROKER_SETUP, BROKER_LINE]) {
    if (lines.filter((l) => l === line).length !== 1) found.push(`start-broker.sh does not hold, once: ${line}`);
  }
  if (!BROKER_SETUP.every((line, k) => at(line) >= 0 && at(line) < at(BROKER_LINE) && (k === 0 || at(line) > at(BROKER_SETUP[k - 1])))) found.push("start-broker.sh does not set the core limit, the mask and the directory up, in that order, before it starts the broker");
  const nodes = lines.filter((l) => /(^|[\s;&|(\/])node"?(\s|$)/.test(l.replace(/#.*$/, "")));
  if (nodes.length !== 1) found.push(`start-broker.sh starts node ${nodes.length} times, not once`);
  // The key is named twice, in the check that it is set and in the line that hands it to the
  // broker, and the shell never traces what it runs.
  const named = lines.filter((l) => l.includes("ANTHROPIC_API_KEY") && !l.trimStart().startsWith("#"));
  if (named.length !== 2 || !named.includes(BROKER_LINE) || !named.some((l) => l.startsWith(': "${ANTHROPIC_API_KEY:?'))) found.push("start-broker.sh names the key beyond its check and the line that hands it to the broker");
  if (lines.some((l) => /^\s*set\s+(-\w*x|-o\s+xtrace)|BASH_XTRACEFD|\bxtrace\b/.test(l.replace(/#.*$/, "")))) found.push("start-broker.sh traces what it runs, the key among it");
  return found;
}

/// What is wrong with how a secret job holds its key: root taken from the runner's user before the
/// key is in reach (LOCK_STEP); one step holding the key, BROKER_STEP exactly; the one other step
/// that may hold a secret, the held-out checkout, holding its token as its one input secret and
/// keeping it out of .git/config; no secret wider than a step; the harness talking to the broker
/// after it starts, holding none; STOP_STEP last; and permission.json's Node, set up once.
export function brokerProblems(key, job, doc = {}) {
  const found = [];
  const all = steps(job);
  if (holdsSecret(doc.env) || holdsSecret({ ...job, steps: null }) || "secrets" in job) found.push(`${key} holds a secret wider than a step, which would reach every step beside the broker`);
  const locks = all.filter((s) => isStep(s, LOCK_STEP) && s.if === undefined);
  const brokers = all.filter((s) => isStep(s, BROKER_STEP) && s.if === undefined);
  if (brokers.length !== 1) found.push(`${key} does not start the broker once, as BROKER_STEP`);
  if (locks.length !== 1 || all.indexOf(locks[0]) > all.findIndex((s) => holdsSecret(s))) found.push(`${key} does not take root from the runner's user once, as LOCK_STEP, before any secret is in reach`);
  const heldOut = all.filter((s) => isStep(s, HELD_OUT_STEP));
  for (const s of all.filter((x) => holdsSecret(x) && !brokers.includes(x))) {
    if (!heldOut.includes(s)) found.push(`${key}, step "${s.name ?? s.uses}": holds a secret where none belongs (the key belongs in BROKER_STEP alone, ${HELD_OUT_TOKEN} in HELD_OUT_STEP alone)`);
  }
  if (heldOut.length > 1) found.push(`${key} checks the held-out set out more than once`);
  // Nothing before the held-out step or the broker's writes the environment of later steps but
  // ENV_WRITES (the review of aafe59b: SHELLOPTS=xtrace through $GITHUB_ENV traced the token); the
  // scripts those steps run are pinned (secretPrefixFiles).
  const lastHeld = secretStepsEnd(all);
  for (const s of all.slice(0, lastHeld)) if (writesEnv(s)) found.push(`${key}, step "${s.name ?? s.uses}": writes the environment or the PATH of later steps before the secrets are in reach`);
  // No step checks the held-out repository out but HELD_OUT_STEP: actions/checkout prints the
  // commit's subject into the log, and a token as its input reaches .git/config unless told not to.
  for (const s of all) if (/^actions\/checkout@/.test(String(s.uses ?? "")) && String(s.with?.repository ?? "").toLowerCase() === HELD_OUT_REPOSITORY.toLowerCase()) found.push(`${key}, step "${s.name ?? s.uses}": checks the held-out repository out through actions/checkout, which prints its commit's subject into the log`);
  const stops = all.filter((s) => isStep(s, STOP_STEP) && s.if === "always()");
  if (stops.length !== 1 || all.at(-1) !== stops[0]) found.push(`${key} does not stop the broker in its last step, as STOP_STEP`);
  const harness = all.filter((s) => /benchmark\/harness\/(bench|prelim|probe)\.ts/.test(String(s.run ?? "")));
  const talks = harness.length === 1 && String(harness[0].run).includes('--broker "$RUNNER_TEMP/broker/socket"') && all.indexOf(harness[0]) > all.indexOf(brokers[0]);
  if (!talks) found.push(`${key} does not run the harness once, after the broker starts, talking to it`);
  const node = all.filter((s) => /^actions\/setup-node@/.test(String(s.uses ?? "")));
  if (node.length !== 1 || String(node[0].with?.["node-version"]) !== PERMISSION.node) found.push(`${key} does not set up Node ${PERMISSION.node} once, whose permission model the broker's flags rely on`);
  return found;
}

/// What is wrong with a job that runs the benchmark's sandbox: root taken from the runner's user
/// (LOCK_STEP) before the first step that runs it, and checked out of reach in the job's last step
/// (STOP_STEP in a secret job, LOCKED_STEP in another).
export function sudoLockProblems(key, job) {
  const all = steps(job);
  const first = all.findIndex((s) => /"\$SANDBOX"|benchmark\/harness\/sandbox(-check)?\.(sh|ts)/.test(String(s.run ?? "")));
  if (first < 0) return [];
  const found = [];
  const locks = all.filter((s) => isStep(s, LOCK_STEP));
  if (locks.length !== 1 || all.indexOf(locks[0]) > first) found.push(`${key} runs the sandbox, but does not take root from the runner's user once, as LOCK_STEP, before it`);
  const last = all.at(-1);
  if (!last || !(isStep(last, STOP_STEP) || isStep(last, LOCKED_STEP))) found.push(`${key} runs the sandbox, but does not check in its last step that root is still out of reach`);
  return found;
}

/// What is wrong with the Node the jobs set up: each sets up permission.json's, one Node for the
/// broker, for the tests of its permission model and for every other job, read in one place.
export function nodeProblems(jobs) {
  const found = [];
  for (const { file, id, job } of jobs) {
    for (const s of steps(job).filter((x) => /^actions\/setup-node@/.test(String(x.uses ?? "")))) {
      if (String(s.with?.["node-version"]) !== PERMISSION.node) found.push(`${file}#${id} sets up Node ${s.with?.["node-version"]}, not ${PERMISSION.node}, whose permission model the broker's flags rely on`);
    }
  }
  return found;
}

check("the broker, the one process that holds the key, starts under permission.json's flags, root out of every reach, and every job runs its Node", () => {
  // The flags: Node's permission model, no code from strings and no node:sqlite, reads of the
  // broker and its limits alone, and none of the flags that would let it start, load or write
  // anything. What the broker's own code imports or calls guarantees nothing, since
  // process.getBuiltinModule reaches any built-in; the flags do, and the shape of what it sends.
  const flags = PERMISSION.broker.flags;
  for (const f of ["--permission", "--disallow-code-generation-from-strings", "--no-experimental-sqlite"]) assert.ok(flags.includes(f), `the broker's flags lose ${f}`);
  assert.deepEqual(flags.filter((f) => f.startsWith("--allow-")).sort(), ["--allow-fs-read=$GITHUB_WORKSPACE/benchmark/harness/broker-limits.json", "--allow-fs-read=$GITHUB_WORKSPACE/benchmark/harness/broker.ts"]);
  for (const r of PERMISSION.refused) assert.ok(!flags.some((f) => f.startsWith(r)), `the broker's flags hold ${r}`);
  assert.match(PERMISSION.node, /^\d+\.\d+\.\d+$/);
  // A relative import would fail as the broker ran, since its flags let it read no other file.
  assert.deepEqual(readSecretModule(PERMISSION.broker.script, read(PERMISSION.broker.script)).imports, [], "the broker imports a file, which its flags do not let it read");
  // start-broker.sh starts it so, and nothing else.
  const script = read("benchmark/harness/start-broker.sh");
  assert.deepEqual(startBrokerProblems(script), []);
  for (const [what, text] of [
    ["--allow-child-process among the flags", script.replace("--permission", "--permission --allow-child-process")],
    ["no env -i", script.replace('/usr/bin/env -i PATH="$PATH" ', "")],
    ["Node found through PATH", script.replace('"$RUNNER_TOOL_CACHE/node/' + "24.22.1" + '/x64/bin/node"', "node")],
    ["env found through PATH", script.replace('/usr/bin/env -i PATH="$PATH" ', 'env -i PATH="$PATH" ')],
    ["no core limit", script.replace("ulimit -c 0\n", "")],
    ["no mask", script.replace("umask 0177\n", "")],
    ["a mask after the broker starts", script.replace("umask 0177\n", "").replace(/(> "\$dir\/log" 2>&1 &\n)/, "$1umask 0177\n")],
    ["a second node", `${script}node -e 1\n`],
    // The re-check of 4ab15da: lines added before the key's line, each a way to leak the key.
    ["a trace", script.replace("ulimit -c 0\n", "set -x\nulimit -c 0\n")],
    ["the key written to a file", script.replace("ulimit -c 0\n", 'printf %s "$ANTHROPIC_API_KEY" > "$RUNNER_TEMP/k"\nulimit -c 0\n')],
    ["the key handed to another program", script.replace("ulimit -c 0\n", '/usr/bin/printf %s "$ANTHROPIC_API_KEY" | sha256sum\nulimit -c 0\n')],
  ]) {
    assert.ok(startBrokerProblems(text).length > 0, `a start-broker.sh with ${what} passes`);
    // And the pin, which holds the file whole, fails on it whatever the rules above read.
    assert.notEqual(digest(text), PINNED_FILES.get("benchmark/harness/start-broker.sh"), `a start-broker.sh with ${what} keeps its pin`);
  }
  for (const file of ["start-broker.sh", "stop-broker.sh", "lock-sudo.sh", "sandbox-root.sh", "fetch-held-out.sh"]) assert.ok(PINNED_FILES.has(`benchmark/harness/${file}`), `${file}, on the key's path, the held-out token's or the lock's, is not pinned whole`);
  // The held-out step's script is held by its pin, whose digest is its exact text: no rule over its
  // lines stands for that (the review of aafe59b found a denylist of them easy to pass).
  // held-out-set, where it checks the set out, is no path of the repository in any form: a link
  // tracked there would take the checkout wherever it pointed (the review of aafe59b).
  assert.deepEqual(heldOutPathProblems(tracked()), []);
  assert.ok(heldOutPathProblems(new Set(["held-out-set"])).length > 0 && heldOutPathProblems(new Set(["held-out-set/x.md"])).length > 0, "a tracked held-out-set passed");
  // Every job sets up that Node: the secret jobs, the jobs that run the harness's tests and the
  // planted escapes among them, and the rest.
  assert.deepEqual(nodeProblems(allJobs()), []);
  for (const v of [24, "24", "24.x", "lts/*", undefined]) {
    const planted = [{ file: "x.yml", id: "y", job: { steps: [{ uses: "actions/setup-node@0", with: v === undefined ? {} : { "node-version": v } }] } }];
    assert.ok(nodeProblems(planted).length > 0, `a job that sets up Node ${v} passes`);
  }
  // Every job that runs the sandbox takes root from the runner's user before it, and checks at its
  // end that it is still out of reach.
  for (const { file, id, job } of allJobs()) assert.deepEqual(sudoLockProblems(`${file}#${id}`, job), []);
  assert.ok(allJobs().some(({ file, id, job }) => !SECRET_JOBS.has(`${file}#${id}`) && sudoLockProblems(`${file}#${id}`, { steps: steps(job).filter((s) => !isStep(s, LOCK_STEP)) }).length > 0), "no job that runs the sandbox outside the secret jobs proves the lock on every change");
  // Planted: each way a secret job's broker could go wrong.
  const job = {
    steps: [
      { uses: "actions/checkout@0", with: { "persist-credentials": false } },
      { uses: "actions/setup-node@0", with: { "node-version": PERMISSION.node, "package-manager-cache": false } },
      LOCK_STEP,
      HELD_OUT_STEP,
      BROKER_STEP,
      { name: "Run", run: 'node --disallow-code-generation-from-strings benchmark/harness/bench.ts --sandbox "$SANDBOX" --broker "$RUNNER_TEMP/broker/socket"' },
      STOP_STEP,
    ],
  };
  assert.deepEqual(brokerProblems("x.yml#y", job), []);
  assert.deepEqual(sudoLockProblems("x.yml#y", job), []);
  const at = (k, step) => ({ steps: job.steps.map((s, i) => (i === k ? step : s)) });
  const without = (k) => ({ steps: job.steps.filter((_, i) => i !== k) });
  for (const [what, planted] of [
    ["the broker's step changed", at(4, { ...BROKER_STEP, run: "bash benchmark/harness/start-broker.sh --allow-child-process" })],
    ["the key in the harness's step", at(5, { ...job.steps[5], env: { ANTHROPIC_API_KEY: "${{ secrets.ANTHROPIC_API_KEY }}" } })],
    ["a harness that does not talk to the broker", at(5, { name: "Run", run: 'node benchmark/harness/bench.ts --sandbox "$SANDBOX"' })],
    ["the held-out checkout keeping its token", at(3, { uses: "actions/checkout@0", with: { token: HELD_OUT_TOKEN } })],
    // Run 3 of the plan: actions/checkout of the held-out repository printed its commit's subject.
    ["the held-out set checked out by actions/checkout", at(3, { uses: "actions/checkout@0", with: { repository: "Menteion/mente-bench-heldout", token: HELD_OUT_TOKEN, "persist-credentials": false } })],
    ["the held-out repository checked out by actions/checkout with no secret", { steps: [...job.steps.slice(0, 3), { uses: "actions/checkout@0", with: { repository: "menteion/MENTE-BENCH-HELDOUT" } }, ...job.steps.slice(3)] }],
    ["the held-out step changed", at(3, { ...HELD_OUT_STEP, run: "bash benchmark/harness/fetch-held-out.sh; git -C held-out-set log -1" })],
    ["the key in the held-out step", at(3, { ...HELD_OUT_STEP, env: { ...HELD_OUT_STEP.env, ANTHROPIC_API_KEY: "${{ secrets.ANTHROPIC_API_KEY }}" } })],
    ["the held-out set checked out twice", { steps: [...job.steps.slice(0, 4), HELD_OUT_STEP, ...job.steps.slice(4)] }],
    ["the held-out step run in the job's environment", at(3, { ...HELD_OUT_STEP, run: "bash benchmark/harness/fetch-held-out.sh" })],
    ["a trace set for later steps before the secrets", { steps: [...job.steps.slice(0, 2), { run: 'echo "SHELLOPTS=xtrace" >> "$GITHUB_ENV"' }, ...job.steps.slice(2)] }],
    ["a program put first on PATH before the secrets", { steps: [...job.steps.slice(0, 2), { run: 'echo "$RUNNER_TEMP/bin" >> "$GITHUB_PATH"' }, ...job.steps.slice(2)] }],
    // The re-check of c55cf8f: the runner's bash read BASH_ENV and SHELLOPTS before the step's
    // first line, the token already in its environment; env, mktemp and bash came from PATH; and
    // the token stood on env's command line.
    ["BASH_ENV set for later steps before the held-out step", { steps: [...job.steps.slice(0, 3), { run: 'echo "BASH_ENV=$RUNNER_TEMP/x.sh" >> "$GITHUB_ENV"' }, ...job.steps.slice(3)] }],
    ["SHELLOPTS=xtrace set for later steps before the broker's step", { steps: [...job.steps.slice(0, 4), { run: 'echo "SHELLOPTS=xtrace" >> "$GITHUB_ENV"' }, ...job.steps.slice(4)] }],
    ["the held-out step in the runner's shell", at(3, { name: HELD_OUT_STEP.name, if: HELD_OUT_STEP.if, env: HELD_OUT_STEP.env, run: HELD_OUT_STEP.run })],
    ["the broker's step in the runner's shell", at(4, { name: BROKER_STEP.name, env: BROKER_STEP.env, run: BROKER_STEP.run })],
    ["a shell that keeps BASH_ENV", at(3, { ...HELD_OUT_STEP, shell: SECRET_SHELL.replace("-u BASH_ENV ", "") })],
    // The final pass on 89c7691: an exported function in place of printf printed the secret.
    ["a shell whose bash is not privileged", at(3, { ...HELD_OUT_STEP, shell: SECRET_SHELL.replace(" -p ", " ") })],
    ["fetch-held-out.sh in a bash that is not privileged", at(3, { ...HELD_OUT_STEP, run: HELD_OUT_STEP.run.replace(" -p ", " ") })],
    ["start-broker.sh in a bash that is not privileged", at(4, { ...BROKER_STEP, run: BROKER_STEP.run.replace(" -p ", " ") })],
    ["env found through PATH in the held-out step", at(3, { ...HELD_OUT_STEP, run: HELD_OUT_STEP.run.replace("/usr/bin/env ", "env ") })],
    ["mktemp found through PATH", at(3, { ...HELD_OUT_STEP, run: HELD_OUT_STEP.run.replace("/usr/bin/mktemp", "mktemp") })],
    ["the held-out token on env's command line", at(3, { ...HELD_OUT_STEP, run: 'home="$(/usr/bin/mktemp -d)"\n/usr/bin/env -i PATH=/usr/bin:/bin HOME="$home" HELD_OUT_TOKEN="$HELD_OUT_TOKEN" HELD_OUT_COMMIT="$HELD_OUT_COMMIT" /usr/bin/bash --noprofile --norc -p benchmark/harness/fetch-held-out.sh\n' })],
    ["bash found through PATH in the broker's step", at(4, { ...BROKER_STEP, run: "bash benchmark/harness/start-broker.sh" })],
    ["the key as the checkout's token", at(3, { uses: "actions/checkout@0", with: { token: "${{ secrets.ANTHROPIC_API_KEY }}", "persist-credentials": false } })],
    ["another secret as the checkout's token", at(3, { uses: "actions/checkout@0", with: { token: "${{ secrets.OTHER }}", "persist-credentials": false } })],
    ["the held-out token in a step's env", at(5, { ...job.steps[5], env: { HELD_OUT: HELD_OUT_TOKEN } })],
    ["the held-out token in the checkout's env", at(3, { ...job.steps[3], env: { TOKEN: HELD_OUT_TOKEN } })],
    ["the key in the job's env", { ...job, env: { ANTHROPIC_API_KEY: "${{ secrets.ANTHROPIC_API_KEY }}" } }],
    ["no stop", { steps: job.steps.slice(0, 6) }],
    ["a stop that is not last", { steps: [...job.steps.slice(0, 5), STOP_STEP, job.steps[5]] }],
    ["a stop without the lock's check", at(6, { ...STOP_STEP, run: "bash benchmark/harness/stop-broker.sh\n" })],
    ["the broker started twice", { steps: [...job.steps.slice(0, 5), BROKER_STEP, ...job.steps.slice(5)] }],
    ["no lock", without(2)],
    ["the lock after the key is in reach", { steps: [job.steps[0], job.steps[1], job.steps[3], LOCK_STEP, ...job.steps.slice(4)] }],
    ["the lock under a condition", at(2, { ...LOCK_STEP, if: "false" })],
    ["no setup-node", without(1)],
    ["another Node", at(1, { uses: "actions/setup-node@0", with: { "node-version": "24" } })],
  ]) {
    assert.ok(brokerProblems("x.yml#y", planted).length > 0, `a secret job with ${what} passes`);
  }
  assert.ok(brokerProblems("x.yml#y", job, { env: { K: "${{ secrets.ANTHROPIC_API_KEY }}" } }).length > 0, "a secret job whose workflow's env holds the key passes");
  // Planted: a job that runs the sandbox and holds no secret.
  const watched = (s) => ({ ...s, if: "steps.watch.outputs.run != 'false'" });
  const fixtures = { steps: [watched(LOCK_STEP), watched({ name: "Prove", run: 'node benchmark/harness/sandbox-check.ts --sandbox "$SANDBOX"' }), LOCKED_STEP] };
  assert.deepEqual(sudoLockProblems("x.yml#f", fixtures), []);
  for (const [what, planted] of [
    ["no lock", { steps: fixtures.steps.slice(1) }],
    ["the lock after the sandbox", { steps: [fixtures.steps[1], fixtures.steps[0], fixtures.steps[2]] }],
    ["no check at the end", { steps: fixtures.steps.slice(0, 2) }],
    ["a check that is not last", { steps: [fixtures.steps[0], fixtures.steps[2], fixtures.steps[1]] }],
  ]) {
    assert.ok(sudoLockProblems("x.yml#f", planted).length > 0, `a job that runs the sandbox with ${what} passes`);
  }
});
/// A node a job of SECRET_JOBS starts without disallowing code generation from strings.
const NODE_WITHOUT_GUARD = /(^|[\s;&|(])node\s+(?!--disallow-code-generation-from-strings\s)/;

check("a node line without the flag that refuses code from strings is read as one", () => {
  for (const line of ["node benchmark/harness/bench.ts", "x && node -e 1", "(node x.mjs)"]) assert.match(line, NODE_WITHOUT_GUARD, line);
  assert.doesNotMatch("node --disallow-code-generation-from-strings benchmark/harness/bench.ts --budget 1", NODE_WITHOUT_GUARD);
});

/// The other jobs that name an environment, each with its name: the Pages deployment, whose
/// environment holds no secret.
const OTHER_ENVIRONMENTS = new Map([[PAGES_DEPLOY_JOB, "github-pages"]]);

/// What is wrong with a job's environment: a job of SECRET_JOBS names SECRET_ENVIRONMENT, as a
/// literal string, a job of OTHER_ENVIRONMENTS names its own, and no other job names any, so that
/// the secrets reach no job the policy does not hold as one of SECRET_JOBS.
export function environmentProblems(key, job) {
  if (SECRET_JOBS.has(key)) return job.environment === SECRET_ENVIRONMENT ? [] : [`${key} holds a secret and does not name the environment ${SECRET_ENVIRONMENT} as a literal string`];
  const other = OTHER_ENVIRONMENTS.get(key);
  const name = typeof job.environment === "object" && job.environment !== null ? job.environment.name : job.environment;
  if (other !== undefined) return name === other ? [] : [`${key} does not name the environment ${other}, the one OTHER_ENVIRONMENTS gives it`];
  return "environment" in job ? [`${key} names an environment, which only the jobs of SECRET_JOBS and OTHER_ENVIRONMENTS do`] : [];
}

/// What is wrong with how a workflow that holds a job of SECRET_JOBS starts and what its token may
/// do: it starts by a dispatch alone, which only someone with write access can make, and its token
/// reads the repository's contents and nothing more, at the workflow's level and in no job's
/// override. Any other trigger would start the job with ref main, which the environment admits:
/// `issue_comment` on any user's comment, `push` on every merge, `schedule` unasked (the review of
/// 41a54b6 found `issue_comment:` and `contents: write` in probe.yml passing every check).
export function secretWorkflowProblems(file, doc) {
  const found = [];
  const on = Object.keys(triggers(doc));
  if (on.length !== 1 || on[0] !== "workflow_dispatch") found.push(`${file} holds a secret job and starts on ${on.join(", ") || "nothing"}, not on workflow_dispatch alone`);
  if (!isPlain(doc.permissions) || Object.keys(doc.permissions).length !== 1 || doc.permissions.contents !== "read") found.push(`${file} holds a secret job and its token's permissions are not exactly contents: read`);
  for (const [id, job] of Object.entries(doc.jobs ?? {})) if (isPlain(job) && "permissions" in job) found.push(`${file}#${id} sets permissions of its own in a workflow that holds a secret job`);
  // Every other job of it names no environment, so a secret it could see is a repository's or an
  // organization's, which must not exist: each carries LEAK_STEP, which fails then.
  for (const [id, job] of Object.entries(doc.jobs ?? {})) {
    if (SECRET_JOBS.has(`${file}#${id}`)) continue;
    const leak = steps(job).filter((s) => JSON.stringify(s) === JSON.stringify(LEAK_STEP)).length;
    if (leak !== 1) found.push(`${file}#${id} names no environment and does not check, once, as LEAK_STEP, that no secret of the benchmark is set outside it`);
  }
  return found;
}
/// The step that fails when a secret of the benchmark is set outside its environment, as a
/// repository's or an organization's secret of the same name (the review of 41a54b6).
const LEAK_STEP = {
  name: "No secret of the benchmark outside its environment",
  env: { KEY_LEAKED: "${{ secrets.ANTHROPIC_API_KEY != '' }}", HELD_OUT_LEAKED: "${{ secrets.MENTE_BENCH_HELDOUT_TOKEN != '' }}" },
  run: 'if [ "$KEY_LEAKED" != false ] || [ "$HELD_OUT_LEAKED" != false ]; then echo "::error::a secret of the benchmark is set outside the environment benchmark"; exit 1; fi',
};

/// The required job that runs LEAK_STEP on every pull request, so that a secret of the benchmark
/// set again at the repository's or the organization's level is seen at once, not at the next
/// dispatch of a secret workflow (the final pass of ad38011).
const LEAK_CHECKED = "ci.yml#docs";

check("every workflow that holds a secret job starts by a dispatch alone, with a token that only reads", () => {
  const holding = new Set([...SECRET_JOBS.keys()].map((k) => k.split("#")[0]));
  for (const { file, doc } of workflows()) if (holding.has(file)) assert.deepEqual(secretWorkflowProblems(file, doc), []);
  const [leakFile, leakJob] = LEAK_CHECKED.split("#");
  const checked = workflows().find((x) => x.file === leakFile)?.doc.jobs?.[leakJob];
  assert.equal(steps(checked).filter((s) => JSON.stringify(s) === JSON.stringify(LEAK_STEP)).length, 1, `${LEAK_CHECKED} does not run LEAK_STEP once, on every pull request`);
  assert.ok(requiredJobs().some(({ file, id }) => `${file}#${id}` === LEAK_CHECKED), `${LEAK_CHECKED}, which checks on every pull request that no secret leaked, is no required job`);
  const base = { on: { workflow_dispatch: { inputs: { runs: { default: "1" } } } }, permissions: { contents: "read" }, jobs: { write: { "runs-on": "ubuntu-24.04" }, check: { steps: [{ uses: "actions/checkout@0" }, LEAK_STEP] } } };
  assert.deepEqual(secretWorkflowProblems("probe.yml", base), []);
  for (const check of [{ steps: [] }, { steps: [{ ...LEAK_STEP, run: "true" }] }, { steps: [{ ...LEAK_STEP, env: { KEY_LEAKED: LEAK_STEP.env.KEY_LEAKED } }] }, { steps: [LEAK_STEP, LEAK_STEP] }]) {
    assert.ok(secretWorkflowProblems("probe.yml", { ...base, jobs: { ...base.jobs, check } }).length > 0, `a job without the leak check passes: ${JSON.stringify(check)}`);
  }
  // The leak check reads as no secret held: only the test whether one is set.
  assert.ok(!holdsSecret(LEAK_STEP));
  for (const [what, doc] of [
    ["issue_comment", { ...base, on: { ...base.on, issue_comment: { types: ["created"] } } }],
    ["push to main", { ...base, on: { ...base.on, push: { branches: ["main"] } } }],
    ["schedule", { ...base, on: { ...base.on, schedule: [{ cron: "0 0 * * *" }] } }],
    ["workflow_call", { ...base, on: { ...base.on, workflow_call: null } }],
    ["repository_dispatch", { ...base, on: { ...base.on, repository_dispatch: null } }],
    ["pull_request", { ...base, on: "pull_request" }],
    ["a list of triggers", { ...base, on: ["workflow_dispatch", "push"] }],
    ["contents: write", { ...base, permissions: { contents: "write" } }],
    ["write-all", { ...base, permissions: "write-all" }],
    ["id-token beside contents", { ...base, permissions: { contents: "read", "id-token": "write" } }],
    ["no permissions", { ...base, permissions: undefined }],
    ["a job's own permissions", { ...base, jobs: { write: { "runs-on": "ubuntu-24.04", permissions: { contents: "write" } } } }],
  ]) {
    assert.ok(secretWorkflowProblems("probe.yml", doc).length > 0, `a secret workflow with ${what} passes`);
  }
});

check("every job that holds a secret names the environment benchmark, for main alone, and no other job names it", () => {
  for (const { file, id, job } of allJobs()) assert.deepEqual(environmentProblems(`${file}#${id}`, job), []);
  for (const [key, job] of [
    ["benchmark.yml#bench", {}],
    ["benchmark.yml#bench", { environment: { name: SECRET_ENVIRONMENT } }],
    ["prelim.yml#prelim", { environment: "${{ inputs.environment }}" }],
    ["benchmark.yml#build", { environment: SECRET_ENVIRONMENT }],
    ["ci.yml#test", { environment: "staging" }],
    ["pages-publish.yml#deploy", { environment: { name: SECRET_ENVIRONMENT } }],
  ]) {
    assert.ok(environmentProblems(key, job).length > 0, `the environment rule lets through ${key}: ${JSON.stringify(job)}`);
  }
});

/// The names a secret may be handed over as, read from the harness that leaves them out of every
/// child's environment, so that the two cannot drift: one definition, in build.ts.
/// It is read from build.ts's tokens, so that a comment or a string naming it is none, and it must
/// be defined there exactly once, as `const SECRET_NAME = /.../i;`.
export function secretNameOf(text) {
  const toks = jsTokens(text);
  const found = toks.flatMap((x, i) =>
    x.t === "id" && x.v === "const" && toks[i + 1]?.v === "SECRET_NAME" && toks[i + 2]?.v === "=" && toks[i + 3]?.t === "re" && toks[i + 4]?.v === ";" ? [toks[i + 3].v] : [],
  );
  const defined = toks.filter((x, i) => x.t === "id" && x.v === "SECRET_NAME" && /^(const|let|var|function|class)$/.test(toks[i - 1]?.v ?? "")).length;
  if (found.length !== 1 || defined !== 1) return null;
  const m = /^\/(.+)\/i$/.exec(found[0]);
  return m ? new RegExp(m[1], "i") : null;
}
const SECRET_NAME = (() => {
  const name = secretNameOf(read("benchmark/harness/build.ts"));
  assert.ok(name, "benchmark/harness/build.ts does not define SECRET_NAME exactly once, as a case-blind regular expression");
  return name;
})();
/// The action inputs a secret may be handed to, each an action by its repository and the input:
/// the checkout's token, which it keeps from the logs and from what it writes. Any other input of
/// any action may show it, as `actions/cache`'s key does in the list of caches (the review of
/// 41a54b6).
const SECRET_INPUTS = new Set(["actions/checkout#token"]);
/// The one form a secret is handed over in: the whole value, the secret alone, so that no
/// expression turns it into something no name or mask follows (`format('{0}', secrets.X)`).
const SECRET_VALUE = /^\$\{\{ secrets\.[A-Z][A-Z0-9_]* \}\}$/;

/// Where a job holds a secret other than as the value of a step's or the job's environment
/// variable, or of a step's action input, whose name SECRET_NAME matches: a secret in a script, in
/// the workflow's env, under a name that does not look like one (the review of b2be87b: a `*_PAT`
/// would reach a child of the harness, which leaves out only names that look like a secret's).
export function secretPlaceProblems(key, doc, job) {
  const at = [];
  const walk = (v, where) => {
    if (typeof v === "string") {
      if (holdsSecret(v)) at.push(where);
    } else if (Array.isArray(v)) v.forEach((x, i) => walk(x, `${where}[${i}]`));
    else if (v !== null && typeof v === "object") {
      for (const [k, x] of Object.entries(v)) {
        if (holdsSecret(k)) at.push(`${where}.<a key>`);
        walk(x, `${where}.${k}`);
      }
    }
  };
  walk(doc.env ?? {}, "workflow.env");
  walk(job, "job");
  const value = (where) => where.replace(/^job/, "").split(/\.|\[(\d+)\]/).filter(Boolean).reduce((v, k) => v?.[k], job);
  // An expression that reads a secret-named variable back out of the environment, or the whole
  // environment, hands the secret on as `secrets.` would: in an action's input, an artifact's
  // name, a script's output (the final pass of ad38011: `${{ env.ANTHROPIC_API_KEY }}` in a
  // cache's key).
  const fromEnv = [];
  const read = (v, where) => {
    if (typeof v === "string") {
      const k = v.indexOf("${{");
      if (k < 0) return;
      // Allowed, not refused: every `env` of an expression must be `env.<name>` or `env['<name>']`
      // with a name that is no secret's; anything else, `env.*`, `env[<expression>]` or `env`
      // alone, counts as reading the whole environment (the review of 75e5eeb: `join(env.*, '-')`
      // and `env[format(...)]` matched none of the forms refused before).
      const text = v.slice(k);
      for (const m of text.matchAll(/(?<![.\w])env\b/gi)) {
        const after = text.slice(m.index + m[0].length);
        const named = /^\s*\.\s*([A-Za-z_][\w-]*)(?![\w-]*\s*[([*])/.exec(after) ?? /^\s*\[\s*'([A-Za-z_][\w-]*)'\s*\]/.exec(after);
        if (!named || SECRET_NAME.test(named[1])) fromEnv.push(`${key}: ${where} reads ${named ? named[1] : "the whole environment"} out of the environment in an expression`);
      }
    } else if (Array.isArray(v)) v.forEach((x, i) => read(x, `${where}[${i}]`));
    else if (v !== null && typeof v === "object") for (const [k, x] of Object.entries(v)) read(x, `${where}.${k}`);
  };
  read(doc.env ?? {}, "workflow.env");
  read(job, "job");
  return at
    .filter((where) => {
      // Only a step's own environment, which no other step of the job inherits.
      const env = /^job\.steps\[\d+\]\.env\.([A-Za-z_][\w-]*)$/.exec(where);
      if (env) return !(SECRET_NAME.test(env[1]) && SECRET_VALUE.test(String(value(where))));
      const input = /^job\.steps\[(\d+)\]\.with\.([A-Za-z_][\w-]*)$/.exec(where);
      if (input) {
        const action = String(steps(job)[Number(input[1])]?.uses ?? "").replace(/@.*$/, "");
        return !(SECRET_INPUTS.has(`${action}#${input[2]}`) && SECRET_VALUE.test(String(value(where))));
      }
      return true;
    })
    .map((where) => `${key}: a secret at ${where}, not the whole value of a step's environment variable whose name matches ${SECRET_NAME} or of an input of SECRET_INPUTS`)
    .concat(fromEnv);
}

check("every secret is the value of an environment variable or an action input whose name looks like a secret's", () => {
  for (const { file, doc, id, job } of allJobs()) assert.deepEqual(secretPlaceProblems(`${file}#${id}`, doc, job), []);
  const s = "${{ secrets.ANTHROPIC_API_KEY }}";
  for (const [doc, job] of [
    [{}, { steps: [{ env: { BENCH_PAT: s } }] }],
    [{}, { env: { HELD_OUT: s } }],
    [{}, { steps: [{ uses: "actions/checkout@0", with: { ref: s } }] }],
    [{}, { steps: [{ run: `export ANTHROPIC_API_KEY=${s}` }] }],
    [{ env: { ANTHROPIC_API_KEY: s } }, { steps: [] }],
    [{}, { steps: [{ if: "${{ secrets.ANTHROPIC_API_KEY == 'x' }}", run: "true" }] }],
    // The review of 41a54b6: an input of another action, or of the checkout but its token, and a
    // secret turned by an expression.
    [{}, { steps: [{ uses: "actions/cache@0", with: { path: "x", key: s } }] }],
    [{}, { steps: [{ uses: "actions/checkout@0", with: { "ssh-key": s } }] }],
    [{}, { steps: [{ uses: "someone/checkout@0", with: { token: s } }] }],
    [{}, { steps: [{ env: { ANTHROPIC_API_KEY: "${{ format('{0}', secrets.ANTHROPIC_API_KEY) }}" } }] }],
    [{}, { steps: [{ env: { ANTHROPIC_API_KEY: `x${s}` } }] }],
    [{}, { steps: [{ uses: "actions/checkout@0", with: { token: "${{ secrets.A || github.token }}" } }] }],
    // The final pass of ad38011: the secret in the job's env, read back by `env.`.
    [{}, { env: { ANTHROPIC_API_KEY: s }, steps: [] }],
    [{}, { steps: [{ uses: "actions/cache@0", with: { key: "${{ env.ANTHROPIC_API_KEY }}" } }] }],
    [{}, { steps: [{ uses: "actions/upload-artifact@0", with: { name: "x-${{ env['MENTE_BENCH_HELDOUT_TOKEN'] }}" } }] }],
    [{}, { steps: [{ run: "echo ${{ toJSON(env) }} >> $GITHUB_OUTPUT" }] }],
    // The review of 75e5eeb: the whole environment by a filter or a computed name.
    [{}, { steps: [{ uses: "actions/cache@0", with: { key: "${{ join(env.*, '-') }}" } }] }],
    [{}, { steps: [{ uses: "actions/cache@0", with: { key: "${{ toJSON(env.*) }}" } }] }],
    [{}, { steps: [{ uses: "actions/cache@0", with: { key: "${{ env[format('{0}_API_KEY', 'ANTHROPIC')] }}" } }] }],
    [{}, { steps: [{ uses: "actions/cache@0", with: { key: "${{ env[matrix.k] }}" } }] }],
  ]) {
    assert.ok(secretPlaceProblems("x.yml#y", doc, job).length > 0, `a secret in the wrong place let through: ${JSON.stringify({ doc, job })}`);
  }
  assert.deepEqual(secretPlaceProblems("x.yml#y", {}, { env: { HAS: "${{ secrets.A != '' }}" }, steps: [{ env: { ANTHROPIC_API_KEY: s } }, { uses: "actions/checkout@0", with: { token: s } }, { uses: "actions/cache@0", with: { key: "x-${{ env.MDBOOK_VERSION }}-${{ inputs.env }}" } }] }), []);
  // SECRET_NAME is read from tokens, defined once (the review of 41a54b6: the first match of a
  // regular expression could have been a comment).
  assert.equal(String(secretNameOf("// const SECRET_NAME = /x/i;\nconst SECRET_NAME = /key|token/i;")), "/key|token/i");
  for (const text of ["const SECRET_NAME = /key/;", "const SECRET_NAME = /a/i;\nconst SECRET_NAME = /b/i;", "const SECRET_NAME = new RegExp('key', 'i');", '"const SECRET_NAME = /key/i;"', "const SECRET_NAME = /key/i;\nfunction SECRET_NAME() {}"]) {
    assert.equal(secretNameOf(text), null, `SECRET_NAME read from: ${text}`);
  }
});

check("every job that holds a secret is pinned whole, takes mn from a job that checks it first, and runs agent code only in its proven sandbox", () => {
  const found = new Set();
  const holds = (v) => holdsSecret(v);
  for (const { file, doc, id, job } of allJobs()) {
    const key = `${file}#${id}`;
    const all = steps(job);
    // The workflow's env, anything of the job but its steps, and a job that passes secrets on.
    const wide = holds(doc.env) || holds({ ...job, steps: null }) || "secrets" in job;
    const secretAt = wide ? 0 : all.findIndex((s) => holds(s));
    if (secretAt < 0) continue;
    found.add(key);
    const held = SECRET_JOBS.get(key);
    assert.ok(held, `${key} holds a secret and is not in SECRET_JOBS: name the job that builds mn for it, if any`);
    assert.ok(PINNED_JOBS.has(key), `${key} holds a secret and is not pinned whole`);
    // Every file it runs, and every module those import, is the code owner's: the pin holds the
    // job's steps, and CODEOWNERS what they run (the final review of PR 137 found the toolchains'
    // installer and five modules of the process that holds the key in neither).
    const runs = [...repoPaths(scriptTexts(doc, job))];
    const loaded = runs.flatMap((f) => COMPUTED_LOADS.get(f)?.loads ?? []);
    const unowned = [...runs, ...loaded].filter((f) => !GATE_FILES.some((p) => owns(p, f)));
    assert.deepEqual(unowned, [], `${key} holds a secret and runs files .github/CODEOWNERS does not give its owner`);
    // And each module of it loads only what that closure follows, and starts processes only in
    // SPAWNERS.
    assert.deepEqual(runs.filter((f) => JS_MODULE.test(f) || runs.some((g) => JS_MODULE.test(g) && readSecretModule(g, read(g)).imports.includes(f))).flatMap((f) => secretModuleProblems(f, read(f))), [], `${key} holds a secret, and a module it runs loads code or starts a process the policy cannot read`);
    // And every node it starts generates no code from strings, which no spelling of a module gets
    // past: eval, Function and its kin, reached by any route, throw.
    for (const s of all.filter((x) => typeof x.run === "string")) {
      for (const l of logicalLines(s.run)) assert.doesNotMatch(l, NODE_WITHOUT_GUARD, `${stepKey(file, id, s)} starts node without --disallow-code-generation-from-strings: ${l}`);
    }
    if (held.builtBy !== null) {
      assert.ok([job.needs].flat().includes(held.builtBy), `${key} does not need ${held.builtBy}, the job that builds mn for it`);
      const builder = doc.jobs?.[held.builtBy];
      assert.ok(builder, `${file} has no job ${held.builtBy}`);
      assert.deepEqual(beforeToolchainProblems(file, held.builtBy, doc, builder), [], `${file}#${held.builtBy} builds mn for a job that holds a secret without the build-time check first`);
      assert.ok(!holds(builder) && !("secrets" in builder), `${file}#${held.builtBy}, which builds mn, holds a secret`);
      assert.ok(all.some((s) => /^actions\/download-artifact@/.test(String(s.uses ?? ""))), `${key} takes no artifact from ${held.builtBy}`);
    }
    if (held.agents) {
      const check = all.findIndex((s) => /benchmark\/harness\/sandbox-check\.ts/.test(String(s.run ?? "")));
      assert.ok(check >= 0 && check < secretAt, `${key}: runs agent code, but sandbox-check.ts does not run before the secret is in reach`);
      for (const k of ["if", "continue-on-error"]) assert.ok(!(k in all[check]), `${key}: the sandbox check carries ${k}:`);
      // The step that runs agent code, which holds no secret and talks to the broker, runs it all
      // through the sandbox.
      const runs = all.filter((s) => /benchmark\/harness\/(bench|prelim)\.ts/.test(String(s.run ?? "")));
      assert.equal(runs.length, 1, `${key} runs the harness in ${runs.length} steps, not one`);
      assert.match(runs[0].run, /--sandbox "\$SANDBOX"/, `${key}, step "${runs[0].name}": runs agent code, but not through the sandbox`);
    }
    assert.deepEqual(brokerProblems(key, job, doc), []);
  }
  assert.deepEqual([...SECRET_JOBS.keys()].filter((k) => !found.has(k)), [], "SECRET_JOBS names jobs that hold no secret");
  // The final review's plants, and the other spellings of a secret.
  for (const held of ["${{ secrets.ANTHROPIC_API_KEY }}", "${{ secrets['ANTHROPIC_API_KEY'] }}", "${{ toJSON(secrets) }}", "${{ secrets.A || 'x' }}", "${{ fromJSON(toJSON(secrets)).A }}", { env: { K: "${{ secrets['K'] }}" } }]) {
    assert.ok(holdsSecret(held), `a secret read as none: ${JSON.stringify(held)}`);
  }
  for (const none of ["${{ secrets.MENTE_BENCH_HELDOUT_TOKEN != '' }}", "${{ github.token }}", "secrets", "${{ env.SECRETS_DIR }}"]) assert.ok(!holdsSecret(none), `no secret read as one: ${none}`);
  // The review of 35bbe0f: a tab or a line break inside the expression, and the context in another case.
  for (const held of ["${{\tsecrets.ANTHROPIC_API_KEY }}", "${{\n  secrets.ANTHROPIC_API_KEY\n}}", "${{ SECRETS.A }}", "${{ Secrets['A'] }}", { steps: [{ env: { K: "${{\tsecrets.K }}" } }] }]) {
    assert.ok(holdsSecret(held), `a secret read as none: ${JSON.stringify(held)}`);
  }
  // And the modules: a load the policy cannot follow, or a process started outside SPAWNERS.
  for (const [file, text] of [
    ["benchmark/harness/api.ts", 'const m = await import(new URL("../" + "reference.mjs", import.meta.url));'],
    ["benchmark/harness/api.ts", "const r = createRequire(import.meta.url);"],
    ["benchmark/harness/tasks.ts", 'import { execSync as sh } from "child_process";\nsh(`${mn} explain ${code}`);'],
    ["benchmark/harness/tasks.ts", 'const cp = await import("node:child_process");'],
    ["benchmark/harness/build.ts", 'import { spawn, execSync } from "node:child_process";\nspawn(c, a);'],
    ["benchmark/harness/build.ts", 'import { spawn } from "node:child_process";\nspawn(c, a);\nspawn(mn, b);'],
    ["benchmark/check.mjs", 'import { execFileSync } from "node:child_process";\nexecFileSync("node", ["x.mjs"]);'],
    ["benchmark/harness/report.ts", "new Worker(file);"],
    ["benchmark/harness/report.ts", "eval(code);"],
    // The review of b2be87b, run and on paper: a loader's hook, a computed import behind a
    // comment, a built-in module by a computed name, Function and eval by other routes, an import
    // with no space, the loader's internals, a worker by another name, a native binding, and
    // specifiers neither relative nor of NODE_MODULES.
    ["benchmark/harness/api.ts", 'import { register } from "node:module";\nregister("../../docs/hooks.mjs", import.meta.url);'],
    ["benchmark/harness/api.ts", 'register("../../docs/hooks.mjs", import.meta.url);'],
    ["benchmark/harness/api.ts", "const m = await import/**/(process.env.X);"],
    ["benchmark/harness/api.ts", 'process.getBuiltinModule("child_" + "process").spawnSync(mn, ["test"], { env: process.env });'],
    ["benchmark/harness/api.ts", "Function(code)();"],
    ["benchmark/harness/api.ts", "(0, eval)(code);"],
    ["benchmark/harness/api.ts", 'import"../../docs/leak2.mjs";'],
    ["benchmark/harness/api.ts", 'import { x } from"../../docs/leak2.mjs";'],
    ["benchmark/harness/api.ts", "M._load(path, null, false);"],
    ["benchmark/harness/api.ts", "const vm = await import(`node:vm`);"],
    ["benchmark/harness/api.ts", 'import { Worker as W } from "node:worker_threads";\nnew W(code, { eval: true });'],
    ["benchmark/harness/api.ts", 'process.binding("spawn_sync");'],
    ["benchmark/harness/api.ts", 'import { x } from "/home/runner/x.mjs";'],
    ["benchmark/harness/api.ts", 'import { x } from "file:///home/runner/x.mjs";'],
    ["benchmark/harness/api.ts", 'import { runInThisContext } from"node:vm";'],
    ["benchmark/harness/api.ts", 'import * as fs from "fs";'],
    // And the routes to Function and to a computed global that a name alone does not show.
    ["benchmark/harness/api.ts", 'const f = (() => {})["constr" + "uctor"];'],
    ["benchmark/harness/api.ts", "const f = (() => {}).constructor;"],
    ["benchmark/harness/api.ts", "const { constructor: F } = Object.getPrototypeOf(async () => {});"],
    ["benchmark/harness/api.ts", 'const g = process["getBuiltinModule"];'],
    ["benchmark/harness/api.ts", 'const g = process[name];'],
    ["benchmark/harness/api.ts", "\\u0065val(code);"],
    ["benchmark/harness/api.ts", 'const e = "ev\\x61l";'],
    ["benchmark/harness/api.ts", `const e = "${String.fromCharCode(0x435)}val";`],
    ["benchmark/harness/api.ts", "const s = 'unclosed;\nimport(x);"],
    ["benchmark/harness/api.ts", 'const y = await import("./y.ts");'],
    // The review of 41a54b6: process past its allowed members, aliased or destructured, and a
    // member by any computed key, a plain one included.
    ["benchmark/harness/api.ts", 'process.execve("/bin/sh", ["sh", "-c", "id"], process.env);'],
    ["benchmark/harness/api.ts", 'const p = process;\np["getBuilt" + "inModule"]("node:child_" + "process");'],
    ["benchmark/harness/api.ts", "const { [k]: g } = process;"],
    ["benchmark/harness/api.ts", "const { env, getBuiltinModule: g } = process;"],
    ["benchmark/harness/api.ts", "f(process);"],
    ["benchmark/harness/api.ts", "process?.env;"],
    ["benchmark/harness/api.ts", '(() => 0)["constr" + "uctor"]("return 1")();'],
    ["benchmark/harness/api.ts", "const v = xs[i];"],
    ["benchmark/harness/api.ts", "const v = f(x)[0];"],
    ["benchmark/harness/api.ts", "const v = x?.[k];"],
    ["benchmark/harness/api.ts", 'const v = "abc"[k];'],
    ["benchmark/harness/api.ts", "const v = (a || b)[k];"],
    ["benchmark/harness/api.ts", "obj[k] = v;"],
    ["benchmark/harness/api.ts", "Object.defineProperty(Object.prototype, \"shell\", { value: \"/bin/sh\" });"],
    ["benchmark/own.mjs", 'export const own = (o, k) => o[k];\n'],
    // The re-check of da58409: a module of another kind, imported or run, and CommonJS's names.
    ["benchmark/harness/api.ts", 'import { x } from "./tasks.cts";'],
    ["benchmark/harness/api.ts", 'import { x } from "../reference.cjs";'],
    ["benchmark/harness/api.ts", 'import data from "./pins.json" with { type: "json" };'],
    ["benchmark/harness/leak.cjs", "const [, r] = arguments;\nr(\"node:child_\" + \"process\");"],
    ["benchmark/harness/leak.cts", "export const x = 1;"],
    ["benchmark/harness/api.ts", "const [, r] = arguments;"],
    ["benchmark/harness/api.ts", "module.require(name);"],
    ["benchmark/harness/api.ts", "exports.x = 1;"],
    // The review of 75e5eeb: a second import of child_process with an escape in its path.
    ["benchmark/harness/build.ts", `${read("benchmark/harness/build.ts")}\nimport { execSync } from "node:child\\u005fprocess";\nexport const sh = (c) => execSync(c);\n`],
    ["benchmark/harness/api.ts", 'const m = "node:child\\u005fprocess";'],
    // The final pass of ad38011: a name written as a string in an export's list.
    ["benchmark/harness/api.ts", 'export { default as z, "q" as w } from "node:child_process";'],
    ["benchmark/harness/api.ts", 'export { "x" as y } from "../../docs/leak.mjs";'],
    ["benchmark/harness/api.ts", 'const m = "child_process";'],
    // And the computed accesses the reading missed: a computed key in a pattern, an object or a
    // class, a keyword as a property's or a method's name, and an expression's non-null `!`.
    ["benchmark/harness/api.ts", "const { [k]: F } = Array.of;"],
    ["benchmark/harness/api.ts", "const o = { [k]: 1 };"],
    ["benchmark/harness/api.ts", "const o = { a: 1, [k]: 2 };"],
    ["benchmark/harness/api.ts", "class A {\n  x = 1;\n  [k]() {}\n}"],
    ["benchmark/harness/api.ts", "const f = Array.of[k];"],
    ["benchmark/harness/api.ts", "const f = m.delete[k];"],
    ["benchmark/harness/api.ts", "const f = o.catch(x)[k];"],
    ["benchmark/harness/api.ts", "const f = o?.for[k];"],
    ["benchmark/harness/api.ts", "const f = g![k];"],
    // A spawner past its named uses: an alias of what it imports, or the checker's git with other
    // arguments, or by another name.
    ["benchmark/harness/build.ts", 'import { spawn } from "node:child_process";\nconst s = spawn;\ns(mn, b);'],
    ["benchmark/harness/prelim.ts", 'import { spawnSync } from "node:child_process";\nconst r = [spawnSync][0](mn, b);'],
  ]) {
    assert.ok(secretModuleProblems(file, text).length > 0, `the rule for a secret job's modules lets through, in ${file}: ${text}`);
  }
  // The reading follows an import however it is spaced, and a comment or a string holds none.
  assert.deepEqual(
    jsImports(jsTokens('import"./a.mjs";import{b}from"./b.mjs";export*from"./c.mjs";// import "./d.mjs"\nconst s = "import \'./e.mjs\'";')).specifiers.map((s) => s.v),
    ["./a.mjs", "./b.mjs", "./c.mjs"],
  );
  assert.deepEqual(secretModuleProblems("benchmark/harness/api.ts", 'import { x } from "./tasks.ts";\nimport fs from "node:fs";\n// eval(code)\nconst s = "rpn-eval";\nclass A {\n  constructor(t) { this.t = t; }\n}\nconst v = own(xs, i) + own(m, "k") + process.env.HOME + process.argv.length;\nconst [a, b] = xs;\nlet [c] = ys;\nfor (const [k, w] of zs) if (k) [a, b] = [b, a];\nconst t: string[] = [];\nf(a, [b, c]);\nconst n = ![x].length;\nconst o = { a: [1, 2], b };\nx = 1;\n[a, b] = [b, a];'), []);
  assert.deepEqual(secretModuleProblems("benchmark/own.mjs", read("benchmark/own.mjs")), [], "benchmark/own.mjs is not the text the policy holds");
  // The review of b2be87b: a `}}` inside a quoted string of the expression, and the named load of
  // check.mjs moved or joined by another.
  for (const held of ["${{ '}}' && secrets.ANTHROPIC_API_KEY }}", "${{ format('{0}}}', secrets.ANTHROPIC_API_KEY) }}", "${{ secrets.A != '' }} ${{ secrets.B }}"]) {
    assert.ok(holdsSecret(held), `a secret read as none: ${held}`);
  }
  const checker = read("benchmark/check.mjs");
  const named = COMPUTED_LOADS.get("benchmark/check.mjs").line;
  assert.deepEqual(secretModuleProblems("benchmark/check.mjs", checker), []);
  for (const plant of [checker.replace(named, named.replace('path.join(dir, "reference.mjs")', 'path.join(dir, "tasks", "reference.mjs")')), `${checker}\nconst extra = await import(process.env.BENCH_EXTRA ?? "node:os");\n`, `${checker}\n${named}\n`]) {
    assert.ok(secretModuleProblems("benchmark/check.mjs", plant).length > 0, "the named load of check.mjs lets through a moved or a second load");
  }
  // The checker's git, held to its two named calls: other arguments (`-c core.pager=...` runs
  // anything), a third call, an alias, or a named call moved into a string.
  const dirty = '    const dirty = git(heldOutDir, "status", "--porcelain");';
  for (const plant of [
    checker.replace(dirty, '    const dirty = git(heldOutDir, "-c", "core.pager=sh -c id", "log");'),
    `${checker}\ngit(".", "-c", "core.fsmonitor=id", "status");\n`,
    `${checker}\nconst g = git;\ng(".", "status");\n`,
    checker.replace(dirty, `    const dirty = \`\n${dirty}\n\`;\n    const g = git;`),
    checker.replace('"/usr/bin/git", ["-C", dir, ...args]', '"/bin/sh", ["-c", ...args]'),
    checker.replace('"/usr/bin/git", ["-C"', '"git", ["-C"'),
  ]) {
    assert.ok(secretModuleProblems("benchmark/check.mjs", plant).length > 0, "the checker's git lets through a call it does not name");
  }
});
