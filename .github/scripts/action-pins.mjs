// The CLI of lib/action-pins.mjs: node .github/scripts/action-pins.mjs, from the repository's root.
import fs from "node:fs";
import { execFileSync } from "node:child_process";
import { main } from "./lib/action-pins.mjs";

main(
  ".github/workflows",
  (dir) => fs.readdirSync(dir),
  (file) => fs.readFileSync(file, "utf8"),
  (repo, tag) => execFileSync("git", ["ls-remote", `https://github.com/${repo}`, `refs/tags/${tag}`, `refs/tags/${tag}^{}`], { encoding: "utf8" }),
);
