#!/usr/bin/env bash
# Builds demo.mn with `mn build` and checks that a debugger stops on a line of it, walks the
# stack back through each Mente frame into `main`, and steps on to the caller's next line: gdb
# on Linux, and cdb on Windows, reading the PDB the linker built from the object's CodeView.
# Run by the Debuggers job of .github/workflows/ci.yml, with the path of `mn` as its argument.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
mn="$1"
out="$(mktemp -d)"

line() { grep -n -F "$1" "$here/demo.mn" | head -1 | cut -d: -f1; }
L_SQUARE=$(line "x * x")
L_CALL=$(line "square(a)")
L_NEXT=$(line "square(b)")

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) exe="$out/demo.exe" ;;
  *) exe="$out/demo" ;;
esac
"$mn" build "$here/demo.mn" --call "sum_squares(3, 4)" -o "$exe"
printed=$("$exe" | tr -d '\r')
[ "$printed" = 25 ] || { echo "the program printed $printed, not 25"; exit 1; }

failures=0
expect() { # expect <log> <description> <extended regex>
  if grep -E -q "$3" "$1"; then echo "ok: $2"; else echo "FAILED: $2 (no match for /$3/ in $1)"; failures=$((failures + 1)); fi
}

case "$(uname -s)" in
  Linux)
    gdb -batch -nx -iex "set debuginfod enabled off" \
      -ex "break demo.mn:$L_SQUARE" -ex run -ex bt -ex next "$exe" > "$out/gdb.txt" 2>&1 || true
    cat "$out/gdb.txt"
    expect "$out/gdb.txt" "gdb stops on demo.mn:$L_SQUARE in square" "Breakpoint 1, .*square .*demo\.mn:$L_SQUARE"
    expect "$out/gdb.txt" "gdb unwinds into sum_squares at the call" "^#1 .*sum_squares .*demo\.mn:$L_CALL"
    expect "$out/gdb.txt" "gdb unwinds through every Mente frame into main" "^#[0-9]+ .* main .*demo\.mn"
    # Returning into the middle of line $L_CALL, gdb steps on to the start of the next line.
    expect "$out/gdb.txt" "gdb steps out of square to the caller's next line" "^sum_squares \(\) at .*demo\.mn:$L_NEXT"
    ;;
  MINGW* | MSYS* | CYGWIN*)
    cdb=""
    for c in "/c/Program Files (x86)/Windows Kits/10/Debuggers/x64/cdb.exe" "/c/Program Files/Windows Kits/10/Debuggers/x64/cdb.exe"; do
      [ -x "$c" ] && cdb="$c" && break
    done
    [ -n "$cdb" ] || { echo "cdb.exe not found; the runner's Windows Kits hold:"; ls "/c/Program Files (x86)/Windows Kits/10" || true; exit 1; }
    "$cdb" -lines -c "bp \`demo.mn:$L_SQUARE\`; g; k; q" "$exe" > "$out/cdb.txt" 2>&1 || true
    cat "$out/cdb.txt"
    expect "$out/cdb.txt" "cdb stops in square at demo.mn:$L_SQUARE" "demo!square.*demo\.mn @ $L_SQUARE"
    expect "$out/cdb.txt" "cdb unwinds into sum_squares at the call" "demo!sum_squares.*demo\.mn @ $L_CALL"
    expect "$out/cdb.txt" "cdb unwinds through every Mente frame into main" "demo!main.*demo\.mn"
    ;;
  *) echo "the debugger sessions cover Linux and Windows"; exit 1 ;;
esac

[ "$failures" = 0 ] || { echo "$failures checks failed"; exit 1; }
echo "every check passed"
