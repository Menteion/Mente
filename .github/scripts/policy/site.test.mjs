// The workflow policy: the documentation site's publication (README.md in this directory).

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { read } from "./repo.mjs";
import { beforeToolchainProblems, sameKeys } from "./toolchain.mjs";
import { isPlain, normalize, steps, triggers, workflows } from "./workflows.mjs";

/// Every check of this module counts itself when it runs, and the module fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// each module's FLOOR equal to the checks it declares.
const FLOOR = 1;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks of site.test.mjs ran: a skip or a filter left the rest out`);
});
/// What is wrong with how pages-publish.yml publishes the site (Norman, 2026-11-17): it runs by hand
/// alone, with a required `tag` as its one input, and every job runs only from main, so that its
/// definition is always main's. A push of a tag would run the workflow as the tagged commit holds
/// it, which could drop the check that the tag is on main.
export function publishProblems(doc) {
  const found = [];
  const on = triggers(doc);
  const events = Object.keys(on);
  if (events.length !== 1 || events[0] !== "workflow_dispatch") found.push(`pages-publish.yml runs on ${events.join(", ") || "nothing"}, not by hand alone`);
  const inputs = on.workflow_dispatch?.inputs;
  if (!isPlain(inputs) || !sameKeys(inputs, ["tag"]) || inputs.tag?.required !== true || inputs.tag?.type !== "string") found.push("pages-publish.yml takes other inputs than one required string, tag");
  const jobs = isPlain(doc.jobs) ? Object.entries(doc.jobs) : [];
  if (jobs.length === 0) found.push("pages-publish.yml has no job");
  for (const [id, job] of jobs) {
    if (normalize(String(job?.if ?? "")) !== "github.ref == 'refs/heads/main'") found.push(`pages-publish.yml, job ${id}, runs from refs other than main: if: ${job?.if ?? "(none)"}`);
  }
  return found;
}

check("the site is published only by hand, from main's own definition, of a tag on main", () => {
  const doc = workflows().find((w) => w.file === "pages-publish.yml").doc;
  assert.deepEqual(publishProblems(doc), []);
  // The tag is proven on main before anything of it is checked out, and only then.
  const build = steps(doc.jobs.build);
  assert.equal(build[1]?.run, "bash .github/scripts/publish-tag.sh", "the build's step after the checkout is not the tag's proof");
  assert.match(read(".github/scripts/publish-tag.sh"), /\n"\$BASH" \.github\/scripts\/commit-on-main\.sh "refs\/tags\/\$tag"\ngit -c advice\.detachedHead=false checkout --quiet --detach "refs\/tags\/\$tag"\n/, "publish-tag.sh checks out the tag other than right after proving it on main");
  // Planted: a push of tags, beside the run by hand or alone; any push; a schedule; no input, an
  // optional one, or one more; and a job that runs from another ref, or from any.
  const clone = () => JSON.parse(JSON.stringify(doc));
  const planted = (change) => {
    const d = clone();
    change(d);
    return publishProblems(d).length > 0;
  };
  for (const [what, change] of [
    ["a push of tags beside the run by hand", (d) => (d.on.push = { tags: ["m*"] })],
    ["a push of tags alone", (d) => (d.on = { push: { tags: ["m*"] } })],
    ["a push to main", (d) => (d.on.push = { branches: ["main"] })],
    ["a schedule", (d) => (d.on.schedule = [{ cron: "0 0 * * 1" }])],
    ["no input", (d) => delete d.on.workflow_dispatch.inputs],
    ["an optional tag", (d) => (d.on.workflow_dispatch.inputs.tag.required = false)],
    ["an input more", (d) => (d.on.workflow_dispatch.inputs.ref = { required: false, type: "string" })],
    ["a build from any ref", (d) => delete d.jobs.build.if],
    ["a deployment from a tag", (d) => (d.jobs.deploy.if = "github.ref_type == 'tag'")],
    ["a job more, from any ref", (d) => (d.jobs.extra = { "runs-on": "ubuntu-24.04", steps: [] })],
  ]) {
    assert.ok(planted(change), `${what} passes`);
  }
  // The tag's step carries its one variable, exactly, before the toolchain: planted, a variable
  // more, another value, and none.
  assert.deepEqual(beforeToolchainProblems("pages-publish.yml", "build", doc, doc.jobs.build), []);
  for (const [what, env] of [
    ["a variable more", { TAG: "${{ inputs.tag }}", BASH_ENV: "./x.sh" }],
    ["another value", { TAG: "${{ github.event.inputs.tag }}" }],
    ["none", undefined],
  ]) {
    const d = clone();
    if (env === undefined) delete d.jobs.build.steps[1].env;
    else d.jobs.build.steps[1].env = env;
    assert.ok(beforeToolchainProblems("pages-publish.yml", "build", d, d.jobs.build).length > 0, `the tag's step with ${what} passes`);
  }
});
