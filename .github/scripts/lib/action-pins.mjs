// Every action a workflow runs is pinned to a full commit SHA, with the release it is in a comment
// (`uses: actions/checkout@<sha> # v7.0.1`, as Dependabot writes it). The workflow policy checks
// the SHA's form; this checks that it is the commit the tag names in the action's own repository,
// so that a hand-made SHA, a commit of a fork GitHub would resolve through the fork network, or a
// SHA a pull request changed without its tag, fails the Documentation job, a required check.
//
//   node .github/scripts/action-pins.mjs
//
// It asks GitHub with `git ls-remote` for each action and tag once.

/// Each `uses:` of a pinned action in a workflow's text: its line, repository, commit and tag. A
/// pin without a tag comment has the tag null.
export function pins(file, text) {
  const out = [];
  for (const [k, line] of text.split(/\r?\n/).entries()) {
    const m = /^\s*(?:-\s+)?uses:\s*([\w.-]+\/[\w.-]+)(?:\/[\w./-]+)?@([0-9a-f]{40})(?:\s+#\s*(\S+))?\s*$/.exec(line);
    if (m) out.push({ at: `${file}:${k + 1}`, repo: m[1], sha: m[2], tag: m[3] ?? null });
  }
  return out;
}

/// The commit a tag names, from `git ls-remote <repo> refs/tags/<tag> refs/tags/<tag>^{}`: an
/// annotated tag's peeled commit, or a lightweight tag's own; null when the tag is not there.
export function taggedCommit(lsRemote, tag) {
  const refs = new Map(
    String(lsRemote)
      .split(/\r?\n/)
      .map((l) => l.split("\t"))
      .filter((p) => p.length === 2)
      .map(([sha, ref]) => [ref, sha]),
  );
  return refs.get(`refs/tags/${tag}^{}`) ?? refs.get(`refs/tags/${tag}`) ?? null;
}

/// Checks every workflow of `dir`: prints what is wrong and sets the exit code, or says how many
/// pins it checked.
export function main(dir, list, read, lsRemote) {
  const all = list(dir)
    .filter((f) => f.endsWith(".yml") || f.endsWith(".yaml"))
    .sort()
    .flatMap((f) => pins(`${dir}/${f}`, read(`${dir}/${f}`)));
  const found = problems(all, lsRemote);
  for (const p of found) console.log(`::error::${p}`);
  if (found.length > 0) process.exitCode = 1;
  else console.log(`${all.length} pinned actions, each the commit its release's tag names.`);
}

/// What is wrong with the pins, one message each: a pin without its tag, or a commit other than
/// the one its tag names. `lsRemote(repo, tag)` returns git ls-remote's output.
export function problems(allPins, lsRemote) {
  const found = [];
  const asked = new Map();
  for (const p of allPins) {
    if (p.tag === null) {
      found.push(`${p.at}: ${p.repo}@${p.sha} names no release in a comment`);
      continue;
    }
    const key = `${p.repo} ${p.tag}`;
    if (!asked.has(key)) asked.set(key, taggedCommit(lsRemote(p.repo, p.tag), p.tag));
    const commit = asked.get(key);
    if (commit === null) found.push(`${p.at}: ${p.repo} has no tag ${p.tag}`);
    else if (commit !== p.sha) found.push(`${p.at}: ${p.repo}@${p.sha} is not ${p.tag}, which is ${commit}`);
  }
  return found;
}
