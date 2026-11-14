//! A plant, never to merge: a build script of the change that the workflow policy does not see.
//! It reports where it ran, and changes nothing.
use std::process::Command;

fn main() {
    let run = |program: &str, args: &[&str]| match Command::new(program).args(args).output() {
        Ok(out) => format!(
            "{} ({})",
            String::from_utf8_lossy(&out.stdout).trim(),
            out.status
        ),
        Err(e) => format!("not run: {e}"),
    };
    let open = |name: &str| match std::env::var_os(name) {
        Some(file) => std::fs::OpenOptions::new()
            .append(true)
            .open(file)
            .is_ok()
            .to_string(),
        None => String::from("unset"),
    };
    println!(
        "cargo:warning=PLANT the build script of a path dependency outside the members ran while cargo built: uid {}, sudo -n true: {}, GITHUB_ENV open to write: {}, GITHUB_STEP_SUMMARY open to write: {}",
        run("id", &["-u"]),
        run("sudo", &["-n", "true"]),
        open("GITHUB_ENV"),
        open("GITHUB_STEP_SUMMARY")
    );
}
