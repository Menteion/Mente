// The YAML reader the workflow policy test reads the workflows with (lib/yaml.mjs): `npm run test:ci`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseYaml } from "./lib/yaml.mjs";

test("mappings, sequences and scalars read as YAML reads them", () => {
  const doc = parseYaml(`# a comment
name: CI
on:
  pull_request:
    types: [opened, "edited"]
  merge_group:
permissions:
  contents: read
jobs:
  a:
    name: Rust (\${{ matrix.name }})
    strategy:
      matrix:
        include:
          - name: x86-64 Linux # trailing comment
            os: ubuntu-24.04
          - name: 'it''s'
            os: windows-2025
    steps:
      - uses: actions/checkout@0123 # v7
        with:
          fetch-depth: 0
          persist-credentials: false
      - run: |
          echo "a #not a comment"
          # a line of the script

          tail
      - run: >-
          node x
          --flag
      - name: "quoted: with colon"
        if: \${{ !cancelled() }}
        run: echo "x" # comment
list:
- one
- two
`);
  assert.equal(doc.name, "CI");
  assert.deepEqual(doc.on, { pull_request: { types: ["opened", "edited"] }, merge_group: null });
  assert.deepEqual(doc.jobs.a.strategy.matrix.include, [
    { name: "x86-64 Linux", os: "ubuntu-24.04" },
    { name: "it's", os: "windows-2025" },
  ]);
  const [checkout, script, folded, last] = doc.jobs.a.steps;
  assert.deepEqual(checkout, { uses: "actions/checkout@0123", with: { "fetch-depth": 0, "persist-credentials": false } });
  assert.equal(script.run, 'echo "a #not a comment"\n# a line of the script\n\ntail\n');
  assert.equal(folded.run, "node x --flag");
  assert.deepEqual(last, { name: "quoted: with colon", if: "${{ !cancelled() }}", run: 'echo "x"' });
  assert.deepEqual(doc.list, ["one", "two"]);
});

test("what the subset leaves out is refused, not misread", () => {
  for (const bad of [
    "a: {b: 1}\n",
    "a: &x 1\n",
    "a: *x\n",
    "a: plain\n  continued\n",
    "a: 1\na: 2\n",
    "a:\n\tb: 1\n",
    "a: 'open\n",
    "a: 1\n---\nb: 2\n",
    "a:\n  - x\n   - y\n",
    '"continue-on\\u002derror": true\n',
    '"i\\x66": "false"\n',
    'a: "\\u0041"\n',
    'a: "\\x41"\n',
    "a:\n  - \"i\\u0066\": x\n",
  ]) {
    assert.throws(() => parseYaml(bad), /line \d+/, JSON.stringify(bad));
  }
});

test("a character GitHub's reader could break a line at, or any other outside printable ASCII, is refused", () => {
  // The fourth review of PR 137: NEL (U+0085) in a comment hid a step from this reader and from
  // every pin, while GitHub's reader broke the line there and read the step.
  for (const c of [0x85, 0x2028, 0x2029, 0xa0, 0x200e, 0xfeff, 0x0d, 0x0b, 0x0c, 0x7f, 0x1b]) {
    const ch = String.fromCharCode(c);
    assert.throws(() => parseYaml(`a: 1\n# x${ch}b: 2\n`), /line 2: a character outside printable ASCII/, `U+${c.toString(16)}`);
  }
  assert.deepEqual(parseYaml("a: 1\r\nb: 2\r\n"), { a: 1, b: 2 });
});
