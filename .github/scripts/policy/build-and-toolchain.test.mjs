// The workflow policy: the build and its toolchain (README.md in this directory). Overflow checks,
// the pinned toolchain, and nothing of a change running where cargo builds.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { BUILD_TIME_CODE, CRATES_IO, approvedCrates, listedMembers, lockPackages, lockProblems, main as buildTimeCode, metadataProblems, toolchainProblems } from "../lib/build-time-code.mjs";
import { PINNED_JOBS, jobDigest } from "./pins.mjs";
import { filesUnder, read, tomlSubset, tracked, workspace } from "./repo.mjs";
import { GATE_ONLY } from "./sandbox.mjs";
import { SECRET_JOBS } from "./secrets.mjs";
import { BEFORE_TOOLCHAIN, NODE_INPUTS, PERMISSION, TOOLCHAIN_STEP, beforeToolchainProblems } from "./toolchain.mjs";
import { WORKFLOWS, allJobs, isPlain, logicalLines, requiredJobs, stepKey, steps, workflows } from "./workflows.mjs";

/// Every check of this module counts itself when it runs, and the module fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// each module's FLOOR equal to the checks it declares.
const FLOOR = 10;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks of build-and-toolchain.test.mjs ran: a skip or a filter left the rest out`);
});
/// Any spelling of the overflow-checks flag rustc and cargo accept: a hyphen or an underscore, in
/// any case (`-C overflow_checks=off` works as well as `overflow-checks`).
const OVERFLOW = /overflow[-_]checks/i;
/// The cargo profile variables a workflow may set, each with its reason.
const CARGO_PROFILE_ENV = new Map([
  // The fuzzer's release build also asserts what a debug build asserts (fuzz.yml).
  ["CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", "the fuzzer's debug assertions"],
]);

check("overflow checks stay on in every build", () => {
  for (const { file, doc } of workflows()) {
    assert.doesNotMatch(read(`${WORKFLOWS}/${file}`), OVERFLOW, `${file} names overflow checks: [profile.release] in Cargo.toml is their one source`);
    const envs = [doc.env, ...Object.values(doc.jobs ?? {}).flatMap((j) => [j.env, ...steps(j).map((s) => s.env)])].filter(Boolean);
    for (const env of envs) {
      for (const [name, value] of Object.entries(env)) {
        if (/^CARGO_PROFILE_/.test(name)) assert.ok(CARGO_PROFILE_ENV.has(name), `${file} sets ${name}, a profile Cargo.toml does not show`);
        if (/RUSTFLAGS$|^CARGO_/.test(name)) assert.doesNotMatch(String(value), /overflow/i, `${file}: ${name} touches overflow checks`);
      }
    }
  }
  for (const f of [...tracked()].filter((f) => /(^|\/)\.cargo\//.test(f))) {
    assert.doesNotMatch(read(f), OVERFLOW, `${f} names overflow checks`);
  }
  const manifests = new Map([...tracked()].filter((f) => f === "Cargo.toml" || f.endsWith("/Cargo.toml")).map((f) => [f, read(f)]));
  assert.deepEqual(overflowProblems(manifests, [...tracked()]), []);
  // What the check stops: a package's own profile, in either spelling and quoted, a release profile
  // without the checks, a .cargo/config anywhere.
  const root = "[profile.release]\noverflow-checks = true\n";
  const one = (text, files = []) => overflowProblems(new Map([["Cargo.toml", text]]), files).length;
  assert.equal(one(root), 0);
  assert.equal(one(`${root}\n[profile.release.package.mente-mir]\noverflow-checks = false\n`), 1);
  assert.equal(one(`${root}\n[profile.release.package.mente-sema]\n"overflow-checks" = false\n`), 1);
  assert.equal(one(`${root}\n[profile.dev]\n'Overflow_Checks' = false\n`), 1);
  assert.equal(one(`${root}\n[profile.release.package]\nmente-mir = { overflow-checks = false }\n`), 1);
  assert.equal(one("[profile.release]\nopt-level = 3\n"), 1);
  assert.equal(one(root, ["compiler/x/.cargo/config.toml"]), 1);
  assert.equal(one(root, ["compiler/x/.cargo/config"]), 1);
});

/// Why some build could run without overflow checks: any setting of the flag in any manifest but
/// `true`, in any spelling or quoting, a root [profile.release] that does not turn it on, or a
/// .cargo/config anywhere in the tree, which can change a profile where Cargo.toml does not show it.
export function overflowProblems(manifests, files) {
  const found = [];
  for (const [file, text] of manifests) {
    for (const m of text.matchAll(/["']?overflow[-_]checks["']?\s*=\s*([^\s,}#]+)/gi)) {
      if (m[1] !== "true") found.push(`${file} sets overflow checks to ${m[1]}`);
    }
  }
  const release = /^\[profile\.release\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(manifests.get("Cargo.toml") ?? "");
  if (!/^overflow-checks\s*=\s*true\s*$/m.test(release?.[1] ?? "")) found.push("Cargo.toml's [profile.release] does not set overflow-checks = true");
  for (const f of files) if (/(^|\/)\.cargo\/config(\.toml)?$/.test(f)) found.push(`${f} can change a profile unseen`);
  return found;
}

check("every compiler crate proves its overflow checks in a release build", () => {
  const crates = [...tracked()].filter((f) => /^compiler\/[^/]+\/Cargo\.toml$/.test(f)).map((f) => path.posix.dirname(f));
  assert.ok(crates.length > 0);
  for (const dir of crates) {
    const has = filesUnder(`${dir}/src`).some((f) => f.endsWith(".rs") && read(f).includes("fn test_arithmetic_overflow_panics_in_this_crate"));
    assert.ok(has, `${dir} has no test_arithmetic_overflow_panics_in_this_crate (src/overflow_traps.rs)`);
  }
  // The proof every release-building required job runs: every crate's test in release, each
  // crate's run on its own, counted.
  const proof = read(".github/scripts/overflow-proof.sh");
  assert.match(proof, /cargo test --release --locked -p "\$name" "\$\{targets\[@\]\}" -- --exact "\$test" > "\$log" 2>&1 \|\| status=\$\?/);
  assert.match(proof, /test=overflow_traps::test_arithmetic_overflow_panics_in_this_crate/);
  assert.match(proof, /if \[ "\$expected" -eq 0 \] \|\| \[ "\$passed" -ne "\$expected" \]; then/);
});

check("the toolchain is the release rust-toolchain.toml pins, from rustup, with nothing else", () => {
  assert.ok(!tracked().has("rust-toolchain"), "a rust-toolchain file beside rust-toolchain.toml: rustup reads it instead");
  const tables = tomlSubset(read("rust-toolchain.toml"), "rust-toolchain.toml");
  assert.deepEqual(tables.map((t) => t.name), ["toolchain"], "rust-toolchain.toml holds more than [toolchain]");
  const keys = tables[0].keys;
  assert.deepEqual([...keys.keys()].sort(), ["channel", "components", "profile"], "rust-toolchain.toml sets more than its channel, components and profile: a path names a toolchain no one pinned");
  assert.match(keys.get("channel"), /^"\d+\.\d+\.\d+"$/, "rust-toolchain.toml's channel is not a release");
  assert.equal(keys.get("components").replace(/\s+/g, ""), '["clippy","rustfmt"]', "rust-toolchain.toml's components changed");
  assert.equal(keys.get("profile"), '"minimal"', "rust-toolchain.toml's profile changed");
});

/// The settings clippy and rustfmt read: each looks for `clippy.toml` or `.clippy.toml`, and
/// `rustfmt.toml` or `.rustfmt.toml`, from the crate's directory up, and takes the first it finds, so
/// one beside a crate, or a dotted one at the root, would stand in for the root's (the third review
/// of 980965e: an unowned `.clippy.toml` dropped the root's ban on RefCell, and the required clippy
/// step passed). The root's clippy.toml is the one, and code-owned.
const LINT_SETTINGS = /(^|\/)\.?(clippy|rustfmt)\.toml$/;

check("no lint or format settings stand but the root's clippy.toml", () => {
  const found = [...tracked()].filter((f) => LINT_SETTINGS.test(f) && f !== "clippy.toml");
  assert.deepEqual(found, [], "a clippy or rustfmt settings file other than the root's clippy.toml, which clippy or rustfmt would read instead of it for the crates below it");
  for (const f of [".clippy.toml", "compiler/mente-ast/clippy.toml", "compiler/mente-ast/.clippy.toml", "rustfmt.toml", ".rustfmt.toml", "compiler/mente-mir/rustfmt.toml"]) assert.match(f, LINT_SETTINGS, f);
  for (const f of ["clippy.toml.md", "docs/clippy.md", "compiler/mente-ast/Cargo.toml"]) assert.doesNotMatch(f, LINT_SETTINGS, f);
});

check("no required job restores a cache, and no job caches nextest", () => {
  // Another job of the same pull request, pinned or not, can save any entry under any key in the
  // pull request's scope, and a later run restores it.
  for (const { file, id, job } of requiredJobs()) {
    for (const s of steps(job)) assert.doesNotMatch(String(s.uses ?? ""), /^actions\/cache(\/restore)?@/, `${stepKey(file, id, s)}: a required job restores a cache another job could have written`);
  }
  // setup-node restores a cache of its own once package.json names a package manager, unless told
  // not to (the third review of 980965e): every setup-node says no, and names no cache.
  for (const { file, id, job } of allJobs()) {
    for (const s of steps(job).filter((x) => /^actions\/setup-node@/.test(String(x.uses ?? "")))) {
      assert.ok(isPlain(s.with) && s.with["package-manager-cache"] === false && !("cache" in s.with) && !("cache-dependency-path" in s.with), `${stepKey(file, id, s)}: setup-node may restore a cache another job could have written; give it package-manager-cache: false and no cache`);
    }
  }
  for (const { file, id, job } of allJobs()) {
    for (const s of steps(job)) {
      if (/^actions\/cache(\/save|\/restore)?@/.test(String(s.uses ?? ""))) assert.doesNotMatch(String(s.with?.path ?? ""), /cargo-nextest/, `${stepKey(file, id, s)} caches nextest, which install-nextest.sh fetches by its digest`);
    }
  }
});

/// The variables that change how cargo, rustc, rustup, nextest, the shell, node or npm run what a
/// job checks, in any case; and those of them a required or pinned job may set, each with the one
/// value it may take (null: any) and the jobs that may set it (null: any job).
/// The dynamic loader's (LD_AUDIT, LD_PRELOAD, GLIBC_TUNABLES, GCONV_PATH), git's
/// (GIT_CONFIG_COUNT and its pairs run a command on checkout or diff), OpenSSL's (a configuration
/// loads a module), those that send rustup's download elsewhere (a proxy, with a certificate of
/// the change's) and those that move where a tool reads its settings (HOME, XDG_*, TMPDIR) belong
/// here too.
const TOOL_ENV =
  /^(CARGO_|NEXTEST_|RUSTC|RUSTDOC|RUSTUP_|RUSTFLAGS$|RUSTFMT|CLIPPY_|BASH_ENV$|BASH_FUNC_|ENV$|PATH$|SHELLOPTS$|BASHOPTS$|NODE_|NPM_CONFIG_|LD_|DYLD_|GLIBC_TUNABLES$|GCONV_|LOCPATH$|MALLOC_|GIT_|HOME$|XDG_|TMPDIR$|OPENSSL_|SSL_CERT_|CURL_CA_BUNDLE$|HTTPS?_PROXY$|ALL_PROXY$|NO_PROXY$)/i;
const TOOL_ENV_ALLOWED = new Map([
  ["CARGO_TERM_COLOR", { value: "always", jobs: null }],
  // The full run's alarm inherits the workflow's two, and runs no cargo and no nextest.
  ["CARGO_MUTANTS_VERSION", { value: null, jobs: ["mutants.yml#in-diff", "mutants-full.yml#full", "mutants-full.yml#alarm"] }],
  // The fuzzer's release build also asserts what a debug build asserts.
  ["CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", { value: "true", jobs: ["fuzz.yml#fuzz"] }],
  // The base branch's build beside this change's.
  ["CARGO_TARGET_DIR", { value: "${{ github.workspace }}/../target", jobs: ["perf.yml#kernels"] }],
  ["NEXTEST_PROFILE", { value: "mutants", jobs: ["mutants.yml#in-diff", "mutants-full.yml#full", "mutants-full.yml#alarm"] }],
  // A warning fails the build.
  ["RUSTFLAGS", { value: "-D warnings", jobs: ["ci.yml#rust"] }],
]);

/// The variables of TOOL_ENV a job sets, in the workflow, the job or a step, that TOOL_ENV_ALLOWED
/// does not hold for it.
function toolEnvProblems(key, doc, job) {
  const found = [];
  for (const env of [doc.env, job.env, ...steps(job).map((s) => s.env)].filter(Boolean)) {
    for (const [name, value] of Object.entries(env)) {
      if (!TOOL_ENV.test(name)) continue;
      const held = TOOL_ENV_ALLOWED.get(name);
      const allowed = held !== undefined && (held.value === null || held.value === String(value)) && (held.jobs === null || held.jobs.includes(key));
      if (!allowed) found.push(`${key} sets ${name}=${value}, which changes how cargo, rustc, nextest, the shell, git, node or the loader run what the job checks; TOOL_ENV_ALLOWED names the jobs that may set it, and its value`);
    }
  }
  return found;
}

check("no required or pinned job sets a variable that changes how its tools run", () => {
  const keys = new Set([...requiredJobs().map((j) => `${j.file}#${j.id}`), ...PINNED_JOBS.keys()]);
  for (const { file, doc, id, job } of allJobs().filter((j) => keys.has(`${j.file}#${j.id}`))) assert.deepEqual(toolEnvProblems(`${file}#${id}`, doc, job), []);
  for (const name of ["BASH_ENV", "npm_config_script_shell", "NODE_OPTIONS", "PATH", "Path", "cargo_target_x86_64_unknown_linux_gnu_runner", "RUSTC_WRAPPER", "RUSTUP_TOOLCHAIN"]) assert.match(name, TOOL_ENV, name);
  // The plants of the second review of 4e896ca: a variable of the Documentation job's.
  for (const name of ["CLIPPY_CONF_DIR", "RUSTFMT", "LD_AUDIT", "LD_PRELOAD", "GIT_CONFIG_COUNT", "GIT_CONFIG_KEY_0", "GIT_EXEC_PATH", "GLIBC_TUNABLES", "GCONV_PATH", "HOME", "XDG_CONFIG_HOME", "TMPDIR", "NODE_EXTRA_CA_CERTS", "BASH_FUNC_cargo%%", "OPENSSL_CONF", "SSL_CERT_FILE", "https_proxy", "ALL_PROXY"]) assert.match(name, TOOL_ENV, name);
  for (const name of ["MENTE_GEN_PROGRAMS", "GITHUB_TOKEN", "ACTIONLINT_VERSION"]) assert.doesNotMatch(name, TOOL_ENV, name);
});

/// A manifest's `build =` that names a script, the key bare or quoted, and any mention of a
/// procedural macro, `proc-macro = true` or a `crate-type` of one, comments aside. This reads the
/// listed members' text, and so sees the plain spellings at once, with no toolchain. It does not
/// hold: a key written with a TOML escape reads as another key here and as `build` to cargo, and a
/// package only a path names is no listed member. What holds is cargo's own reading, in the check
/// after this one.
const buildScriptKey = /^\s*["']?build["']?\s*=(?!\s*false\b)/m;
const procMacro = /proc[-_]macro(?![\w-])/;
/// A doc attribute in any spelling, plain, spaced or under a cfg_attr.
const docAttribute = /#!?\[\s*(doc\s*=|cfg_attr\b[^\]]*\bdoc\s*=)/;

check("no workspace member runs code at build time", () => {
  for (const [name, { dir }] of workspace()) {
    const manifest = read(`${dir}/Cargo.toml`);
    const has = [];
    if (tracked().has(`${dir}/build.rs`)) has.push("a build.rs");
    if (buildScriptKey.test(manifest)) has.push("a build script `build =` names");
    if (procMacro.test(manifest.replace(/#.*$/gm, ""))) has.push("a procedural macro");
    if (has.length > 0) assert.ok(BUILD_TIME_CODE.has(name), `${name} has ${has.join(" and ")}, code that runs at build time, before the tests' limits and with the job's rights: BUILD_TIME_CODE in .github/scripts/lib/build-time-code.mjs names the members allowed one`);
  }
  for (const m of ['build = "x.rs"', '"build" = "x.rs"', "'build' = 'x.rs'"]) assert.match(m, buildScriptKey, m);
  assert.doesNotMatch("build = false", buildScriptKey);
  for (const m of ["proc-macro = true", 'crate-type = ["proc-macro"]', "proc_macro = true"]) assert.match(m, procMacro, m);
  for (const m of ['proc-macro2 = "1"', "proc_macro2 = { workspace = true }"]) assert.doesNotMatch(m, procMacro, m);
});

check("as cargo reads the manifests and the lock, no package of the workspace runs code at build time and every other crate is approved", () => {
  // Where the reading runs, and what may come before it, is the rule of the check after this one,
  // the same for every job that carries it, the Documentation job's too.
  // What the reading refuses, on what cargo reports: the plants of the review of pull request
  // 137, each of which the reader of the manifests' text lets through.
  const member = (name, dir, kinds = ["lib"]) => ({ name, version: "0.1.0", source: null, manifest_path: path.join("/w", dir, "Cargo.toml"), targets: kinds.map((k) => ({ kind: [k] })) });
  const crate = (name, source = CRATES_IO, kinds = ["lib"]) => ({ name, version: "1.0.0", source, manifest_path: `/home/.cargo/registry/src/x/${name}-1.0.0/Cargo.toml`, targets: kinds.map((k) => ({ kind: [k] })) });
  const lockOf = (packages) => packages.map((p) => ({ name: p.name, version: p.version, source: p.source }));
  const found = (packages, listed = ["compiler/a", "compiler/b"]) => metadataProblems({ workspace_root: path.join("/w"), packages }, { listed, lock: lockOf(packages) });
  const own = [member("mente-a", "compiler/a"), member("mente-b", "compiler/b", ["bin"])];
  assert.deepEqual(found([...own, crate("serde_derive", CRATES_IO, ["proc-macro"]), crate("libc", CRATES_IO, ["lib", "custom-build"])]), [], "an approved crate's own build script or macro is refused");
  assert.match(found([member("mente-a", "compiler/a", ["lib", "custom-build"]), own[1]])[0], /mente-a \(compiler\/a\) has a build script/);
  assert.match(found([member("mente-a", "compiler/a", ["proc-macro"]), own[1]])[0], /mente-a \(compiler\/a\) has a procedural macro/);
  const unlisted = found([...own, member("helper", "compiler/b/helper", ["lib", "custom-build"])]);
  assert.equal(unlisted.length, 2, unlisted.join("\n"));
  assert.match(unlisted[0], /cargo builds helper from compiler\/b\/helper, which the members of Cargo\.toml do not list/);
  assert.match(found([...own, crate("forked", "git+https://example.invalid/x#abc")])[0], /forked comes from git\+https/);
  assert.match(found(own, ["compiler/a", "compiler/b", "compiler/c"])[0], /lists the member compiler\/c, which cargo does not read/);
  assert.match(metadataProblems({ workspace_root: path.join("/w"), packages: [...own, crate("unlocked")] }, { listed: ["compiler/a", "compiler/b"], lock: lockOf(own) })[0], /cargo would build unlocked 1\.0\.0, which Cargo\.lock does not hold/);
  // The lock, read as the lines cargo writes: a name written with an escape, a source that is not
  // crates.io and a table cargo does not write stop cargo from being run on it at all.
  const approved = approvedCrates("# c\ntop: top\nleaf: top\n");
  assert.deepEqual([...approved], ["top", "leaf"]);
  assert.throws(() => approvedCrates("top\n"), /not "<crate>: <dependency>"/);
  const lock = (extra) => `# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 4\n\n[[package]]\nname = "mente-a"\nversion = "0.1.0"\ndependencies = [\n "top",\n "leaf 2.0.0",\n "leaf 2.0.0 (registry+https://github.com/rust-lang/crates.io-index)",\n]\n\n[[package]]\nname = "top"\nversion = "1.0.0"\nsource = "${CRATES_IO}"\nchecksum = "${"a".repeat(64)}"\n${extra}`;
  assert.deepEqual(lockProblems(lock(""), approved), { unreadable: [], unapproved: [] });
  assert.equal(lockPackages(lock("")).packages.length, 2);
  const other = lockProblems(lock(`\n[[package]]\nname = "other"\nversion = "1.0.0"\nsource = "${CRATES_IO}"\n`), approved);
  assert.equal(other.unreadable.length, 0);
  assert.match(other.unapproved[0], /Cargo\.lock holds other, which \.config\/approved-crates\.txt does not list/);
  for (const [extra, why] of [
    ['\n[[package]]\nname = "le\\u0061f"\nversion = "2.0.0"\n', /not a line cargo writes/],
    ['\n[[package]]\n"name" = "leaf"\nversion = "2.0.0"\n', /not a line cargo writes/],
    ['\n[[package]]\nname = "leaf"\nversion = "2.0.0"\nsource = "git+https://example.invalid/leaf#abc"\n', /leaf comes from git\+https/],
    ['\n[[package]]\nname = "leaf"\nversion = "2.0.0"\nsource = "registry+https://example.invalid/index"\n', /not crates\.io/],
    ['\n[[patch.unused]]\nname = "leaf"\nversion = "2.0.0"\n', /not a line cargo writes/],
    ['\n[metadata]\n"checksum leaf" = "x"\n', /not a line cargo writes/],
    ['\n[[package]]\nname = "leaf"\nname = "top"\nversion = "2.0.0"\n', /not a line cargo writes/],
    ['\n[[package]]\nversion = "2.0.0"\n', /without its name/],
  ]) {
    const got = lockProblems(lock(extra), approved).unreadable;
    assert.ok(got.length > 0 && why.test(got.join("\n")), `the lock's reader lets through, or misnames: ${JSON.stringify(extra)} -> ${got.join(" | ")}`);
  }
  // The repository's own lock and members read clean, and its approved list reads as a list.
  assert.deepEqual(lockProblems(read("Cargo.lock"), approvedCrates(read(".config/approved-crates.txt"))), { unreadable: [], unapproved: [] });
  assert.deepEqual(listedMembers(read("Cargo.toml")), [...workspace().values()].map((c) => c.dir));
  assert.throws(() => listedMembers('[workspace]\nmembers = ["compiler/*"]\n'), /is a glob/);
  // The CLI itself, with a stand-in for cargo: it runs cargo only on a lock it can read, and
  // reports what cargo read.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "build-time-"));
  try {
    fs.mkdirSync(path.join(dir, ".config"));
    fs.writeFileSync(path.join(dir, ".config", "approved-crates.txt"), "top: top\n");
    fs.writeFileSync(path.join(dir, "Cargo.toml"), '[workspace]\nmembers = ["compiler/a"]\n');
    const report = (packages) => () => JSON.stringify({ workspace_root: dir, packages });
    const a = (kinds) => ({ name: "mente-a", version: "0.1.0", source: null, manifest_path: path.join(dir, "compiler", "a", "Cargo.toml"), targets: kinds.map((k) => ({ kind: [k] })) });
    // The toolchain's check runs first, in a tree of its own: a cargo home with nothing in it, and a
    // walk up that stops at this directory, so that a configuration of the machine running the
    // policy does not count.
    fs.mkdirSync(path.join(dir, "home"));
    const toolchain = { cargoHome: path.join(dir, "home"), tracked: [], top: dir };
    const run = (metadata, options = {}) => buildTimeCode(dir, metadata, { toolchain, ...options });
    fs.writeFileSync(path.join(dir, "rust-toolchain.toml"), '# c\n[toolchain]\nchannel = "1.97.1"\ncomponents = ["clippy", "rustfmt"]\nprofile = "minimal"\n');
    fs.writeFileSync(path.join(dir, "Cargo.lock"), lock(""));
    const clean = run(report([a(["lib"]), crate("top")]));
    assert.deepEqual(clean.problems, []);
    assert.match(clean.summary, /cargo reads 1 packages of the workspace.* and 1 crates of crates\.io/);
    assert.equal(run(report([a(["lib", "custom-build"]), crate("top")])).problems.length, 1);
    assert.deepEqual(run(() => assert.fail("cargo runs before the toolchain is installed"), { beforeToolchain: true }).problems, []);
    fs.writeFileSync(path.join(dir, "Cargo.lock"), lock('\n[[package]]\nname = "leaf"\nversion = "2.0.0"\nsource = "git+https://example.invalid/leaf#abc"\n'));
    const unread = run(() => assert.fail("cargo runs on a lock that names another source"));
    assert.match(unread.problems[0], /leaf comes from git\+https/);
    fs.writeFileSync(path.join(dir, "Cargo.lock"), lock(""));
    // What the toolchain's check refuses before rustup or cargo runs, cargo itself never asked.
    const valid = fs.readFileSync(path.join(dir, "rust-toolchain.toml"), "utf8");
    const refused = (why) => {
      const r = run(() => assert.fail(`cargo runs though ${why}`), {});
      assert.ok(r.problems.length > 0, `the toolchain's check lets through ${why}`);
    };
    for (const [text, why] of [
      [`${valid}path = "/tmp/toolchain"\n`, "a path toolchain"],
      [`${valid}targets = ["wasm32-unknown-unknown"]\n`, "another target"],
      [valid.replace('"1.97.1"', '"stable"'), "a channel that is no release"],
      [valid.replace('profile = "minimal"', 'profile = "default"'), "another profile"],
      [`${valid}channel = "1.97.1"\n`, "a channel twice"],
      [valid.replace("[toolchain]\n", ""), "no table"],
      [valid.replace("[toolchain]", '"toolchain"'), "a table spelled otherwise"],
    ]) {
      fs.writeFileSync(path.join(dir, "rust-toolchain.toml"), text);
      refused(why);
    }
    fs.writeFileSync(path.join(dir, "rust-toolchain.toml"), valid);
    for (const [file, why] of [
      [path.join(dir, "rust-toolchain"), "a rust-toolchain file beside rust-toolchain.toml"],
      [path.join(dir, ".cargo", "config.toml"), "a cargo configuration file in the workspace"],
      [path.join(dir, "home", "config"), "a cargo configuration file in CARGO_HOME"],
    ]) {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, "x\n");
      refused(why);
      fs.rmSync(file);
    }
    const nested = path.join(dir, "inner");
    fs.mkdirSync(path.join(dir, ".cargo"), { recursive: true });
    fs.writeFileSync(path.join(dir, ".cargo", "config"), "x\n");
    for (const f of ["rust-toolchain.toml", ".config/approved-crates.txt", "Cargo.toml", "Cargo.lock"]) {
      fs.mkdirSync(path.dirname(path.join(nested, f)), { recursive: true });
      fs.copyFileSync(path.join(dir, f), path.join(nested, f));
    }
    assert.ok(toolchainProblems(nested, { ...toolchain }).some((p) => /\.cargo[\\/]config: a cargo configuration file/.test(p)), "the toolchain's check does not walk up to a configuration above the workspace");
    fs.rmSync(path.join(dir, ".cargo"), { recursive: true });
    assert.ok(toolchainProblems(dir, { ...toolchain, tracked: ["compiler/x/.cargo/config.toml"] }).length > 0, "the toolchain's check lets through a tracked configuration deeper in the tree");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("every required or pinned job but those of GATE_ONLY and SECRET_JOBS checks, before anything of the workspace is built or installed, that nothing of the change can run where cargo builds", () => {
  // Every such job carries the check, whatever its steps run: a rule that asked which jobs build,
  // by the names of the tools in their scripts, missed a tool spelled otherwise (the third review
  // of 980965e). A job of GATE_ONLY is pinned whole instead.
  const keys = new Set([...requiredJobs().map(({ file, id }) => `${file}#${id}`), ...PINNED_JOBS.keys()]);
  let checked = 0;
  for (const { file, doc, id, job } of allJobs().filter((j) => keys.has(`${j.file}#${j.id}`))) {
    const key = `${file}#${id}`;
    const all = steps(job);
    // No step of a required or pinned job runs elsewhere than the workspace's root.
    for (const s of all) assert.ok(!("working-directory" in s), `${stepKey(file, id, s)} runs in a directory of its own`);
    if (GATE_ONLY.has(key) || SECRET_JOBS.has(key)) continue;
    checked++;
    assert.deepEqual(beforeToolchainProblems(file, id, doc, job), []);
    // After it, no step changes the toolchain, but in the base branch's own worktree, whose
    // rust-toolchain.toml is the base's (the performance jobs build the base's compiler there).
    const at = all.findIndex((s) => s.run === TOOLCHAIN_STEP);
    for (const s of all.slice(at + 1)) {
      for (const line of logicalLines(s.run ?? "").map((l) => l.trim())) {
        if (/^\(cd \.\.\/base && /.test(line)) continue;
        assert.doesNotMatch(line, /\brustup\s+(toolchain|default|override|component|target)\b/, `${stepKey(file, id, s)} changes the toolchain after the check: ${line}`);
      }
    }
  }
  assert.equal(checked, keys.size - GATE_ONLY.size - SECRET_JOBS.size, "a required or pinned job was left out of the check");
  // The plants of the second review of 4e896ca, on the Documentation job and on a Rust job: each
  // is refused by this rule, by TOOL_ENV or by the pin.
  const ci = workflows().find((w) => w.file === "ci.yml").doc;
  const clone = (v) => JSON.parse(JSON.stringify(v));
  const index = (job, test) => job.steps.findIndex(test);
  for (const id of ["docs", "rust"]) {
    const plant = (change) => {
      const doc = clone(ci);
      const job = doc.jobs[id];
      change(job, doc);
      return [...beforeToolchainProblems("ci.yml", id, doc, job), ...toolEnvProblems(`ci.yml#${id}`, doc, job), ...(jobDigest(doc, job) === PINNED_JOBS.get(`ci.yml#${id}`) ? [] : ["the pin"])];
    };
    const ruled = (change) => {
      const doc = clone(ci);
      change(doc.jobs[id], doc);
      return [...beforeToolchainProblems("ci.yml", id, doc, doc.jobs[id]), ...toolEnvProblems(`ci.yml#${id}`, doc, doc.jobs[id])];
    };
    const toolchain = (job) => job.steps[index(job, (s) => s.run === TOOLCHAIN_STEP)];
    for (const [name, change] of [
      ["P1, working-directory on the toolchain step", (job) => (toolchain(job)["working-directory"] = "compiler")],
      ["P2, a variable on the toolchain step", (job) => (toolchain(job).env = { MENTE_X: "1" })],
      ["P2, a shell on the toolchain step", (job) => (toolchain(job).shell = "sh")],
      ["P3, setup-node with an input more", (job) => job.steps.splice(1, id === "docs" ? 1 : 0, { uses: `actions/setup-node@${"0".repeat(40)}`, with: { ...NODE_INPUTS, cache: "npm" } })],
      ["P3, the checkout with a path", (job) => (job.steps[0].with = { ...job.steps[0].with, path: "elsewhere" })],
      ["P3, the checkout of another repository", (job) => (job.steps[0].with = { ...job.steps[0].with, repository: "someone/else" })],
      ["P4, a job default of a working directory", (job) => (job.defaults = { run: { shell: "bash", "working-directory": "compiler" } })],
      ["P4, a workflow default of a working directory", (job, doc) => (doc.defaults = { run: { "working-directory": "compiler" } })],
      ["P5, the checkout of main", (job) => (job.steps[0].with = { ...job.steps[0].with, ref: "main" })],
      ["P6, LD_AUDIT on the job", (job) => (job.env = { ...job.env, LD_AUDIT: "./x.so" })],
      ["P6, GIT_CONFIG_COUNT on the workflow", (job, doc) => (doc.env = { ...doc.env, GIT_CONFIG_COUNT: "1", GIT_CONFIG_KEY_0: "core.fsmonitor", GIT_CONFIG_VALUE_0: "./x" })],
      ["a step of the change's before the check", (job) => job.steps.splice(1, 0, { name: "x", run: "bash ./x.sh" })],
      ["a step allowed only in another job, before the check", (job) => job.steps.splice(1, 0, { name: "x", run: 'echo "RUSTUP_HOME=$RUNNER_TEMP/rustup" >> "$GITHUB_ENV"' })],
      ["setup-node with its own cache left on", (job) => job.steps.splice(1, id === "docs" ? 1 : 0, { uses: `actions/setup-node@${"0".repeat(40)}`, with: { "node-version": PERMISSION.node } })],
      ["npm ci before the check", (job) => job.steps.splice(1, 0, { name: "x", run: "npm ci --ignore-scripts" })],
      ["an action before the check", (job) => job.steps.splice(1, 0, { uses: `someone/action@${"0".repeat(40)}` })],
      ["a second checkout", (job) => job.steps.splice(1, 0, clone(job.steps[0]))],
      ["the toolchain step under if: false", (job) => (toolchain(job).if = "false")],
      ["the toolchain step allowed to fail", (job) => (toolchain(job)["continue-on-error"] = true)],
      ["a container", (job) => (job.container = "someone/image")],
      ["the policy step with a variable", (job) => {
        const s = job.steps.find((x) => BEFORE_TOOLCHAIN.has(String(x.run ?? "").trim()));
        if (s) s.env = { MENTE_X: "1" };
        else toolchain(job).env = { MENTE_X: "1" };
      }],
    ]) {
      assert.ok(ruled(change).length > 0, `ci.yml#${id}: the rule lets through ${name}`);
      assert.ok(plant(change).length > 0, `ci.yml#${id}: nothing refuses ${name}`);
    }
    assert.deepEqual(ruled(() => {}), [], `ci.yml#${id}: the rule refuses the job as it is`);
  }
});

/// The code blocks of a library's doc comments rustdoc lists as doctests, as `file:line`: a fenced
/// block whose info string names no other language than Rust, `ignore` included, which rustdoc
/// lists though it does not run it, and an indented block (four spaces after a blank doc line),
/// which rustdoc reads as Rust. A block another language names, `text` say, is no doctest.
export function doctestBlocks(file, text) {
  const found = [];
  let fence = null;
  let blank = true;
  const ATTRIBUTE = /^(rust|ignore(-.+)?|no_run|compile_fail|should_panic|test_harness|edition\d{4}|standalone_crate|allow_fail|E\d{4})$/;
  for (const [k, raw] of text.split(/\r?\n/).entries()) {
    const doc = /^\s*\/\/[/!](?![/!]) ?(.*)$/.exec(raw);
    if (!doc) {
      fence = null;
      blank = true;
      continue;
    }
    const body = doc[1];
    const marker = /^\s*(`{3,}|~{3,})\s*(.*)$/.exec(body);
    if (fence !== null) {
      if (marker && marker[1][0] === fence[0] && marker[1].length >= fence.length && marker[2] === "") fence = null;
      continue;
    }
    if (marker) {
      fence = marker[1];
      const tokens = marker[2].split(/[\s,]+/).filter(Boolean).map((t) => t.replace(/^\{|\}$/g, ""));
      const rust = tokens.includes("rust") || tokens.every((t) => ATTRIBUTE.test(t));
      if (rust) found.push(`${file}:${k + 1}`);
    } else if (blank && /^ {4,}\S/.test(body)) {
      found.push(`${file}:${k + 1}`);
    }
    blank = body.trim() === "";
  }
  return found;
}

check("no library's doc comments hold an example rustdoc would compile", () => {
  // nextest runs no doctests, and `cargo test --doc` has no limit per test: an example belongs in a
  // test, or in a block marked `text`.
  // The required Rust jobs list the doctests rustdoc finds, and fail on any; this reads the
  // sources first, so that an example fails here too, and names where it is.
  const found = [];
  for (const [, { dir }] of workspace()) {
    for (const f of filesUnder(dir).filter((x) => x.endsWith(".rs"))) {
      const text = read(f);
      if (!tracked().has(`${dir}/src/lib.rs`) || !f.startsWith(`${dir}/src/`) || f === `${dir}/src/main.rs` || f.startsWith(`${dir}/src/bin/`)) continue;
      // A module read from another path, which these lines would never read, is refused in every
      // crate by the check of `#[path]` above.
      found.push(...doctestBlocks(f, text));
      if (docAttribute.test(text)) found.push(`${f}: a doc attribute, whose text the policy does not read`);
      if (/^\s*\/\*[*!](?![*/])/m.test(text)) found.push(`${f}: a block doc comment, which the policy does not read`);
    }
  }
  assert.deepEqual(found, [], "doc comments hold examples rustdoc lists as doctests: mark each `text`, or make it a test");
  for (const a of ['#[doc = "x"]', '#![doc = include_str!("a.md")]', '#[ doc = "x"]', '#[cfg_attr(all(), doc = "x")]']) assert.match(a, docAttribute, a);
  assert.doesNotMatch("#[doc(hidden)]", docAttribute);
  const blocks = (text) => doctestBlocks("f.rs", text).map((at) => Number(at.split(":")[1]));
  assert.deepEqual(blocks("/// a\n/// ```\n/// x\n/// ```\n"), [2]);
  assert.deepEqual(blocks("//! ```rust,no_run\n//! x\n//! ```\n"), [1]);
  assert.deepEqual(blocks("/// ```should_panic\n/// ```\n/// ```edition2021\n/// ```\n"), [1, 3]);
  assert.deepEqual(blocks("/// ```text\n/// x\n/// ```\n/// ```mn\n/// ```\n/// ~~~sh\n/// ~~~\n"), []);
  assert.deepEqual(blocks("/// ```ignore\n/// x\n/// ```\n"), [1], "rustdoc lists an `ignore` block");
  assert.deepEqual(blocks("/// a\n///\n///     let x = 1;\n"), [3]);
  assert.deepEqual(blocks("/// - a\n///   b\n//// ```\n/// ```text\n///     x\n/// ```\n"), []);
});
