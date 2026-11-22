// The workflow policy's reading of the workflows (README.md in this directory): every workflow
// parsed, its jobs and their checks' names, the required ones, and the forms a step's condition takes.

import fs from "node:fs";
import path from "node:path";
import { parseYaml } from "../lib/yaml.mjs";
import { ALWAYS, PATHS_FILE, decide, globToRegExp, matches, parsePaths } from "../lib/watch.mjs";
import { JS_MODULE, jsImports, jsTokens } from "./js.mjs";
import { ROOT, filesUnder, read, tracked } from "./repo.mjs";
export const WORKFLOWS = ".github/workflows";
export const REQUIRED_FILE = ".github/required-checks.txt";
export const WATCH = "steps.watch.outputs.run != 'false'";
/// The conditions a step after the watch may carry: the watch's decision, alone or after
/// `!cancelled()`, which still runs a check when an earlier one failed.
export const WATCH_FORMS = [WATCH, `!cancelled() && ${WATCH}`];
/// The conditions that run a step only when another failed, or even when the run was cancelled:
/// for a step that keeps or reports what the others found, never for a check, which they would
/// skip whenever the job is passing.
export const REPORT_FORMS = [`always() && ${WATCH}`, `failure() && ${WATCH}`];
/// The steps of watch jobs, beside the uploads of artifacts, that only report: workflow, job, step.
export const REPORT_STEPS = new Set(["generated.yml#paths#Summary"]);
export const isReport = (file, id, s) => String(s.uses ?? "").startsWith("actions/upload-artifact@") || REPORT_STEPS.has(stepKey(file, id, s));
/// The shells a step may run in, named bare: a shell given as a template (`true {0}`) could run
/// nothing and pass. The one template allowed, SECRET_SHELL, stands only on a step that is
/// HELD_OUT_STEP or BROKER_STEP exactly, whose script the policy holds too.
export const SHELLS = new Set(["bash", "sh", "pwsh"]);

export const normalize = (expression) =>
  String(expression)
    .trim()
    .replace(/^\$\{\{\s*([\s\S]*?)\s*\}\}$/, "$1")
    .replace(/\s+/g, " ");

export function workflows() {
  return fs
    .readdirSync(path.join(ROOT, WORKFLOWS))
    .filter((f) => f.endsWith(".yml") || f.endsWith(".yaml"))
    .sort()
    .map((file) => ({ file, doc: parseYaml(read(`${WORKFLOWS}/${file}`)) }));
}

/// The events a workflow runs on, as an object whatever form `on:` takes.
export function triggers(doc) {
  const on = doc.on;
  if (typeof on === "string") return { [on]: null };
  if (Array.isArray(on)) return Object.fromEntries(on.map((e) => [e, null]));
  return on ?? {};
}

/// The names GitHub gives a job's checks: its `name:`, or its id, once for each combination of its
/// matrix, with `${{ matrix.<key> }}` filled in, or, where the name has no matrix expression, the
/// combination's values appended in parentheses.
export function checkNames(id, job) {
  const template = String(job.name ?? id);
  const matrix = job.strategy?.matrix;
  if (!matrix || typeof matrix !== "object") return [template];
  const keys = Object.keys(matrix).filter((k) => k !== "include" && k !== "exclude");
  if (keys.some((k) => !Array.isArray(matrix[k]))) return [template];
  if (matrix.exclude) throw new Error(`${id}: a matrix with exclude is not read here`);
  let combos = [{}];
  for (const k of keys) combos = combos.flatMap((c) => matrix[k].map((v) => ({ ...c, [k]: v })));
  if (matrix.include) {
    if (keys.length > 0) throw new Error(`${id}: a matrix with both keys and include is not read here`);
    combos = matrix.include;
  }
  const expression = () => /\$\{\{\s*matrix\.([\w-]+)\s*\}\}/g;
  return combos.map((c) =>
    expression().test(template)
      ? template.replace(expression(), (all, key) => (key in c ? String(c[key]) : all))
      : `${template} (${Object.values(c).map(String).join(", ")})`,
  );
}

export function allJobs() {
  return workflows().flatMap(({ file, doc }) =>
    Object.entries(doc.jobs ?? {}).map(([id, job]) => ({ file, doc, id, job, names: checkNames(id, job) })),
  );
}

export function requiredChecks() {
  return read(REQUIRED_FILE)
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l !== "" && !l.startsWith("#"));
}

export function requiredJobs() {
  const required = new Set(requiredChecks());
  return allJobs().filter((j) => j.names.some((n) => required.has(n)));
}

export const steps = (job) => (Array.isArray(job.steps) ? job.steps : []);
export const stepKey = (file, id, s) => `${file}#${id}#${s.name ?? s.uses ?? String(s.run).split("\n")[0]}`;
export const hasWatch = (job) => steps(job).some((s) => s.id === "watch" || /watch\.mjs/.test(String(s.run ?? "")));

/// The one job that publishes the site: the only one whose token may write Pages or an OIDC token,
/// and whose environment is github-pages.
export const PAGES_DEPLOY_JOB = "pages-publish.yml#deploy";
/// The nightly alarms: the last job of each workflow that runs on a schedule, the only jobs whose
/// token may write issues (Norman, 2026-11-21).
export const NIGHTLY_ALARM_JOBS = ["fuzz-nightly.yml#alarm", "generated-nightly.yml#alarm", "mutants-full.yml#alarm"];

/// GitHub's permission scopes, each with the values it admits, from the workflow syntax's
/// `permissions` (read on 2026-11-16): `id-token` and `copilot-requests` take write or none,
/// `vulnerability-alerts` read or none, every other scope read, write or none.
export const READ_WRITE_NONE = ["read", "write", "none"];
export const PERMISSION_SCOPES = new Map([
  ["actions", READ_WRITE_NONE],
  ["artifact-metadata", READ_WRITE_NONE],
  ["attestations", READ_WRITE_NONE],
  ["checks", READ_WRITE_NONE],
  ["code-quality", READ_WRITE_NONE],
  ["contents", READ_WRITE_NONE],
  ["copilot-requests", ["write", "none"]],
  ["deployments", READ_WRITE_NONE],
  ["discussions", READ_WRITE_NONE],
  ["id-token", ["write", "none"]],
  ["issues", READ_WRITE_NONE],
  ["packages", READ_WRITE_NONE],
  ["pages", READ_WRITE_NONE],
  ["pull-requests", READ_WRITE_NONE],
  ["security-events", READ_WRITE_NONE],
  ["statuses", READ_WRITE_NONE],
  ["vulnerability-alerts", ["read", "none"]],
]);

/**
 * What is wrong with a workflow's permissions, at its top and in each job. They are an allow-list,
 * not a reading of what GitHub might accept: the shorthand `read-all`, or a map whose every key is a
 * scope of PERMISSION_SCOPES and whose every value is one that scope admits, exactly as written.
 * Anything else is refused, other cases and expressions (`${{ }}`) included. And no token may post
 * a check run or a commit status: `write-all`, `checks: write` and `statuses: write` are refused.
 * GitHub matches a required check by its name and its source, and every required check of main
 * comes from GitHub Actions, as such a post would. The probe of policy-runtime-judge (2026-11-16,
 * judge-probe's pull request 3) saw a change's own workflow raise its token so and post a success
 * status and a success check run under the judge's name. The judge posts through its own App, so
 * no workflow needs either permission. And only PAGES_DEPLOY_JOB's token may write Pages or an
 * OIDC token, which publish the site (the review of 7ba7a5c). And only a nightly alarm's token may
 * write issues: the job named alarm of a workflow that runs on a schedule.
 */
export function permissionProblems(file, doc) {
  const found = [];
  const readPermissions = (where, p, key = null) => {
    if (p === "read-all") return;
    if (p === "write-all") {
      found.push(`${where}: write-all, by which its token may post check runs or commit statuses`);
      return;
    }
    if (!isPlain(p)) {
      found.push(`${where}: permissions that are neither read-all nor a map of GitHub's scopes: ${JSON.stringify(p)}`);
      return;
    }
    for (const [scope, value] of Object.entries(p)) {
      const admits = PERMISSION_SCOPES.get(scope);
      if (admits === undefined) found.push(`${where}: ${JSON.stringify(scope)}, no permission scope of GitHub's`);
      else if (!admits.includes(value)) found.push(`${where}: ${scope}: ${JSON.stringify(value)}, none of ${admits.join(", ")}`);
      else if ((scope === "checks" || scope === "statuses") && value === "write") found.push(`${where}: ${scope}: write, by which its token may post check runs or commit statuses`);
      else if ((scope === "pages" || scope === "id-token") && value === "write" && key !== PAGES_DEPLOY_JOB) found.push(`${where}: ${scope}: write, which only ${PAGES_DEPLOY_JOB} may hold, to publish the site`);
      else if (scope === "issues" && value === "write" && !(key === `${file}#alarm` && "schedule" in triggers(doc))) found.push(`${where}: issues: write, which only a nightly alarm may hold, the job alarm of a workflow that runs on a schedule`);
    }
  };
  // A workflow's own permissions are required by the check above; a job's may be left out, and
  // then are the workflow's.
  if (doc.permissions !== undefined) readPermissions(file, doc.permissions);
  for (const [id, job] of Object.entries(isPlain(doc.jobs) ? doc.jobs : {})) {
    if (isPlain(job) && "permissions" in job) readPermissions(`${file}, job ${id}`, job.permissions, `${file}#${id}`);
  }
  return found;
}
export const isPlain = (v) => v !== null && typeof v === "object" && !Array.isArray(v);

/// The events a workflow holding a required job may run on: a pull request, the merge queue and a
/// push to main. A dispatched or scheduled run, or one with inputs, could report the job's check
/// on a pull request's head with the work it chose.
export const REQUIRED_EVENTS = new Set(["pull_request", "merge_group", "push"]);

/// The logical lines of a script: each continued line joined to the next by one space.
export const logicalLines = (script) => String(script).replace(/[ \t]*\\\r?\n[ \t]*/g, " ").split("\n");

/// The texts a job's scripts read: every run: script, every env value (the workflow's, the job's,
/// each step's), and the package.json script behind every `npm run`, recursively.
export function scriptTexts(doc, job) {
  const texts = [];
  const scripts = JSON.parse(read("package.json")).scripts ?? {};
  const addNpm = (text, seen) => {
    for (const m of text.matchAll(/\bnpm run ([\w:-]+)/g)) {
      if (seen.has(m[1]) || !(m[1] in scripts)) continue;
      seen.add(m[1]);
      texts.push(scripts[m[1]], "package.json");
      addNpm(scripts[m[1]], seen);
    }
  };
  const envs = [doc.env, job.env, ...steps(job).map((s) => s.env)].filter(Boolean);
  for (const env of envs) texts.push(...Object.values(env).map(String));
  for (const s of steps(job)) {
    if (typeof s.run !== "string") continue;
    texts.push(s.run);
    addNpm(s.run, new Set());
  }
  return texts;
}

/// The tracked files the texts name: a file, every file under a directory, every file a glob
/// matches, and, for a script, every file it imports, recursively. Shell comments are left out.
export function repoPaths(texts) {
  const files = tracked();
  const out = new Set();
  // A file a script names is read for imports when it is a module by its name; a file a module
  // imports is read whatever its name, so that the closure misses no module of another kind (the
  // re-check of da58409: a `.cts` imported by a module of a secret job was never read).
  const addScript = (file, imported = false) => {
    if (out.has(file)) return;
    out.add(file);
    if (!imported && !JS_MODULE.test(file)) return;
    // Read from its tokens, so that no spacing or comment hides an import from the closure (the
    // review of b2be87b: `import"../../docs/leak2.mjs"` was past the regular expression before).
    for (const { v } of jsImports(jsTokens(read(file))).specifiers.filter((s) => /^\.{1,2}\//.test(s.v))) {
      const target = path.posix.normalize(path.posix.join(path.posix.dirname(file), v));
      if (files.has(target)) addScript(target, true);
    }
  };
  for (const text of texts) {
    const code = text
      .split("\n")
      .filter((l) => !/^\s*#/.test(l))
      .map((l) => l.replace(/\s#\s.*$/, ""))
      .join("\n");
    for (let token of code.split(/[\s;|&()<>"'`=,{}]+/)) {
      token = token.replace(/^\.\//, "").replace(/\/+$/, "");
      if (token === "" || token.includes("$") || token.startsWith("-") || token.startsWith("/")) continue;
      if (token.includes("*")) {
        // A glob names paths only with a directory in it: a bare `*)` is a shell case's default.
        if (!/[\w.-]\//.test(token)) continue;
        const re = globToRegExp(token);
        [...files].filter((f) => re.test(f)).forEach(addScript);
      } else if (files.has(token)) addScript(token);
      else filesUnder(token).forEach(addScript);
    }
  }
  return out;
}
