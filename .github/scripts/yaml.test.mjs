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
