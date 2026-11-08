#!/usr/bin/env bash
# Builds demo.mn's object with the spike, links it into host.rs with rustc, and checks that a
# debugger stops on a source line of demo.mn, walks the stack back through every Cranelift
# frame into the host, and steps to the next line. gdb and lldb on Linux; cdb on Windows,
# reading the PDB the linker built from the object's CodeView. Run by
# .github/workflows/spike-cranelift.yml.
set -euo pipefail
cd "$(dirname "$0")"

line() { grep -n -F "$1" demo.mn | head -1 | cut -d: -f1; }
L_SQUARE=$(line "return x * x")
L_CALL=$(line "s += square(i)")
L_NEXT=$(line "i += 1")
L_MAIN=$(line "return sum_squares(10)")

mkdir -p target/demo
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) obj=target/demo/demo.obj exe=target/demo/demo.exe ;;
  *) obj=target/demo/demo.o exe=target/demo/demo ;;
esac
cargo run --locked --quiet -- demo.mn "$obj"
rustc -C debuginfo=2 -C link-arg="$obj" host.rs -o "$exe"
out=$("./$exe")
[ "$out" = 385 ] || { echo "the program printed $out, not 385"; exit 1; }

failures=0
expect() { # expect <log> <description> <extended regex>
  if grep -E -q "$3" "$1"; then echo "ok: $2"; else echo "FAILED: $2 (no match for /$3/ in $1)"; failures=$((failures + 1)); fi
}

case "$(uname -s)" in
  Linux)
    gdb -batch -nx -iex "set debuginfod enabled off" \
      -ex "break demo.mn:$L_SQUARE" -ex run -ex bt -ex next "$exe" > target/demo/gdb.txt 2>&1 || true
    cat target/demo/gdb.txt
    expect target/demo/gdb.txt "gdb stops on demo.mn:$L_SQUARE in square" "Breakpoint 1, .*square .*demo\.mn:$L_SQUARE"
    expect target/demo/gdb.txt "gdb unwinds into sum_squares at the call" "^#1 .*sum_squares .*demo\.mn:$L_CALL"
    expect target/demo/gdb.txt "gdb unwinds into mente_main at the call" "^#2 .*mente_main .*demo\.mn:$L_MAIN"
    expect target/demo/gdb.txt "gdb unwinds through every Cranelift frame into the host" "^#3 .*host::main"
    # Returning into the middle of line $L_CALL, gdb steps on to the start of the next line.
    expect target/demo/gdb.txt "gdb steps out of square to the caller's next line" "^sum_squares \(\) at .*demo\.mn:$L_NEXT"

    lldb --batch -o "breakpoint set --file demo.mn --line $L_SQUARE" -o run -o "thread backtrace" -o "thread step-over" "$exe" > target/demo/lldb.txt 2>&1 || true
    cat target/demo/lldb.txt
    expect target/demo/lldb.txt "lldb stops on demo.mn:$L_SQUARE in square" "frame #0: .*square at demo\.mn:$L_SQUARE"
    expect target/demo/lldb.txt "lldb unwinds into sum_squares at the call" "frame #1: .*sum_squares at demo\.mn:$L_CALL"
    expect target/demo/lldb.txt "lldb unwinds into mente_main at the call" "frame #2: .*mente_main at demo\.mn:$L_MAIN"
    expect target/demo/lldb.txt "lldb unwinds through every Cranelift frame into the host" "frame #3: .*host::main"
    expect target/demo/lldb.txt "lldb steps out of square to the caller at the call" "frame #0: .*sum_squares at demo\.mn:$L_CALL"
    ;;
  MINGW* | MSYS* | CYGWIN*)
    cdb=""
    for c in "/c/Program Files (x86)/Windows Kits/10/Debuggers/x64/cdb.exe" "/c/Program Files/Windows Kits/10/Debuggers/x64/cdb.exe"; do
      [ -x "$c" ] && cdb="$c" && break
    done
    [ -n "$cdb" ] || { echo "cdb.exe not found; the runner's Windows Kits hold:"; ls "/c/Program Files (x86)/Windows Kits/10" || true; exit 1; }
    "$cdb" -lines -c "bp \`demo.mn:$L_SQUARE\`; g; k; q" "$exe" > target/demo/cdb.txt 2>&1 || true
    cat target/demo/cdb.txt
    expect target/demo/cdb.txt "cdb stops in square at demo.mn:$L_SQUARE" "demo!square.*demo\.mn @ $L_SQUARE"
    expect target/demo/cdb.txt "cdb unwinds into sum_squares at the call" "demo!sum_squares.*demo\.mn @ $L_CALL"
    expect target/demo/cdb.txt "cdb unwinds into mente_main at the call" "demo!mente_main.*demo\.mn @ $L_MAIN"
    expect target/demo/cdb.txt "cdb unwinds through every Cranelift frame into the host" "demo!host::main"
    ;;
  *) echo "the spike covers Linux and Windows"; exit 1 ;;
esac

[ "$failures" = 0 ] || { echo "$failures checks failed"; exit 1; }
echo "every check passed"
