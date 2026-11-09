//! The error-code catalogue `mn explain` reads (`spec/diagnostics.md`).
//!
//! Every code the compiler reports has an entry: what it means, the spec that governs it, a
//! program that must fail with it and a near miss that must pass; tests run both. A code the
//! spec defines but the compiler does not report yet is `Reserved`, and a code is never reused:
//! one that is withdrawn stays here as `Retired`. Each code also has a failure class, which the
//! agent benchmark counts when a program fails to build (`ai-coding-problems`).

/// What kind of mistake a code reports. A class joins with the first code that needs it: foreign
/// idioms and unknown names in M1, effects in M4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureClass {
    /// The text does not parse.
    Syntax,
    /// A view, an arena value or a task-bound value is used where its provenance does not reach.
    Provenance,
    /// A layout, bit-field or branch-freedom contract does not hold.
    HardwareContract,
    /// A contract, a refinement or a termination measure is not proven.
    Proof,
    /// Types do not match, units included.
    Type,
    /// A compile-time evaluation goes wrong.
    Comptime,
}

impl FailureClass {
    /// The name `mn explain` prints and the benchmark report counts.
    pub fn name(self) -> &'static str {
        match self {
            FailureClass::Syntax => "syntax",
            FailureClass::Provenance => "provenance",
            FailureClass::HardwareContract => "hardware contract",
            FailureClass::Proof => "proof",
            FailureClass::Type => "type",
            FailureClass::Comptime => "comptime",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeStatus {
    /// The compiler reports it today.
    Emitted,
    /// A spec defines it; no check reports it yet.
    Reserved,
    /// Withdrawn; kept so the number is never given to anything else.
    #[allow(dead_code)]
    Retired,
}

#[derive(Clone, Copy, Debug)]
pub struct CodeEntry {
    pub code: &'static str,
    pub class: FailureClass,
    pub status: CodeStatus,
    pub spec: &'static str,
    pub title: &'static str,
    pub explain: &'static str,
    /// A module that must fail with this code; empty unless the code is emitted.
    pub fails: &'static str,
    /// The nearest module that must pass; empty unless the code is emitted.
    pub passes: &'static str,
}

const fn emitted(
    code: &'static str,
    class: FailureClass,
    spec: &'static str,
    title: &'static str,
    explain: &'static str,
    fails: &'static str,
    passes: &'static str,
) -> CodeEntry {
    CodeEntry {
        code,
        class,
        status: CodeStatus::Emitted,
        spec,
        title,
        explain,
        fails,
        passes,
    }
}

const fn reserved(
    code: &'static str,
    class: FailureClass,
    spec: &'static str,
    title: &'static str,
    explain: &'static str,
) -> CodeEntry {
    CodeEntry {
        code,
        class,
        status: CodeStatus::Reserved,
        spec,
        title,
        explain,
        fails: "",
        passes: "",
    }
}

/// Every code, in order.
pub const CATALOGUE: &[CodeEntry] = &[
    emitted(
        "E0101",
        FailureClass::Syntax,
        "spec/surface-syntax.md",
        "A name was expected",
        "A declaration, a field, a module path or an attribute needs a name at this point, and \
         found something else: a number, a symbol, a keyword or the end of the file. Keywords \
         such as `raise` or `from` cannot be names.",
        "module m\npub struct 42 {\n}\n",
        "module m\npub struct Answer {\n}\n",
    ),
    emitted(
        "E0102",
        FailureClass::Syntax,
        "spec/surface-syntax.md",
        "An opening brace was expected",
        "A `struct` and an `@example` open their body with `{`. The parser found something else \
         where the brace belongs.",
        "module m\npub struct S\n  a: u8\n",
        "module m\npub struct S {\n  a: u8\n}\n",
    ),
    emitted(
        "E0103",
        FailureClass::Syntax,
        "spec/surface-syntax.md",
        "A declaration has no closing brace",
        "A `struct`, `bitstruct`, `enum` or `trait` opened with `{` is not closed before the file \
         ends. The diagnostic points at where the parser stopped, names the declaration in a \
         related span, and suggests a `}`; the right place may be earlier.",
        "module m\npub enum Side { Left, Right\n",
        "module m\npub enum Side { Left, Right }\n",
    ),
    emitted(
        "E0104",
        FailureClass::Syntax,
        "spec/regions-and-pools.md",
        "A from clause names no parameter",
        "`from` after a return type lists the parameters a returned view may borrow from, \
         separated by commas. Written with no name after it, it says nothing.",
        "module m\npub fn first(data: []u8) -> []u8 from\n",
        "module m\npub fn first(data: []u8) -> []u8 from data\n",
    ),
    emitted(
        "E0105",
        FailureClass::Syntax,
        "spec/grammar.md",
        "A character Mente does not use",
        "Outside comments and string literals, Mente uses ASCII letters, digits, `_` and the \
         operators and punctuation of the grammar. Anything else, such as `#`, `$`, `'` or a \
         letter outside ASCII, is an error rather than being skipped. An attribute is written \
         `@name`, not `#[name]`, and Mente writes no lifetimes and has no character literals.",
        "module m\npub fn f() -> i32 = 1 # 2\n",
        "module m\npub fn f() -> i32 = 1\n",
    ),
    emitted(
        "E0106",
        FailureClass::Syntax,
        "spec/grammar.md",
        "A string literal is not closed on its line",
        "A string literal ends with `\"` on the line it starts on. A line break inside a string \
         is written `\\n`. The diagnostic points at the string from its opening quote.",
        "module m\npub fn f() -> i32 {\n  todo(\"unfinished)\n}\n",
        "module m\npub fn f() -> i32 {\n  todo(\"unfinished\")\n}\n",
    ),
    emitted(
        "E0107",
        FailureClass::Syntax,
        "spec/grammar.md",
        "A string literal uses an escape Mente does not know",
        "A string literal knows four escapes: `\\\"` for a quote, `\\\\` for a backslash, `\\n` \
         for a line break and `\\t` for a tab. Any other character after a backslash is an \
         error.",
        "module m\npub fn f() -> i32 {\n  todo(\"a\\qb\")\n}\n",
        "module m\npub fn f() -> i32 {\n  todo(\"a\\tb\")\n}\n",
    ),
    emitted(
        "E0108",
        FailureClass::Syntax,
        "spec/grammar.md",
        "An integer literal is malformed",
        "An integer literal is decimal, hexadecimal after `0x` or binary after `0b`, with at \
         least one digit, and `_` only between two digits or before its suffix: `1_000`, \
         `0xFF_u8`, `0b1010`. A digit its base does not have, a suffix written without its \
         `_` (`5ms` for `5_ms`, which the fix inserts) and a value beyond `u128` are errors.",
        "module m\npub fn f() -> u8 = 0b102\n",
        "module m\npub fn f() -> u8 = 0b101\n",
    ),
    reserved(
        "E0310",
        FailureClass::Provenance,
        "spec/regions-and-pools.md",
        "A view outlives the value it borrows from",
        "A returned or stored view borrows from somewhere its signature does not allow, or would \
         outlive its source. The fix is one of two paths: allocate in a caller's arena \
         (`in arena: Arena`), or store a `Handle<T>` into a `Pool<T>` instead of the view. \
         Internal region names never appear in the message.",
    ),
    emitted(
        "E0311",
        FailureClass::Provenance,
        "spec/regions-and-pools.md",
        "A from clause names a parameter a view cannot borrow from",
        "Every name in a `from` clause must be a parameter of the function, and not an `own` one: \
         the callee consumes an `own` value, so a view of it would outlive it. When the function \
         has exactly one parameter a view may borrow from, the fix suggests it.",
        "module m\npub fn head(own data: []u8) -> []u8 from data\n",
        "module m\npub fn head(data: []u8) -> []u8 from data\n",
    ),
    reserved(
        "E0312",
        FailureClass::Provenance,
        "spec/regions-and-pools.md",
        "A type with a destructor is allocated in an arena",
        "An arena is reset in O(1) without running destructors, so a value that owns a resource \
         or heap storage, such as a `String` or a `List<T>`, cannot be allocated in one.",
    ),
    reserved(
        "E0320",
        FailureClass::Provenance,
        "spec/regions-and-pools.md",
        "A task-bound value crosses a task boundary",
        "A `Local<T>` counts references without atomic instructions, so a value holding one may \
         not be sent over a channel, moved into another task, or named in a branch of \
         `parallel (...)` or a `parallel_for` body. Use `Shared<T>`.",
    ),
    emitted(
        "E0401",
        FailureClass::HardwareContract,
        "spec/hardware-contracts.md",
        "A @branchless function loops with while",
        "`@branchless` forbids every data-dependent branch. A `while` loop branches on its \
         condition each time round; a loop with a compile-time trip count (`for i in 0..N`) is \
         unrolled and allowed. MIR reports the same code for a conditional-jump node in a \
         `@branchless` region.",
        "module m\n@branchless\npub fn spin(x: i32) -> i32 {\n  while x > 0 {\n  }\n  x\n}\n",
        "module m\n@branchless\npub fn spin(x: i32) -> i32 {\n  for i in 0..4 {\n  }\n  x\n}\n",
    ),
    emitted(
        "E0402",
        FailureClass::HardwareContract,
        "spec/hardware-contracts.md",
        "A @branchless function raises",
        "`raise` leaves the function along an error path, which is a branch. A `@branchless` \
         function computes its result with selects, and reports failure in its value.",
        "module m\n@branchless\npub fn pick(x: i32) -> i32 ! RangeError {\n  raise RangeError.TooBig\n}\n",
        "module m\npub fn pick(x: i32) -> i32 ! RangeError {\n  raise RangeError.TooBig\n}\n",
    ),
    reserved(
        "E0403",
        FailureClass::HardwareContract,
        "spec/hardware-contracts.md",
        "A @branchless function's machine code has a conditional jump",
        "After code generation, `mn` disassembles every `@branchless` function and rejects a \
         conditional jump the backend introduced, naming the source line.",
    ),
    emitted(
        "E0410",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A @layout alignment is not a power of two",
        "`@layout(size: S, align: A)` needs `A` to be a power of two. The fix suggests the next \
         one up.",
        "module m\n@layout(size: 8, align: 3)\npub struct S {\n  @bytes(0..8) pub v: u64\n}\n",
        "module m\n@layout(size: 8, align: 8)\npub struct S {\n  @bytes(0..8) pub v: u64\n}\n",
    ),
    emitted(
        "E0411",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A @layout size is not a multiple of its alignment",
        "In `@layout(size: S, align: A)`, `S` must be a multiple of `A`, so an array of the struct \
         keeps every element aligned. The fix suggests rounding `S` up.",
        "module m\n@layout(size: 6, align: 4)\npub struct S {\n  @bytes(0..4) pub a: u32\n  @bytes(4..6) pub b: u16\n}\n",
        "module m\n@layout(size: 8, align: 4)\npub struct S {\n  @bytes(0..4) pub a: u32\n  @bytes(4..6) pub b: u16\n  @bytes(6..8) pub pad: u16\n}\n",
    ),
    emitted(
        "E0412",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A @layout field has no @bytes range",
        "Every field of a `@layout` struct states its bytes with `@bytes(start..end)`. When the \
         field's type has a known size, contiguity leaves one possible range, and the fix that \
         inserts it is machine-applicable.",
        "module m\n@layout(size: 8, align: 4)\npub struct S {\n  @bytes(0..4) pub a: u32\n  pub b: u32\n}\n",
        "module m\n@layout(size: 8, align: 4)\npub struct S {\n  @bytes(0..4) pub a: u32\n  @bytes(4..8) pub b: u32\n}\n",
    ),
    emitted(
        "E0413",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A @layout field leaves a gap or overlaps",
        "Each field starts exactly where the previous one ends: a gap would be padding nobody \
         wrote, and an overlap would alias two fields. Padding is written as a field.",
        "module m\n@layout(size: 8, align: 4)\npub struct S {\n  @bytes(0..4) pub a: u32\n  @bytes(6..8) pub b: u16\n}\n",
        "module m\n@layout(size: 8, align: 4)\npub struct S {\n  @bytes(0..4) pub a: u32\n  @bytes(4..6) pub pad: u16\n  @bytes(6..8) pub b: u16\n}\n",
    ),
    emitted(
        "E0414",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A @bytes range is empty",
        "`@bytes(start..end)` is half-open and must hold at least one byte: `end` is greater than \
         `start`.",
        "module m\n@layout(size: 4, align: 4)\npub struct S {\n  @bytes(0..0) pub a: u32\n}\n",
        "module m\n@layout(size: 4, align: 4)\npub struct S {\n  @bytes(0..4) pub a: u32\n}\n",
    ),
    emitted(
        "E0415",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A @bytes range is not as wide as its type",
        "The range of a field holds exactly its type: a `u32` takes 4 bytes, a `[6]u8` 6, a \
         `q16.16` 4.",
        "module m\n@layout(size: 8, align: 4)\npub struct S {\n  @bytes(0..4) pub a: u32\n  @bytes(4..6) pub b: u32\n}\n",
        "module m\n@layout(size: 8, align: 4)\npub struct S {\n  @bytes(0..4) pub a: u32\n  @bytes(4..8) pub b: u32\n}\n",
    ),
    emitted(
        "E0416",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A @layout struct's fields do not fill its size",
        "The last field ends at the declared size, so no byte of the struct is unaccounted for. \
         Declare the size the fields take, or write the trailing padding as a field.",
        "module m\n@layout(size: 8, align: 4)\npub struct S {\n  @bytes(0..4) pub a: u32\n}\n",
        "module m\n@layout(size: 4, align: 4)\npub struct S {\n  @bytes(0..4) pub a: u32\n}\n",
    ),
    emitted(
        "E0417",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A @layout field's type has no known size",
        "A `@layout` struct means the same bytes on every target, so its fields are fixed-size \
         types: primitives, whole-byte `uN` / `iN`, fixed-point, arrays of those, and the module's \
         own bitstructs and `@layout` structs. `usize` and `isize` depend on the target.",
        "module m\n@layout(size: 8, align: 8)\npub struct S {\n  @bytes(0..8) pub n: usize\n}\n",
        "module m\n@layout(size: 8, align: 8)\npub struct S {\n  @bytes(0..8) pub n: u64\n}\n",
    ),
    emitted(
        "E0418",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A @layout field is not aligned",
        "Unless the struct is `@packed`, each field starts at a multiple of its type's natural \
         alignment. `@packed` allows unaligned fields for wire formats, at the cost of unaligned \
         loads and stores.",
        "module m\n@layout(size: 8, align: 4)\npub struct S {\n  @bytes(0..2) pub a: u16\n  @bytes(2..6) pub b: u32\n  @bytes(6..8) pub c: u16\n}\n",
        "module m\n@packed\n@layout(size: 8, align: 4)\npub struct S {\n  @bytes(0..2) pub a: u16\n  @bytes(2..6) pub b: u32\n  @bytes(6..8) pub c: u16\n}\n",
    ),
    emitted(
        "E0419",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A @layout field needs more alignment than its struct",
        "A field's natural alignment may not exceed the struct's `align`, or an array of the \
         struct would misalign it. Raise the struct's alignment, or mark it `@packed`.",
        "module m\n@layout(size: 8, align: 4)\npub struct S {\n  @bytes(0..8) pub v: u64\n}\n",
        "module m\n@layout(size: 8, align: 8)\npub struct S {\n  @bytes(0..8) pub v: u64\n}\n",
    ),
    emitted(
        "E0420",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A bitstruct's backing type is not supported",
        "A bitstruct packs its fields into `u8`, `u16`, `u32`, `u64` or `u128`. The fix suggests \
         the smallest of those that holds the declared width.",
        "module m\npub bitstruct B : u12 {\n  @bits(0..12) pub v: u12\n}\n",
        "module m\npub bitstruct B : u16 {\n  @bits(0..12) pub v: u12\n  @bits(12..16) pub spare: u4\n}\n",
    ),
    emitted(
        "E0421",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A bitstruct field leaves a gap or overlaps",
        "The `@bits(start..end)` ranges of a bitstruct partition its backing integer: each field \
         starts where the previous one ends.",
        "module m\npub bitstruct B : u8 {\n  @bits(0..4) pub a: u4\n  @bits(5..8) pub b: u3\n}\n",
        "module m\npub bitstruct B : u8 {\n  @bits(0..4) pub a: u4\n  @bits(4..8) pub b: u4\n}\n",
    ),
    emitted(
        "E0422",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A @bits range is empty",
        "`@bits(start..end)` is half-open and must hold at least one bit.",
        "module m\npub bitstruct B : u8 {\n  @bits(0..0) pub a: u8\n}\n",
        "module m\npub bitstruct B : u8 {\n  @bits(0..8) pub a: u8\n}\n",
    ),
    emitted(
        "E0423",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A bitstruct's fields do not fill its backing type",
        "The last field ends at the width of the backing integer, so every bit has a name. Write \
         spare bits as a field, or choose a narrower backing type.",
        "module m\npub bitstruct B : u16 {\n  @bits(0..8) pub a: u8\n}\n",
        "module m\npub bitstruct B : u16 {\n  @bits(0..8) pub a: u8\n  @bits(8..16) pub spare: u8\n}\n",
    ),
    emitted(
        "E0424",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A bitstruct field's type is not as wide as its range",
        "A field's type is exactly as wide as its range: `bool` is 1 bit, `uN` and `iN` are `N` \
         bits, and a nested bitstruct is its backing width.",
        "module m\npub bitstruct B : u8 {\n  @bits(0..3) pub mode: u2\n  @bits(3..8) pub rest: u5\n}\n",
        "module m\npub bitstruct B : u8 {\n  @bits(0..3) pub mode: u3\n  @bits(3..8) pub rest: u5\n}\n",
    ),
    emitted(
        "E0425",
        FailureClass::HardwareContract,
        "spec/morphic-layouts.md",
        "A bitstruct field's type cannot be packed into bits",
        "A bitstruct field is `bool`, `uN`, `iN` or a bitstruct of the same module. Floats, \
         fixed-point values and other types have no bit-exact packing.",
        "module m\npub bitstruct B : u32 {\n  @bits(0..32) pub v: f32\n}\n",
        "module m\npub bitstruct B : u32 {\n  @bits(0..32) pub v: u32\n}\n",
    ),
    emitted(
        "E0501",
        FailureClass::Proof,
        "spec/ghost-contracts-and-totality.md",
        "A @total function loops without a decreases measure",
        "`@total` promises the function terminates, so a `while` loop gives a measure that \
         decreases each time round (`decreases hi - lo`). Today this is a token check: it does \
         not yet verify that the measure decreases.",
        "module m\n@total\npub fn countdown(n: u32) -> u32 {\n  while n > 0 {\n  }\n  n\n}\n",
        "module m\n@total\npub fn countdown(n: u32) -> u32\n  decreases n\n{\n  while n > 0 {\n  }\n  n\n}\n",
    ),
    reserved(
        "E0503",
        FailureClass::Proof,
        "spec/ghost-contracts-and-totality.md",
        "A contract clause outside Tier 1 is not routed",
        "A `requires`, `ensures` or `invariant` clause outside the Tier-1 fragment needs `@prove` \
         (Tier 2 proves it) or `@checked` (it becomes a runtime check). The help offers both.",
    ),
    reserved(
        "E0601",
        FailureClass::Type,
        "spec/dimensional-units.md",
        "Quantities of different units are added or compared",
        "Addition, subtraction and comparison need the same unit, scale included: \
         `5.0_ms + 1.0_sec` is an error. Convert one side explicitly with `.to<U>()`.",
    ),
    reserved(
        "E0602",
        FailureClass::Type,
        "spec/dimensional-units.md",
        "A unit conversion changes dimension",
        "`x.to<U>()` converts between units of the same dimension only: seconds to milliseconds, \
         never seconds to meters.",
    ),
    reserved(
        "E0603",
        FailureClass::Type,
        "spec/dimensional-units.md",
        "An integer conversion to a coarser unit would lose a remainder",
        "On an integer quantity, `.to<U>()` must multiply by an integer ratio. Converting `u64<ms>` \
         to `u64<sec>` would discard a remainder; divide explicitly, or convert a fixed-point or \
         floating value.",
    ),
    reserved(
        "E0701",
        FailureClass::Comptime,
        "spec/comptime-and-ffi.md",
        "A comptime evaluation exceeds its step budget",
        "Each `comptime` evaluation runs under a step budget, 10^7 steps by default, so a build \
         never hangs. `@comptime_budget(steps: N)` raises it for evaluations a function starts.",
    ),
];

/// The entry of a code, if the catalogue has one.
pub fn lookup(code: &str) -> Option<&'static CodeEntry> {
    CATALOGUE.iter().find(|e| e.code == code)
}

/// What `mn explain` prints for an entry.
pub fn explain(e: &CodeEntry) -> String {
    let status = match e.status {
        CodeStatus::Emitted => "",
        CodeStatus::Reserved => " (reserved: specified, not reported yet)",
        CodeStatus::Retired => " (retired)",
    };
    let mut out = format!(
        "{}: {}{}\n\n{}\n\nGoverned by {}. Failure class: {}.\n",
        e.code,
        e.title,
        status,
        e.explain,
        e.spec,
        e.class.name()
    );
    if e.status == CodeStatus::Emitted {
        out.push_str(&format!(
            "\nThis fails with {}:\n\n{}\nThis passes:\n\n{}",
            e.code,
            indent(e.fails),
            indent(e.passes)
        ));
    }
    out
}

fn indent(src: &str) -> String {
    src.lines().map(|l| format!("    {}\n", l)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mente_sema::check_source;
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Every `E0ddd` code a text names.
    fn codes_in(text: &str) -> BTreeSet<String> {
        let bytes = text.as_bytes();
        let mut found = BTreeSet::new();
        for i in 0..bytes.len() {
            let word_start = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let shape = bytes.len() >= i + 5
                && bytes[i] == b'E'
                && bytes[i + 1] == b'0'
                && bytes[i + 2..i + 5].iter().all(u8::is_ascii_digit);
            let word_end = bytes.get(i + 5).is_none_or(|b| !b.is_ascii_alphanumeric());
            if word_start && shape && word_end {
                found.insert(text[i..i + 5].to_string());
            }
        }
        found
    }

    #[test]
    fn test_catalogue_codes_are_unique_and_in_order() {
        let codes: Vec<&str> = CATALOGUE.iter().map(|e| e.code).collect();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(codes, sorted, "codes must be unique and in order");
        for e in CATALOGUE {
            assert_eq!(codes_in(e.code).len(), 1, "{} is not an E0ddd code", e.code);
            assert!(root().join(e.spec).is_file(), "{} names {}", e.code, e.spec);
            assert!(!e.title.is_empty() && !e.explain.is_empty(), "{}", e.code);
            let has_programs = !e.fails.is_empty() && !e.passes.is_empty();
            assert_eq!(e.status == CodeStatus::Emitted, has_programs, "{}", e.code);
        }
        assert_eq!(
            lookup("E0413").map(|e| e.title),
            Some("A @layout field leaves a gap or overlaps")
        );
        assert!(lookup("E0999").is_none());
    }

    #[test]
    fn test_each_reported_code_fails_its_program_and_passes_its_near_miss() {
        for e in CATALOGUE.iter().filter(|e| e.status == CodeStatus::Emitted) {
            let errs = check_source(e.fails).err().unwrap_or_default();
            assert_eq!(
                errs.first().map(|d| d.code),
                Some(e.code),
                "{} should fail its program, got {:?}",
                e.code,
                errs
            );
            assert!(
                check_source(e.passes).is_ok(),
                "{}: the near miss should pass, got {:?}",
                e.code,
                check_source(e.passes).err()
            );
        }
    }

    #[test]
    fn test_catalogue_covers_every_code_in_the_compiler_and_the_documents() {
        // Codes the compiler's sources spell as string literals: what it can report.
        let mut in_compiler = BTreeSet::new();
        for krate in fs::read_dir(root().join("compiler")).unwrap().flatten() {
            for file in fs::read_dir(krate.path().join("src"))
                .into_iter()
                .flatten()
                .flatten()
            {
                let path = file.path();
                if path.file_name().is_some_and(|n| n == "catalogue.rs") {
                    continue;
                }
                let text = fs::read_to_string(&path).unwrap_or_default();
                for code in codes_in(&text) {
                    if text.contains(&format!("\"{}\"", code)) {
                        in_compiler.insert(code);
                    }
                }
            }
        }
        let emitted: BTreeSet<String> = CATALOGUE
            .iter()
            .filter(|e| e.status == CodeStatus::Emitted)
            .map(|e| e.code.to_string())
            .collect();
        assert_eq!(
            in_compiler, emitted,
            "the catalogue's reported codes and the compiler's differ"
        );

        // Codes the documents name: each has an entry, reported or reserved.
        let mut docs: Vec<PathBuf> = ["README.md", "ROADMAP.md", "CLAUDE.md"]
            .iter()
            .map(|f| root().join(f))
            .collect();
        for dir in ["spec", "docs", "examples"] {
            docs.extend(
                fs::read_dir(root().join(dir))
                    .unwrap()
                    .flatten()
                    .map(|e| e.path()),
            );
        }
        for doc in docs {
            for code in codes_in(&fs::read_to_string(&doc).unwrap_or_default()) {
                assert!(
                    lookup(&code).is_some(),
                    "{} names {}, which the catalogue lacks",
                    doc.display(),
                    code
                );
            }
        }
    }

    /// The agent benchmark classifies a program that fails to build by the classes of the codes
    /// it reports (`ai-coding-problems`), so each code's class is fixed here and printed by
    /// `mn explain`. Today's codes take their class from their hundred; a code of another kind
    /// in the same hundred (a foreign idiom among the E01xx, say) is listed here when it arrives.
    #[test]
    fn test_every_code_has_a_failure_class() {
        let all = [
            FailureClass::Syntax,
            FailureClass::Provenance,
            FailureClass::HardwareContract,
            FailureClass::Proof,
            FailureClass::Type,
            FailureClass::Comptime,
        ];
        let names: Vec<&str> = all.iter().map(|c| c.name()).collect();
        assert_eq!(
            names,
            [
                "syntax",
                "provenance",
                "hardware contract",
                "proof",
                "type",
                "comptime"
            ]
        );
        for e in CATALOGUE {
            let expected = match &e.code[..3] {
                "E01" => FailureClass::Syntax,
                "E03" => FailureClass::Provenance,
                "E04" => FailureClass::HardwareContract,
                "E05" => FailureClass::Proof,
                "E06" => FailureClass::Type,
                "E07" => FailureClass::Comptime,
                other => panic!("{}: no class is expected for {}xx", e.code, other),
            };
            assert_eq!(e.class, expected, "{}", e.code);
        }
        for (code, class) in [
            ("E0311", "provenance"),
            ("E0501", "proof"),
            ("E0601", "type"),
        ] {
            let text = explain(lookup(code).unwrap());
            assert!(
                text.contains(&format!(". Failure class: {}.\n", class)),
                "{}",
                text
            );
        }
    }

    #[test]
    fn test_explain_text_shows_the_programs_of_reported_codes() {
        let text = explain(lookup("E0412").unwrap());
        assert!(
            text.starts_with("E0412: A @layout field has no @bytes range\n\n"),
            "{}",
            text
        );
        assert!(
            text.contains("Governed by spec/morphic-layouts.md."),
            "{}",
            text
        );
        assert!(
            text.contains("This fails with E0412:\n\n    module m\n"),
            "{}",
            text
        );
        assert!(text.contains("This passes:\n\n    module m\n"), "{}", text);
        let reserved = explain(lookup("E0601").unwrap());
        assert!(
            reserved.contains("(reserved: specified, not reported yet)")
                && !reserved.contains("This fails"),
            "{}",
            reserved
        );
    }
    #[test]
    fn test_every_reported_code_has_a_conformance_program_under_its_spec() {
        for e in CATALOGUE.iter().filter(|e| e.status == CodeStatus::Emitted) {
            let stem = e.spec.trim_start_matches("spec/").trim_end_matches(".md");
            let dir = root().join("conformance").join(stem);
            let expects_it = fs::read_dir(&dir).into_iter().flatten().flatten().any(|f| {
                fs::read_to_string(f.path())
                    .is_ok_and(|src| src.lines().next() == Some(&format!("// expect: {}", e.code)))
            });
            assert!(
                expects_it,
                "no program under conformance/{} expects {}",
                stem, e.code
            );
        }
    }
}
