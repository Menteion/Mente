//! `spike-cranelift-debug-info <demo.mn> <out object>`: compiles demo.mn's functions for the
//! host into an object with debug and unwind information (README.md).

use std::process::ExitCode;

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [source, out] = args.as_slice() else {
        return Err("usage: spike-cranelift-debug-info <demo.mn> <out object>".into());
    };
    let text = std::fs::read_to_string(source).map_err(|e| format!("{source}: {e}"))?;
    let full = std::fs::canonicalize(source).map_err(|e| format!("{source}: {e}"))?;
    let dir = full.parent().ok_or("demo.mn has no directory")?;
    // On Windows, canonicalize gives a verbatim path (`\\?\C:\...`), which debuggers do not match.
    let dir = dir.to_string_lossy();
    let dir = dir.strip_prefix(r"\\?\").unwrap_or(&dir);
    // Debuggers look the file up by the name and directory the debug information gives.
    let (bytes, fns) = spike_cranelift_debug_info::build_object(&text, "demo.mn", dir)?;
    std::fs::write(out, bytes).map_err(|e| format!("{out}: {e}"))?;
    for f in fns {
        println!(
            "{}: {} bytes, {} line ranges, unwind {}",
            f.name,
            f.size,
            f.lines.len(),
            if f.unwind.is_some() { "yes" } else { "no" }
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("spike-cranelift-debug-info: {e}");
            ExitCode::FAILURE
        }
    }
}
