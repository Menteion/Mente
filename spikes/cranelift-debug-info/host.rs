// The host program the spike's object is linked into: rustc finds the platform linker, and
// with `-C debuginfo=2` keeps the object's DWARF (Linux) or turns its CodeView into a PDB
// (Windows).
extern "C" {
    fn mente_main() -> i64;
}

fn main() {
    // SAFETY: the spike's object defines `mente_main` with the platform's C calling convention.
    let v = unsafe { mente_main() };
    println!("{v}");
}
