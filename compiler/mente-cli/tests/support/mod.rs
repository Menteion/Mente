//! What the integration tests share, each including it as `mod support;`.

use std::path::Path;
use std::process::Command;

/// `git`, to run in `dir`, isolated from this machine as `.github/scripts/policy-collect.mjs`
/// isolates the git the workflow policy runs: no user or system configuration, and no default
/// excludes file (`~/.config/git/ignore`), so that only the repository's own rules apply; no
/// file-system monitor, a program `.git/config` could name for `git ls-files` to start; and no `GIT_`
/// variable of the caller's, such as a hook's `GIT_DIR` or a `GIT_CONFIG_PARAMETERS`.
pub fn isolated_git(dir: &Path) -> Command {
    let mut git = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            git.env_remove(key);
        }
    }
    git.args([
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.excludesFile=/dev/null",
    ])
    .current_dir(dir)
    .env("GIT_CONFIG_GLOBAL", "/dev/null")
    .env("GIT_CONFIG_NOSYSTEM", "1");
    git
}
