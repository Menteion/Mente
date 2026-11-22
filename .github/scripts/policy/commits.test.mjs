// The workflow policy: the Commits check (README.md in this directory).

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { CJK_RANGES, DECLARED_BY_AUTHOR, badSubject, baseChangedSincePush, bodyMain, cjkLines, declarationProblems, declaringAuthor, problems, proseLines, pullRequestOf, requestProblems } from "../lib/commits.mjs";
import { read } from "./repo.mjs";

/// Every check of this module counts itself when it runs, and the module fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// each module's FLOOR equal to the checks it declares.
const FLOOR = 5;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks of commits.test.mjs ran: a skip or a filter left the rest out`);
});
// ------------------------------------------------------------------------------ the Commits job

check("the Commits check refuses unfinished subjects, CJK text and a changed base", () => {
  for (const s of ["WIP: parse enums", "wip", "Wip parse", "[WIP] parse", "(wip) x", "Draft: enums", "[Draft] enums", "fixup! Parse", "fixup parse", "Fixup: parse", "squash! x", "squash: x", "[squash] x", "squash", "amend! y"]) {
    assert.ok(badSubject(s), s);
  }
  for (const s of ["Parse enums", "Wipe the cache before a run", "Fix the fixup! handling", "Draft the M3 plan", "Squash two passes into one", "Amend the claims", "Merge pull request #1 from a/b"]) {
    assert.equal(badSubject(s), null, s);
  }
  const cjk = (...codes) => String.fromCodePoint(...codes);
  assert.deepEqual(cjkLines(`English\n${cjk(0x4e2d, 0x6587)}\nmore\n${cjk(0x3002)}`), [2, 4]);
  for (const c of [0x1100, 0x2e80, 0x3105, 0x3131, 0x31a0, 0x3200, 0x33ff, 0xac00, 0xff01, 0x20000, 0x3ffff]) {
    assert.deepEqual(cjkLines(`x ${cjk(c)}`), [1], c.toString(16));
  }
  assert.deepEqual(cjkLines(`caf${cjk(0xe9)}, ${cjk(0x2014)} ${cjk(0x1f916)}`), []);
  const found = problems(
    [
      { sha: "1234567890", subject: "WIP: x", message: "WIP: x\n" },
      { sha: "abcdef0123", subject: "Fine", message: `Fine\n\n${cjk(0xac00)}\n` },
      { sha: "0000000000", subject: "Fine too", message: "Fine too\n" },
    ],
    { title: "A title", body: `Not a fix: a test of the check.\n${cjk(0xff01)}` },
  );
  assert.equal(found.length, 3);
  assert.match(found[0], /1234567 "WIP: x".*"WIP"/);
  assert.match(found[1], /abcdef0.*line 3/);
  assert.match(found[2], /body.*line 2/);
  assert.deepEqual(problems([], null), []);
  assert.deepEqual(problems([], null, true), ["base changed: push a commit to re-run the instruments"]);
  // A subject reaches the log quoted, so it cannot open a workflow command of its own.
  assert.match(problems([{ sha: "1234567", subject: "wip\n::error::x", message: "" }], null)[0], /"wip\\n::error::x"/);
});

check("the base-change rule reads every source and the head's first run", () => {
  const at = (h) => ({ created_at: `2026-11-12T${h}:00:00Z` });
  const change = (h) => ({ event: "base_ref_changed", ...at(h) });
  assert.equal(baseChangedSincePush([], [at("10")]), false);
  assert.equal(baseChangedSincePush([change("09"), { event: "labeled", ...at("11") }], [at("10")]), false);
  assert.equal(baseChangedSincePush([change("09"), change("11")], [at("12"), at("10")]), true, "the first run is the push");
  assert.throws(() => baseChangedSincePush([change("09")], []), /push time is unknown/);
  // A merge group's branch names the pull request and the group's base, never the head.
  assert.deepEqual(pullRequestOf("merge_group", { merge_group: { head_ref: `refs/heads/gh-readonly-queue/main/pr-124-${"a".repeat(40)}` } }), { number: 124, head: null });
  assert.deepEqual(pullRequestOf("pull_request", { pull_request: { number: 9, head: { sha: "h" } } }), { number: 9, head: "h" });
  assert.throws(() => pullRequestOf("merge_group", { merge_group: { head_ref: "refs/heads/x" } }), /names no pull request/);
});

check("the Commits check rejects the code points test_repository_text_is_english rejects", () => {
  const main = read("compiler/mente-cli/tests/gates.rs");
  const body = /fn is_cjk\(c: char\) -> bool \{([\s\S]*?)\n\s*\}/.exec(main)?.[1];
  assert.ok(body, "is_cjk not found in compiler/mente-cli/tests/gates.rs");
  const ranges = [...body.matchAll(/0x([0-9A-Fa-f_]+)\.\.=0x([0-9A-Fa-f_]+)/g)].map((m) => [parseInt(m[1].replace(/_/g, ""), 16), parseInt(m[2].replace(/_/g, ""), 16)]);
  assert.deepEqual(CJK_RANGES, ranges);
});

check("every pull request declares the root cause of what it fixes and whether the fix removes it, or that it is no fix, in one canonical form", () => {
  // root-cause-first (Norman, 2026-11-22): the question is asked of every pull request before its
  // first review, not after three rounds of findings.
  const ok = (body, why) => assert.deepEqual(declarationProblems(body, "someone"), [], why);
  const refused = (body, message, why) => {
    const found = declarationProblems(body, "someone");
    assert.ok(found.some((p) => message.test(p)), `${why} passes, or fails otherwise: ${JSON.stringify(found)}`);
  };
  ok("Root cause: the reading forgave a line feed.\nRemoved at the root: the CLI prints nothing for nothing.", "a fix whose root is removed");
  ok("Intro.\n\nRoot cause: a tool's release lags its fix.\nMore on it.\nNot removable at the root: upstream ships it.\nGuard: policy check \"x\"", "a fix whose root stays, with its guard");
  ok("Not a fix: a feature, the plan of Mutants.", "no fix");
  ok("```text\nRoot cause: quoted in a fence\nNot a fix: quoted\n```\nNot a fix: a brief.", "the labels quoted in a code fence");
  ok("<!-- Root cause: a template's example -->\nNot a fix: instruments.", "the labels in a comment");
  ok("<!--\nRoot cause: a template's example\nRemoved at the root: on lines of their own\n-->\nNot a fix: instruments.", "the labels on lines of a comment");
  ok("Not a fix: instruments.\n<!-- an unclosed comment hides the rest\nRoot cause: x", "the labels after an unclosed comment");
  ok("Root cause: x\r\nRemoved at the root: y\r\n", "lines ended by a carriage return");
  // The review of 5264a01: code blocks are read before comments, and a comment starts only at the
  // start of a line, as GitHub renders them.
  ok("Root cause: x\rRemoved at the root: y", "lines ended by a carriage return alone");
  ok("A note on `<!--` in inline code.\nNot a fix: instruments.", "an inline-code `<!--`, which hides nothing after it");
  refused("A note on `<!--` in inline code.\n```\n-->\nNot a fix: quoted in a fence\n```", /declares neither/, "a comment opened in inline code and closed in a fence, which shows the fence");
  ok("````\n```\nRoot cause: quoted\n```\n````\nNot a fix: z", "a four-backtick fence holding three-backtick lines and a label");
  ok("~~~\nRoot cause: quoted\n```\nRemoved at the root: quoted\n~~~\nNot a fix: z", "a tilde fence, which backticks do not close");
  ok("```\n``` x\nRoot cause: quoted\n```\nNot a fix: z", "a line of backticks with more after them, which closes no fence");
  refused("``` `inline` ```\nRoot cause: x", /followed by neither/, "backticks with a backtick after them, inline code and no fence");
  ok("Not a fix: z\n\n    Root cause: quoted\n    ## Root cause\n\tGuard: quoted", "the forms quoted in an indented code block");
  ok("> ```\n> Root cause: quoted\n> ```\nNot a fix: z", "the forms quoted in a fence in a block quote");
  ok("> ```\n> quoted\nRoot cause: x\nRemoved at the root: y", "a fence in a block quote, which ends with the quote");
  ok("1. An item.\n\n    ```\n   Root cause: quoted\n    ```\nNot a fix: z", "a fence four columns in, in a list item, with a line of its item's left of the fence");
  ok("    ```\nNot a fix: z", "a fence four columns in, which no line at the left edge stands inside");
  refused("```\n        ```\nRoot cause: quoted\n```", /declares neither/, "a closing fence more than three columns in, which closes nothing");
  ok("<pre>\nRoot cause: quoted\nNot a fix: quoted\n</pre>\nNot a fix: z", "the forms quoted in <pre>");
  ok("<PRE class=\"x\">Root cause: quoted</pre>\nNot a fix: z", "a <pre> closed on its own line");
  ok("> <!--\n> a quoted comment\nRoot cause: x\nRemoved at the root: y\n-->", "a comment in a block quote, which ends with the quote");
  ok("Not a fix: a change whose\nroot cause\nlies elsewhere, held by a\nguard\nof its own.", "a hard-wrapped line that is the words alone");
  // Planted, each refused with its own message (the Manager, 2026-11-22).
  refused("", /declares neither/, "an empty body");
  refused("A summary, and no declaration.", /declares neither/, "no declaration");
  refused("Root cause: x\nRemoved at the root: y\nNot a fix: z", /declares both a fix/, "both declarations");
  refused("Root cause:\nRemoved at the root: y", /line 1: `Root cause:` holds no text/, "an empty root cause");
  refused("Root cause:    \nRemoved at the root: y", /holds no text/, "a root cause of spaces");
  refused("Root cause: x\nRemoved at the root:", /line 2: `Removed at the root:` holds no text/, "an empty removal");
  refused("Not a fix:", /holds no text/, "an empty no-fix");
  refused("Root cause: x", /followed by neither/, "a root cause with no follow-up line");
  refused("Root cause: x\nRemoved at the root: y\nNot removable at the root: z\nGuard: g", /says 2 times how the fix meets the root/, "two follow-up lines");
  refused("Root cause: x\nNot removable at the root: y", /names no `Guard:`/, "a root that stays with no guard");
  refused("Root cause: x\nRemoved at the root: y\nGuard: g", /`Guard:` .* with no `Not removable at the root:`/, "a guard beside a removal");
  refused("Removed at the root: y\nRoot cause: x", /stands before its `Root cause:`/, "the follow-up before the root cause");
  refused("Removed at the root: y", /with no `Root cause:` before it/, "a follow-up with no root cause");
  refused("Root cause: x\nRoot cause: z\nRemoved at the root: y", /`Root cause:` 2 times/, "two root causes");
  refused("Not a fix: a.\nNot a fix: b.", /`Not a fix:` 2 times/, "two no-fixes");
  refused("Root cause: x\nGuard: g\nNot removable at the root: y", /`Guard:` \(line 2\) stands before/, "the guard before its reason");
  for (const [near, why] of [
    ["root cause: x", "the label in lower case"],
    ["Root Cause: x", "the label in title case"],
    ["ROOT CAUSE: x", "the label in capitals"],
    ["Root-cause: x", "the label spelled with a hyphen"],
    ["Rootcause: x", "the label spelled as one word"],
    ["**Root cause:** x", "the label in bold"],
    ["## Root cause", "the label as a heading"],
    ["- Root cause: x", "the label in a list"],
    ["> Root cause: x", "the label quoted"],
    [" Root cause: x", "the label after a space"],
    ["Not a Fix: x", "no-fix in title case"],
    ["not a fix: x", "no-fix in lower case"],
    ["Removed At The Root: y", "a follow-up in title case"],
    ["guard: g", "the guard in lower case"],
  ]) {
    refused(`${near}\nNot a fix: z`, /reads like a label of the declaration in another case, spelling or form/, why);
  }
  // A line that only begins with the same words is prose.
  ok("Root causes vary.\nNot a fix: a refactor.", "prose that begins with the words");
  ok("Guards keep the class.\nNot a fix: a refactor.", "prose that begins with Guards");
  // No text of the body reaches a message: only its line numbers and the labels.
  for (const p of declarationProblems("::warning::planted\nroot cause: ::error::x", "someone")) assert.doesNotMatch(p, /planted|::error::x/, p);
  // The authors the policy declares by what they are, and no other: the author is the event's.
  assert.deepEqual([...DECLARED_BY_AUTHOR], [["dependabot[bot]", "Not a fix: a dependency update"]]);
  assert.deepEqual(declarationProblems("Bumps a crate.", "dependabot[bot]"), []);
  for (const author of ["dependabot", "menteion-agent[bot]", "Dependabot[bot]", "", undefined]) assert.ok(declarationProblems("Bumps a crate.", author).length > 0, `${author} passes without a declaration`);
  // It declares only while every commit is its own: a commit a session pushes onto its branch is a
  // change it did not write (the review of 5264a01).
  const bot = "dependabot[bot]";
  assert.deepEqual(declaringAuthor(bot, [{ sha: "a1", login: bot }, { sha: "b2", login: bot }]), { author: bot, others: [] });
  assert.deepEqual(declaringAuthor(bot, [{ sha: "a1", login: bot }, { sha: "b2", login: "someone" }, { sha: "c3", login: "" }]), { author: "", others: ["b2", "c3"] });
  assert.deepEqual(declaringAuthor(bot, []), { author: "", others: [] }, "a pull request with no commits declares itself");
  assert.deepEqual(declaringAuthor("someone", [{ sha: "a1", login: "someone" }]), { author: "", others: [] }, "an author the policy does not name declares");
  // The near-miss says what the form asks, so that a heading turned into a label is not refused again.
  assert.match(declarationProblems("## Root cause\nNot a fix: z", "someone")[0], /with its text on the same line; a heading is not a label/);
  // The check reports the body's problems with the rest, and the line numbers of the body.
  assert.ok(problems([], { title: "T", body: "No declaration.", author: "someone" }).some((p) => p.startsWith("the pull request's body: it declares neither")));
  assert.deepEqual(problems([], { title: "T", body: "Bumps.", author: "dependabot[bot]" }), []);
  assert.deepEqual(proseLines("a\n```\nb\n```\nc").map((l) => l.n), [1, 5]);
  assert.deepEqual(proseLines("a\n<!--\nb\n-->\nc").map((l) => [l.n, l.line.trim()]).filter(([, l]) => l !== ""), [[1, "a"], [5, "c"]]);
});

check("a request that creates or edits a pull request, and a draft, are read as the Commits check reads a pull request's text", () => {
  // The session's hook reads the JSON a session sends (tools/pr-body-guard.mjs), and the body CLI
  // reads a draft or that JSON (the review of 5264a01: the text checked is the text sent).
  const han = String.fromCodePoint(0x4e2d, 0x6587);
  const declared = "Not a fix: a brief.";
  // A request that creates one is read whole: a missing body is an empty one.
  assert.deepEqual(requestProblems({ title: "T", body: declared, head: "h", base: "main" }, { creates: true }), []);
  assert.match(requestProblems({ title: "T" }, { creates: true }).join("\n"), /body: it declares neither/, "a pull request created with no body passes");
  assert.match(requestProblems({ title: `T ${han}`, body: declared }, { creates: true }).join("\n"), /title: CJK text/);
  // One that edits is read for what it sends: the title alone is no edit of the body; a body sent as
  // null clears it.
  assert.deepEqual(requestProblems({ title: "T" }, { creates: false }), []);
  assert.deepEqual(requestProblems({ state: "closed" }, { creates: false }), []);
  assert.match(requestProblems({ title: `T ${han}` }, { creates: false }).join("\n"), /title: CJK text/);
  assert.match(requestProblems({ body: "No declaration." }, { creates: false }).join("\n"), /body: it declares neither/);
  assert.match(requestProblems({ body: null }, { creates: false }).join("\n"), /body: it declares neither/, "a body cleared passes");
  for (const bad of [null, [], "text", 7]) assert.deepEqual(requestProblems(bad, { creates: false }), ["the request is no JSON object"], JSON.stringify(bad));
  assert.deepEqual(requestProblems({ body: 7 }, { creates: false }), ["the request's body is no string"]);
  assert.deepEqual(requestProblems({ title: ["T"] }, { creates: true }), ["the request's title is no string"]);
  // The CLI: a Markdown draft with its title, or a request, JSON, read whole; a byte order mark
  // before either is no part of it.
  const files = new Map([
    ["body.md", `\uFEFF${declared}`],
    ["bare.md", "No declaration."],
    ["request.json", `\uFEFF${JSON.stringify({ title: "T", body: declared })}`],
    ["edit.json", JSON.stringify({ body: "No declaration." })],
    ["title.json", JSON.stringify({ title: "T" })],
    ["broken.json", "{"],
  ]);
  const cli = (...args) => bodyMain(args, (f) => files.get(f));
  assert.equal(cli("body.md").code, 0, "a draft that begins with a byte order mark fails");
  assert.equal(cli("body.md", "--title", `T ${han}`).code, 1);
  assert.equal(cli("bare.md").code, 1);
  assert.equal(cli("request.json").code, 0, "a request that begins with a byte order mark fails");
  assert.match(cli("edit.json").lines.join("\n"), /body: it declares neither/);
  assert.match(cli("title.json").lines.join("\n"), /body: it declares neither/, "a request with no body passes the CLI, which reads one whole");
  assert.equal(cli("broken.json").code, 2);
  assert.equal(cli("request.json", "--title", "T").code, 2, "a request's title given twice");
  assert.equal(cli().code, 2);
  assert.equal(cli("a.md", "b.md").code, 2);
  assert.equal(cli("body.md", "--other").code, 2);
});

