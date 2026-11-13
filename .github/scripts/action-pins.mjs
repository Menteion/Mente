// The CLI of lib/action-pins.mjs: node .github/scripts/action-pins.mjs, from the repository's root.
import fs from "node:fs";
import { execFileSync } from "node:child_process";
import { main } from "./lib/action-pins.mjs";

// git ls-remote for one tag, within a minute, tried twice: a network that does not answer fails the
// check, never passes it.
const lsRemote = (repo, tag) => {
  const ask = () => execFileSync("git", ["ls-remote", `https://github.com/${repo}`, `refs/tags/${tag}`, `refs/tags/${tag}^{}`], { encoding: "utf8", timeout: 60000 });
  try {
    return ask();
  } catch {
    return ask();
  }
};

main(
  ".github/workflows",
  (dir) => fs.readdirSync(dir),
  (file) => fs.readFileSync(file, "utf8"),
  lsRemote,
);
