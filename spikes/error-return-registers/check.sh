#!/usr/bin/env bash
# Builds the spike's two objects for the machine running it at both of Cranelift's opt levels,
# links them into host.rs with rustc, and runs the host, which checks every round trip and
# fails on a mismatch. Then prints the independent disassembly of the functions README.md
# quotes, and Rust's code for the same shapes. Run by .github/workflows/spike-error-return.yml.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p target/run
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) ext=obj exe=.exe ;;
  *) ext=o exe= ;;
esac

for opt in none speed; do
  main=target/run/main-$opt.$ext
  implicit=target/run/implicit-$opt.$ext
  report=target/run/report-$opt.txt
  cargo run --locked --quiet -- --opt "$opt" "$main" "$implicit" > "$report"
  echo "::group::Cranelift's listings, opt_level=$opt"
  cat "$report"
  echo "::endgroup::"
  echo "== where each position travels, opt_level=$opt =="
  grep -E '^(# |placement|refused|explicit)' "$report"
  echo "== round trips, opt_level=$opt =="
  rustc --edition 2021 -C opt-level=2 -C link-arg="$main" -C link-arg="$implicit" host.rs -o "target/run/host-$opt$exe"
  "./target/run/host-$opt$exe"
done

# An independent disassembler over the object the speed build linked.
dis=""
for d in objdump llvm-objdump "/c/Program Files/LLVM/bin/llvm-objdump.exe"; do
  if command -v "$d" > /dev/null 2>&1 || [ -x "$d" ]; then dis="$d"; break; fi
done
[ -n "$dis" ] || { echo "no objdump or llvm-objdump found"; exit 1; }
instructions() { grep -E '^ *[0-9a-f]+:' | grep -v -E ':[[:space:]]+(R_|IMAGE_REL_)' | wc -l | tr -d ' '; }
echo "== $("$dis" --version | head -1) on target/run/main-speed.$ext =="
"$dis" -dr --no-show-raw-insn "target/run/main-speed.$ext" > target/run/objdump.txt
for f in pair_leaf pair_forward pair32_forward plain_forward pair_middle pair32_middle wide_tail_middle wide_buffer_middle; do
  body=$(awk -v f="$f" '$0 ~ "^[0-9a-f]+ <" f ">:" {p = 1} p && /^$/ {exit} p {print}' target/run/objdump.txt)
  [ -n "$body" ] || { echo "the object has no $f"; exit 1; }
  echo "$body"
  echo "-- $f: $(echo "$body" | instructions) instructions"
done

# Rust's code for the same shapes, from rustc's own assembly output.
rustc --edition 2021 -C opt-level=3 -C panic=abort --emit=asm -o target/run/rust_q.s rust_q.rs
echo "== $(rustc --version), -C opt-level=3 =="
for f in rust_leaf rust_forward rust_plain_forward rust_middle rust_outer; do
  # A function ends at its end label (ELF), its unwind directive, or the next symbol's
  # definition (COFF, where a leaf has neither).
  body=$(awk -v f="$f" 'p && /^\.Lfunc_end|^[[:space:]]*\.(cfi_endproc|seh_endproc|def|size)([[:space:]]|$)/ {exit} $0 ~ "^" f ":" {p = 1} p {print}' target/run/rust_q.s)
  [ -n "$body" ] || { echo "rustc's assembly has no $f"; exit 1; }
  echo "$body" | grep -v -E '^\s*\.(cfi|seh)_'
  echo "-- $f: $(echo "$body" | grep -E '^\s+[a-z]' | grep -v -E '^\s+\.' | wc -l | tr -d ' ') instructions"
done
echo "every check passed"
