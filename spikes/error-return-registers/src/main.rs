//! `spike-error-return-registers [--opt none|speed] [--target <triple>] [<main object>
//! <implicit-sret object>]`: compiles the spike's chains for the machine running it, or for
//! `<triple>`, and prints what Cranelift made of each function (README.md). Given the two
//! paths, it also writes the two objects, which check.sh links into host.rs.

use std::process::ExitCode;

use spike_error_return_registers as spike;

const USAGE: &str = "usage: spike-error-return-registers [--opt none|speed] [--target <triple>] [<main object> <implicit-sret object>]";

fn run() -> Result<(), String> {
    let mut opt = "speed".to_string();
    let mut target: Option<String> = None;
    let mut paths = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--opt" => opt = args.next().ok_or(USAGE)?,
            "--target" => target = Some(args.next().ok_or(USAGE)?),
            _ => paths.push(a),
        }
    }
    let outputs = match paths.as_slice() {
        [] => None,
        [main, implicit] => Some((main.clone(), implicit.clone())),
        _ => return Err(USAGE.into()),
    };
    let triple = target.as_deref();

    let isa = spike::isa(triple, &opt, false)?;
    let name = isa.triple().to_string();
    let main = spike::build_object(isa, "main", &spike::MAIN_CHAINS, true)?;
    print!(
        "{}",
        spike::report(
            &format!("{name}, opt_level={opt}, default flags"),
            &main,
            &spike::MAIN_CHAINS
        )
    );

    let isa = spike::isa(triple, &opt, true)?;
    let implicit = spike::build_object(isa, "implicit", &spike::IMPLICIT_CHAINS, false)?;
    print!(
        "\n{}",
        spike::report(
            &format!("{name}, opt_level={opt}, enable_multi_ret_implicit_sret"),
            &implicit,
            &spike::IMPLICIT_CHAINS
        )
    );

    match spike::struct_return_with_tag(triple) {
        Ok(()) => println!("\nexplicit StructReturn beside a returned tag: compiles"),
        Err(e) => println!("\nexplicit StructReturn beside a returned tag: refused: {e}"),
    }

    if let Some((main_out, implicit_out)) = outputs {
        std::fs::write(&main_out, &main.object).map_err(|e| format!("{main_out}: {e}"))?;
        std::fs::write(&implicit_out, &implicit.object)
            .map_err(|e| format!("{implicit_out}: {e}"))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("spike-error-return-registers: {e}");
            ExitCode::FAILURE
        }
    }
}
