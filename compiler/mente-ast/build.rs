// A plant, never to merge: code that would run at build time, outside every sandbox.
fn main() {
    let _ = std::fs::write("/tmp/mente-plant-build-script-ran", "ran
");
    println!("cargo:warning=the plant's build script ran");
}
