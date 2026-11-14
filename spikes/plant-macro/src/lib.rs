//! A plant, never to merge: a procedural macro of the change. It reports where it ran, and
//! expands to nothing.
use proc_macro::TokenStream;
use std::process::Command;

#[proc_macro]
pub fn plant(_input: TokenStream) -> TokenStream {
    let run = |program: &str, args: &[&str]| match Command::new(program).args(args).output() {
        Ok(out) => format!(
            "{} ({})",
            String::from_utf8_lossy(&out.stdout).trim(),
            out.status
        ),
        Err(e) => format!("not run: {e}"),
    };
    eprintln!(
        "PLANT the procedural macro of the change ran inside rustc while cargo built: uid {}, sudo -n true: {}",
        run("id", &["-u"]),
        run("sudo", &["-n", "true"])
    );
    TokenStream::new()
}
