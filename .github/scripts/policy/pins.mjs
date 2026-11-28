// What the workflow policy pins, and whose the gate files are (README.md in this directory): the
// scripts and jobs that run the tests, by their digests, and the files CODEOWNERS gives the owner.

import { digest, read } from "./repo.mjs";
import { triggers } from "./workflows.mjs";

/// The policy's own files, listed once in modules.json beside them, which PINNED_FILES pins: the
/// test modules node runs, the libraries they share, and the checks they hold in all.
export const POLICY_LIST_FILE = ".github/scripts/policy/modules.json";
export const POLICY_LIST = JSON.parse(read(POLICY_LIST_FILE));
/// The policy's own scripts, exactly those listed, which cannot hold their own digests: CODEOWNERS
/// gives them to their owner, and every other script a pinned job runs is pinned, wherever it lies
/// (the review of f217d1e: a directory prefix exempted any script under it).
export const POLICY_FILES = new Set([...POLICY_LIST.modules, ...POLICY_LIST.libraries]);
export const isPolicy = (f) => POLICY_FILES.has(f);
/// A job as its pin reads it: the workflow's env and defaults with the whole job, keys in order,
/// each action named without the commit its pin names, which Dependabot moves and the check of
/// pins holds.
export function jobDigest(doc, job) {
  const canon = (v) =>
    Array.isArray(v)
      ? v.map(canon)
      : v !== null && typeof v === "object"
        ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, k === "uses" ? String(v[k]).replace(/@[0-9a-f]{40}$/, "@pinned") : canon(v[k])]))
        : v;
  // The workflow's triggers and its token's permissions too: what starts a pinned job, and with what
  // rights, is part of it (the review of 41a54b6: an `issue_comment:` trigger and `contents: write`
  // added to probe.yml left every pin as it was).
  return digest(JSON.stringify(canon({ on: triggers(doc), permissions: doc.permissions ?? null, env: doc.env ?? null, defaults: doc.defaults ?? null, job })));
}

/// What the tests run under, pinned whole: the scripts the testing jobs run, by their digests, and
/// the testing jobs themselves, every step's name, action, inputs, environment and script. A
/// change to any of them changes this file too, where review sees that a gate changed; the
/// failure names the digest to write here.
export const PINNED_FILES = new Map([
  // The policy's list of its own files: a module dropped from it, or the total of checks lowered,
  // changes this digest too.
  [".github/scripts/policy/modules.json", "97cd476f4185e30fbfffa9dfc6277436ad8776172007475c36bdaa73d468462b"],
  [".github/scripts/limits.sh", "40747e4a7344895ce42dd902adb28dd669809d4dd6821d3c1e01921552669ed0"],
  [".github/scripts/memory-guard.sh", "c3c629010f58d39682fa06353305181cf7d1064d2b07abefd6885033045630d1"],
  [".github/scripts/job-limits.ps1", "6222b38ac2c9c5598a986c345afde016d55ae70e68138ed975f28ab15cdefc4e"],
  [".github/scripts/lib/job-object.cs", "aa78b142be7db5de8cf53593b6a5eaf9c62f378a85d4584ec341476345bec908"],
  [".github/scripts/install-nextest.sh", "0ab5aa5d6feb4cad3d93e9ff5cbffcc27ec4a8b2ef37e47657bc85884bda9dab"],
  [".github/scripts/publish-tag.sh", "d2c7229991d14351b3f1a061af77caacacc04edb3083b355f4a135969a23cbd6"],
  [".github/scripts/commit-on-main.sh", "ff3e3901d0f81fd54bbee91bf170454a082a4ef868d9e8fb490a98f5ef0da0f8"],
  [".github/scripts/fuzz-summary.mjs", "5f434a6b1c82003c27e5b531609d2877e0e8c61628bd49261f0f045df4616fea"],
  [".github/scripts/nightly-alarm.mjs", "4d6a389e64a183ea0935fee1060d02a02a68e3dd40f2b7b6bd5c0daa47e1da59"],
  [".github/scripts/lib/nightly-alarm.mjs", "6ec77c6bcf004da4da9896d70839f1b843e7c0ba44f324c18231dc697741a5df"],
  [".github/scripts/install-mdbook.sh", "1bbc5b95423f7c849ab9e2e036108fc198120177beee4bf6d8d239bb7832eef6"],
  [".github/scripts/test-list.mjs", "6aa8a0c532adf0a5ccef1e65b9a4a0f05090929db3be40d667d9ebefc828df09"],
  [".github/scripts/lib/test-list.mjs", "9aab6d21844be4b964bcfa9e19257c4b2c61ce3cf40f03a0319b1a759bc05b6b"],
  [".github/scripts/lib/test-times.mjs", "3ca15a31f747f5125186af1648202803a2563f347a326a13c06642cc5a9bc372"],
  [".github/scripts/test-times.mjs", "397e521bd3c3932429a6c16b5634c0de99ec9b2cf3203eb148ee51c462a392e9"],
  [".github/scripts/tools.sh", "dee23b6f996a8b6772446056aa448ad2a35e60563a3e0320e1855b6a420e90d0"],
  [".github/scripts/test-sandbox.sh", "9ac640fe36800bc80902151031552ff8f218c104f03c85a3a79951df880a7bbd"],
  // The sandbox's one program, cargo's runner and the namespace's first process, which
  // test-sandbox.sh builds at setup and every command runs through.
  [".github/scripts/test-sandbox.rs", "8ce1084352c530f615812a052dcbf527f92dafc098d1310e8b64888a42e19b80"],
  [".github/scripts/test-sandbox-env.sh", "f78020328d4fed7700655d3cf5825d4cf9372c9931f168b39114cf3dd5f10d8a"],
  [".github/scripts/overflow-proof.sh", "b27d5bf8296337f43d188e5676cef4dde4586c2344882b6aee6b9e9ea0c66ece"],
  // The policy's collector, its run tests and the fixtures they run instead of a text handed to bash
  // (policy-runtime-judge): run by the Documentation job, pinned like every script it runs.
  [".github/scripts/policy-collect.mjs", "8594d53e73f3b014fff01ee9cc1014fa5575a9b3ee9b102fae33c03754b0f6af"],
  [".github/scripts/policy-run.test.mjs", "0e12e92f47af98c5bea51930569b7fd9dbb78455bb66fff14733ae3b861cdd16"],
  [".github/scripts/fixtures/limits-flags.sh", "6ece2ca2338651617b501bea5c40bf666e497f49d962881c426e6db081c44e09"],
  [".github/scripts/fixtures/memory-guard-rounds.sh", "29cf23124f683670ac11dd3c5cf0bea04f670f077734fa51ff0c4b83aa62b364"],
  [".github/scripts/fixtures/sandbox-env.sh", "e600a29b7080e262ab08f4b7d0f552c193b035bc4db737c950a737d526540c0e"],
  [".github/scripts/fixtures/source-limits.sh", "46892d8cfb2903335e9c6fb689159bba7820e2116a3823161c0e67aa33407988"],
  [".github/scripts/fixtures/timeout.sh", "0697de49d46e74f75710b105bb997630d8e3d0c2c8bd7dfdaca7775c63a5affc"],
  // The node scripts the pinned jobs run, and the modules they import: the build-time check, the
  // Mutants floor, the watch, the actions' pins and the YAML reader the policy reads every
  // workflow with.
  [".github/scripts/build-time-code.mjs", "874280f8818d786a0c6f0bca3a60ced4ae397c0f13169a1244303d4a436487ec"],
  [".github/scripts/lib/build-time-code.mjs", "99b866cb2014b1a97c770819e4707fa06ef203da6bb7e2efc7c6cc3b65517b6a"],
  [".github/scripts/mutants-floor.mjs", "4f1dfe8d84f6356750a2dbeb1af44b99a892f31a837be031f1df70a7783e01f7"],
  [".github/scripts/lib/mutants-floor.mjs", "a72471818940af8684ce20935ebf5b8bae4c59e4c324e602ce93cfd7e6b42628"],
  [".github/scripts/watch.mjs", "55168acde1c9d16a823db62389483640e24e745dfd39ad6e984cf88c3200a7d3"],
  [".github/scripts/lib/watch.mjs", "895bedf2e57eb8dd3fc327a33d34e2adedf2770df18de599fae304a6819fe502"],
  [".github/scripts/action-pins.mjs", "83f1208532ba0bc01bc19a0546fd49b93520d464c22a3dc3a41f65cfce4db458"],
  [".github/scripts/lib/action-pins.mjs", "f08a9cec14bea0adb5a077a2dc33a52b668868bcd6cc15de2691c57ad8e837dc"],
  [".github/scripts/yaml.test.mjs", "4dc4626a1c6ecb74f5c82f82d3994e6d477d30a7ac63b5e25b2b702665f9eab3"],
  [".github/scripts/lib/yaml.mjs", "4c06080bc480b996513b6410b0290e1bc74c569a5a898968af3e8b27cbd88e30"],
  [".github/scripts/lib/commits.mjs", "0ac75bf9db3625fae54ddcb75fc1e31b68fb61389bd7747b56d637a9edb276a3"],
  [".github/scripts/commits.mjs", "33c4156781145f9c0a0e127fb99c304a85e1ed508e1321c4e56a3992ce6d4322"],
  // The body CLI, which the run tests spawn (the review of 5264a01).
  [".github/scripts/commits-body.mjs", "14b23c30b836a333ccc2664fbb4f7ac8063496722ff3f97b3f26f00f3c8bf497"],
  // Every file on the API key's path and on the lock's, which run beside the key or as root: a line
  // added anywhere in one, before the key's line or after it, is a change to a gate review sees
  // (the re-check of 4ab15da: `set -x`, or the key written to a file, passed every rule of
  // start-broker.sh's shape).
  ["benchmark/harness/start-broker.sh", "9d3255c058d22c79fb2816413eb066d761ccd022a13eebe96c78f1946055853b"],
  ["benchmark/harness/stop-broker.sh", "1f4ec139da512a920c4eeafad893e451c0629c93ea19bd740ba4b6fc5b092ffa"],
  ["benchmark/harness/lock-sudo.sh", "c6562d1115b5561722fc3657860b057cdbf75190c491b46e19b1cf4a6d132cdc"],
  ["benchmark/harness/sandbox-root.sh", "2298f2a5e113cd18c796a1359c54ffb0cd755b783ae0d056d662e06eaf24fbc6"],
  ["benchmark/harness/fetch-held-out.sh", "c16dff5677b17c9d6e9090cf35f93dc9f58782a182b2e417f1d23482fdce9792"],
  // The broker's own module, which start-broker.sh names through the workspace's path.
  ["benchmark/harness/broker.ts", "20ad249db858d152ef201a89e96ba41035d619918764d2b8a6a8b2aa6db8df7e"],
  // Every file a secret job runs before its secret steps, with every module those import: none of
  // them can leave the secret steps an environment, a program first on PATH or a library to preload
  // without an edit review sees (secretPrefixFiles; the re-check of c55cf8f). And the pins of
  // toolchains.json, which choose what install-toolchains.sh installs and runs outside the sandbox.
  ["benchmark/harness/install-toolchains.sh", "db0719972981e0e212a3052c4439239852a75da432a9556f5c1200d4e57ac9f7"],
  ["benchmark/harness/toolchains.json", "cd656ee22820f70bb218457f5941066bde4d3ae758355251830646bcaf4034e8"],
  ["benchmark/harness/sandbox.sh", "e07589a6e45ed458002d77165e37866ac9987098ae705f5a9405ca672317c218"],
  ["benchmark/harness/sandbox-check.ts", "982132df137c367d307fdc3a7d45761b45bd9a214860cf63262f5752d9114a54"],
  ["benchmark/harness/build.ts", "b7cbef2b51fc6c7574dedc96435c2b26e74ced96c20ad0ed189ccb285ba5cf54"],
  ["benchmark/harness/languages.ts", "c20a2cc3728849d6262d207194cc64a28efed6be7421a71da191602e8d668200"],
  ["benchmark/harness/tasks.ts", "b9386ec2e03484721e3e05feb0b86c1ba35710c883f418473e188fe7045b2240"],
  ["benchmark/own.mjs", "ec189cfb8e26ba2394424f1ed7922323786abde7c85c09b0f98eefca08f49330"],
  ["benchmark/check.mjs", "36868660c86c3cea1d663ad9b64b374a5f29581279e21ca452f5e2d5e6f20303"],
]);
/// The jobs pinned whole: the sandboxed ones, and the Documentation job, which runs the policy and
/// the build-time check before anything else of the change, so that a step put before them, an
/// input of its checkout, or a variable of the job is a change to a gate review sees.
export const PINNED_JOBS = new Map([
  ["ci.yml#rust", "3105565a83bb93bb4fbee08ec79b936931500637dc51afce73ddb51cc5dec303"],
  ["mutants.yml#plan", "a13a733683622b5149435a55f9f7b5f334383df878d1006b0a4093ee06a5b3bc"],
  ["mutants.yml#shard", "17c2d55473d78fad9ca5fa76de14a74417f0ce17b1ca7c534a57f69aba63c885"],
  ["mutants.yml#aggregate", "579f212be0eb40ba5b554285629ca1d61b5cc8f78d8e5aa56db6f8e26fa73ac4"],
  ["mutants-full.yml#build", "2c983a54519f8d1007516cc85819127a66bba1008c017d70a6fbf9bb5a4d437a"],
  ["mutants-full.yml#full", "e1d21246b9c6b8e0bb849381d7f7a56fe717718ec3a0245f2fb13fb707aac996"],
  ["ci.yml#execution-paths", "ffb9b1d7eb4ab57c72f6cb75d3ab6c181f52c749b3a9e6e078ab1475405099cb"],
  ["generated.yml#paths", "e5da13151fa0b007ca0e8f1f226cc44edff46c76b7800d0e77951aae7d4050a4"],
  ["ci.yml#docs", "9795712883409606684a0f16d9234792a40a76376018d594dc5596673dcae5ed"],
  ["commits.yml#commits", "c4e624153a4a97ce9e979f1d6bd3e8df4312ef3c2832b3c1ef64e7739ac98cd7"],
  ["benchmark.yml#bench", "5fe155416addc5eae5e51174842ae3bdf8e69ffdd1879ba691c482b60a03c971"],
  ["prelim.yml#prelim", "ac6eadf49ad0da4d645891fc15d16450c3dbd0905926e5c9c92b548ed7557bc7"],
  ["probe.yml#write", "0830c255940ceb132a7f00e0426efe6de5c4d1287edb351e8cffd93311c51b34"],
  // The site's publication: the build of a milestone tag, and its deployment, which proves the tag
  // on main first (the review of 7ba7a5c).
  ["pages-publish.yml#build", "576f730be30b3daf87b6eb04b0675d7defc124d744250f0578f5ec7a66445593"],
  ["pages-publish.yml#deploy", "b0a09fe1825e5389c6fe19cd0cdeda6da5b086b8937b345588c037fa19b3ce2b"],
  // The nightly alarms, each with its workflow's triggers (Norman, 2026-11-21).
  ["fuzz-nightly.yml#alarm", "3873cebb627ff350058b72ed6a16297ebd92eff8155306ccc271157ce359d652"],
  ["generated-nightly.yml#alarm", "362bd08920cd3c3830502a91997feda6e45cb81604fcdf9bbe596e16e8ceb946"],
  ["mutants-full.yml#alarm", "61563727026e3b1ee149e9720963b3dd6336e8a19642c64cccf2437f9b93b598"],
]);
export const GATE_FILES = [
  "/.github/",
  "/.cargo/",
  "/Cargo.toml",
  "/Cargo.lock",
  "/rust-toolchain.toml",
  "/rust-toolchain",
  "/.config/",
  "/clippy.toml",
  "/CLAUDE.md",
  "/spec/philosophy.md",
  "/package.json",
  "/package-lock.json",
  "/.spec-guard.json",
  "/.spec-graph.json",
  "/.spec-brief.json",
  "/perf/run.mjs",
  "/perf/lib.mjs",
  "/benchmark/check.mjs",
  "/benchmark/corpus.lock",
  "/benchmark/reference.mjs",
  "/benchmark/own.mjs",
  "/benchmark/harness/",
  "/compiler/mente-cli/tests/gates.rs",
  "/compiler/mente-cli/tests/support/",
  "/compiler/mente-cli/tests/lints.rs",
  "/compiler/mente-cli/tests/register.rs",
  "/compiler/mente-cli/tests/records.rs",
  "/compiler/mente-cli/tests/escapes.rs",
  "/compiler/mente-cli/src/records.rs",
  "/compiler/mente-cli/src/escapes.rs",
  "/compiler/mente-cli/src/evidence.rs",
  "/compiler/mente-cli/src/json.rs",
  "/compiler/mente-cli/tests/evidence.rs",
  "/tools/evidence-reporter.mjs",
  "/tools/evidence-reporter.test.mjs",
  "/tools/evidence-reporter.jsonl",
  "/compiler/mente-cli/src/front-matter/",
  "/compiler/*/src/overflow_traps.rs",
  "/compiler/mente-threads/",
  "/compiler/*/Cargo.toml",
  "/spikes/*/Cargo.toml",
  "/.gitattributes",
  "/tools/quick.mjs",
  "/tools/quick-checks.json",
  "/tools/quick.test.mjs",
  "/tools/push.mjs",
  "/tools/push-guard.mjs",
  "/tools/pr-body-guard.mjs",
  "/tools/push.test.mjs",
  "/tools/git-hooks/",
  "/tools/local-limits.mjs",
  "/tools/local-limits.test.mjs",
  "/tools/capped.ps1",
  "/tools/capped.mjs",
  "/tools/cargo.mjs",
  "/tools/mn.mjs",
  "/tools/raw-build.mjs",
  "/tools/raw-build-guard.mjs",
  "/tools/store-apps.mjs",
  "/.claude/settings.json",
];

/// Whether a CODEOWNERS pattern of the forms GATE_FILES uses owns a file: `/dir/` every file under
/// it, `/file` that file, and `*` any name within one directory.
export const owns = (pattern, file) => {
  const body = pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, "[^/]*");
  return new RegExp(`^${body}${pattern.endsWith("/") ? ".*" : ""}$`).test(`/${file}`);
};

/// The map of the quick checks before a push (tools/quick.mjs), which this policy reads as data.
export const QUICK_MAP = "tools/quick-checks.json";
