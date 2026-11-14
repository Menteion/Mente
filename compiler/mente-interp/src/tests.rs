//! Programs run end to end: each rule of `spec/core-calculus.md` that the interpreter
//! implements, with a value it computes and a trap it raises.

use super::*;

/// A module whose names and types check. Tier 1 is left out: these programs break their
/// refinements and contracts on purpose, to see the interpreter's own checks trap.
fn module(src: &str) -> CheckedModule {
    let text = format!("module m\n{}\n", src);
    match mente_sema::check_program_without_proofs(&text) {
        Ok(m) => m,
        Err(d) => panic!("the program should check: {:?}", d),
    }
}

fn run_with(src: &str, call_text: &str, limits: Limits) -> Result<Value, Stop> {
    let Some((name, args)) = parse_call(call_text) else {
        panic!("not a call: {}", call_text);
    };
    call(&module(src), &name, args, limits)
}

/// The value a call returns, or its stop as `mn run` prints it.
fn run(src: &str, call_text: &str) -> String {
    match run_with(src, call_text, Limits::default()) {
        Ok(v) => v.to_string(),
        Err(s) => s.to_string(),
    }
}

/// A run shows its observer each integer an expression gives, exact and with the expression's
/// text, in the order of evaluation and up to the trap that ends the run, and the value a
/// compound assignment computes, with the assignment's text, before it is narrowed into its
/// place; a run nobody observes gives the same result.
#[test]
fn test_an_observer_sees_each_integer_an_expression_gives() {
    let src = "pub struct P {\n  a: u8\n}\npub fn f(x: u8, c: bool) -> u8 {\n  val y: u8 = x + 250\n  if c { y } else { 7 }\n}\npub fn g(x: u16, p: P) -> [2][1]u16 {\n  val q = P(a: p.a)\n  _ = q\n  [[x], [300]]\n}\npub fn h(x: u8) -> u8 {\n  var v: u8 = x\n  v += 250\n  v = 3\n  v\n}";
    let text = format!("module m\n{}\n", src);
    let m = module(src);
    let seen = |call_text: &str| {
        let (name, args) = parse_call(call_text).unwrap();
        let mut values: Vec<(String, String)> = Vec::new();
        let observed = call_observed(
            &m,
            &name,
            args.clone(),
            Limits::default(),
            &mut |span: Span, x: Int| {
                values.push((text[span.start..span.end].to_string(), x.to_string()));
            },
        );
        let result = |r: Result<(Value, Vec<Value>), Stop>| match r {
            Ok((v, _)) => v.to_string(),
            Err(s) => s.to_string(),
        };
        let alone = result(call_leaving(&m, &name, args, Limits::default()));
        assert_eq!(result(observed), alone, "{}", call_text);
        (values, alone)
    };
    let pairs = |all: &[(&str, &str)]| -> Vec<(String, String)> {
        all.iter()
            .map(|(e, v)| (e.to_string(), v.to_string()))
            .collect()
    };
    assert_eq!(
        seen("f(1, true)"),
        (
            pairs(&[
                ("x", "1"),
                ("250", "250"),
                ("x + 250", "251"),
                ("y", "251"),
                ("if c { y } else { 7 }", "251"),
            ]),
            "251".to_string()
        )
    );
    // A compound assignment shows its right side, then the sum, and a plain one its value
    // alone; the sum that does not fit its place is shown before the trap.
    assert_eq!(
        seen("h(5)"),
        (
            pairs(&[
                ("x", "5"),
                ("250", "250"),
                ("v += 250", "255"),
                ("3", "3"),
                ("v", "3"),
            ]),
            "3".to_string()
        )
    );
    assert_eq!(
        seen("h(6)").0,
        pairs(&[("x", "6"), ("250", "250"), ("v += 250", "256")])
    );
    assert_eq!(
        seen("f(1, false)").0,
        pairs(&[
            ("x", "1"),
            ("250", "250"),
            ("x + 250", "251"),
            ("7", "7"),
            ("if c { y } else { 7 }", "7"),
        ])
    );
    // The sum is seen as it is, 256, before the narrowing into `u8` that traps on it.
    assert_eq!(
        seen("f(6, true)"),
        (
            pairs(&[("x", "6"), ("250", "250"), ("x + 250", "256")]),
            "trap[narrowing] at 6:15: 256 does not fit in `u8`".to_string()
        )
    );
    // An array shows its integers, through the arrays nested in it; a struct shows none, its
    // field having been shown where it was read.
    assert_eq!(
        seen("g(4, P(a: 9))").0,
        pairs(&[
            ("p.a", "9"),
            ("x", "4"),
            ("[x]", "4"),
            ("300", "300"),
            ("[300]", "300"),
            ("[[x], [300]]", "4"),
            ("[[x], [300]]", "300"),
        ])
    );
}

/// The kind of trap a call raises and where, or the value it returns.
fn trapped(src: &str, call_text: &str) -> (String, u32, u32) {
    match run_with(src, call_text, Limits::default()) {
        Err(Stop::Trap(t)) => (t.kind.name().to_string(), t.span.line, t.span.col),
        Ok(v) => (v.to_string(), 0, 0),
        Err(s) => (s.to_string(), 0, 0),
    }
}

fn kind(src: &str, call_text: &str) -> String {
    trapped(src, call_text).0
}

#[test]
fn test_integer_arithmetic_is_exact_and_narrowed_where_it_lands() {
    let mid = "pub fn mid(lo: u64, hi: u64) -> u64 = (lo + hi) / 2";
    assert_eq!(
        run(mid, "mid(18446744073709551614, 18446744073709551615)"),
        "18446744073709551614"
    );
    let sum = "pub fn sum(a: u8, b: u8) -> u16 {\n  val s = a + b\n  s\n}";
    assert_eq!(
        trapped(sum, "sum(200, 100)"),
        ("narrowing".to_string(), 3, 11)
    );
    assert_eq!(run(sum, "sum(200, 55)"), "255");
    let wide = "pub fn sum(a: u8, b: u8) -> u16 {\n  val s: u16 = a + b\n  s\n}";
    assert_eq!(run(wide, "sum(200, 100)"), "300");
    let back = "pub fn back(a: u16) -> u8 = a";
    assert_eq!(trapped(back, "back(300)"), ("narrowing".to_string(), 2, 29));
    assert_eq!(
        run(back, "back(300)"),
        "trap[narrowing] at 2:29: 300 does not fit in `u8`"
    );
    let ret = "pub fn ret(a: u16) -> u8 {\n  return a\n}";
    assert_eq!(trapped(ret, "ret(256)"), ("narrowing".to_string(), 3, 10));
    let neg = "pub fn neg(a: i8) -> i16 = -a";
    assert_eq!(run(neg, "neg(-128)"), "128");
    let cmp = "pub fn below(a: u8, b: i64) -> bool = a < b";
    assert_eq!(run(cmp, "below(200, -1)"), "false");
    assert_eq!(run(cmp, "below(2, 3)"), "true");
    let ops = "pub fn ops(a: i32, b: i32) -> [8]bool = [a <= b, a > b, a >= b, a == b, a != b, a < b, b <= a, b >= a]";
    assert_eq!(
        run(ops, "ops(3, 3)"),
        "[true, false, true, true, false, false, true, true]"
    );
    assert_eq!(
        run(ops, "ops(2, 3)"),
        "[true, false, false, false, true, true, false, true]"
    );
}

#[test]
fn test_division_truncates_and_a_zero_divisor_traps() {
    let div = "pub fn div(a: i32, b: i32) -> i32 = a / b";
    let rem = "pub fn rem(a: i32, b: i32) -> i32 = a % b";
    assert_eq!(run(div, "div(-7, 2)"), "-3");
    assert_eq!(run(rem, "rem(-7, 2)"), "-1");
    assert_eq!(run(rem, "rem(7, -2)"), "1");
    assert_eq!(
        trapped(div, "div(1, 0)"),
        ("division-by-zero".to_string(), 2, 37)
    );
    assert_eq!(kind(rem, "rem(1, 0)"), "division-by-zero");
    let mul = "pub fn mul(a: i32, b: i32) -> i64 = a * b - b";
    assert_eq!(run(mul, "mul(-4, 6)"), "-30");
}

#[test]
fn test_wide_arithmetic_is_checked_at_each_operator() {
    let wide = "pub fn f(a: u128, b: u128) -> u128 = a + b - b";
    assert_eq!(
        trapped(wide, "f(340282366920938463463374607431768211455, 1)"),
        ("narrowing".to_string(), 2, 38)
    );
    assert_eq!(
        run(wide, "f(340282366920938463463374607431768211455, 1)"),
        "trap[narrowing] at 2:38: the result does not fit in `u128`"
    );
    assert_eq!(run(wide, "f(5, 1)"), "5");
    let fits = "pub fn k(a: u128, b: u128) -> u128 = a - b + b";
    assert_eq!(
        run(fits, "k(1, 2)"),
        "trap[narrowing] at 2:38: -1 does not fit in `u128`"
    );
    let signed = "pub fn g(a: i128) -> i128 = a - 1 + 1";
    assert_eq!(
        kind(signed, "g(-170141183460469231731687303715884105728)"),
        "narrowing"
    );
    // `a * a * a` might leave 128 bits, which the checker rejects (`E0222`); bound to a
    // `u128`, the square makes the next product an operation on a `u128`, checked there.
    let cube = "pub fn h(a: u64) -> u128 {\n  val product: u128 = a * a\n  product * a\n}";
    assert_eq!(
        trapped(cube, "h(18446744073709551615)"),
        ("narrowing".to_string(), 4, 3)
    );
    assert_eq!(run(cube, "h(1000)"), "1000000000");
    // A negation of an `i128` is checked at the operator, before its value lands anywhere.
    let neg = "pub fn n(a: i128) -> i8 = -a";
    assert_eq!(
        run(neg, "n(-170141183460469231731687303715884105728)"),
        "trap[narrowing] at 2:27: 170141183460469231731687303715884105728 does not fit in `i128`"
    );
    assert_eq!(run(neg, "n(5)"), "-5");
    // A literal reaches `i128`'s smallest value right after its unary `-`.
    assert_eq!(
        run(
            "pub fn m() -> i128 = -170141183460469231731687303715884105728",
            "m()"
        ),
        "-170141183460469231731687303715884105728"
    );
}

#[test]
fn test_array_literal_elements_are_narrowed_into_its_element_type() {
    // The literal's element type comes from its elements, `u64`, before it lands in `[2]i8`.
    let src = "pub fn f(a: u64) -> [2]i8 = [a, 0 - a]";
    assert_eq!(
        run(src, "f(1)"),
        "trap[narrowing] at 2:29: -1 does not fit in `u64`"
    );
    assert_eq!(run(src, "f(0)"), "[0, 0]");
    let repeat = "pub fn g(a: i16) -> [2]u8 = [a - 1; 2]";
    assert_eq!(
        run(repeat, "g(0)"),
        "trap[narrowing] at 2:29: -1 does not fit in `u8`"
    );
    assert_eq!(run(repeat, "g(3)"), "[2, 2]");
    // Elements of no one integer type stay exact until the array lands.
    let mixed = "pub fn q(a: i32, b: u64) -> [2]i64 = [a, b]";
    assert_eq!(run(mixed, "q(-1, 5)"), "[-1, 5]");
    assert_eq!(
        run(mixed, "q(-1, 18446744073709551615)"),
        "trap[narrowing] at 2:38: 18446744073709551615 does not fit in `i64`"
    );
}

/// A bit operation, a shift or `~` over literals alone works in the type its literals take
/// (`core-calculus.md`, item 5): the other operands', through arithmetic, the branches of an
/// `if`, the arms of a `match` and the elements of an array, and `x`'s in `x op= e`;
/// otherwise `i64`, whatever place the value is narrowed into, where the `i64` value is then
/// narrowed as any value is.
#[test]
fn test_literals_under_a_bit_operation_take_their_operands_type_only() {
    let decls = "struct P {\n  m: u8\n}\nfn mask(m: u8) -> u8 = m\n";
    for (f, call, value) in [
        // The other operands, though the place is wider.
        ("pub fn f(a: u8) -> u8 = a & ~3", "f(255)", "252"),
        ("pub fn f(a: u8) -> u16 = a & ~3", "f(255)", "252"),
        ("pub fn f(a: u8) -> u16 = a + ~3", "f(1)", "253"),
        ("pub fn f(a: u8) -> u16 = (1 + ~3) + a", "f(1)", "254"),
        (
            "pub fn f(a: u8, b: u16) -> u32 = a + b + ~3",
            "f(1, 1)",
            "65534",
        ),
        (
            "pub fn f(a: u8, c: bool) -> u16 = if c { ~3 } else { a }",
            "f(1, true)",
            "252",
        ),
        (
            "pub fn f(a: u8) -> u16 = match a {\n  0 => {\n    ~3\n  }\n  _ => a\n}",
            "f(0)",
            "252",
        ),
        ("pub fn f(a: u8) -> [2]u16 = [~3, a]", "f(7)", "[252, 7]"),
        (
            "pub fn f(a: u8) -> [2][1]u16 = [[~3], [a]]",
            "f(7)",
            "[[252], [7]]",
        ),
        (
            "pub fn f(c: bool, xs: [1]u8) -> [1]u8 = if c { [~3] } else { xs }",
            "f(true, [0])",
            "[252]",
        ),
        ("pub fn f(a: i128) -> i128 = a ^ ~0", "f(5)", "-6"),
        (
            "pub fn f(a: u8) -> u8 {\n  var m: u8 = a\n  m &= ~1\n  m\n}",
            "f(255)",
            "254",
        ),
        (
            "pub fn f(a: u16) -> u16 {\n  var m: u16 = a\n  m ^= ~0\n  m\n}",
            "f(1)",
            "65534",
        ),
        // No other operand: an `i64`, narrowed where it lands.
        (
            "pub fn f() -> u8 {\n  val m: u8 = 3 & 5\n  m\n}",
            "f()",
            "1",
        ),
        (
            "pub fn f() -> u16 {\n  val m: u16 = 1 + ~3\n  m\n}",
            "f()",
            "trap",
        ),
        (
            "pub fn f() -> u8 {\n  val m: u8 = -1 & 255\n  m\n}",
            "f()",
            "255",
        ),
        ("pub fn f() -> u8 = mask(1 << 7)", "f()", "128"),
        ("pub fn f() -> i8 = P(m: ~3).m", "f()", "trap"),
        (
            "pub fn f(c: bool) -> u8 {\n  val m: u8 = if c { ~3 } else { 4 }\n  m\n}",
            "f(true)",
            "trap",
        ),
        (
            "pub fn f(c: bool) -> u8 {\n  val m: u8 = if c { ~3 } else { 4 }\n  m\n}",
            "f(false)",
            "4",
        ),
        ("pub fn f() -> i8 = ~127", "f()", "-128"),
        ("pub fn f() -> i64 {\n  val m = ~3\n  m\n}", "f()", "-4"),
        (
            "pub fn f() -> i64 {\n  val xs = [~3, 1]\n  xs[0]\n}",
            "f()",
            "-4",
        ),
        ("pub fn f(a: u8) -> bool = a == ~3", "f(252)", "false"),
        (
            "pub fn f(xs: [8]u8) -> u8 = xs[~3 & 7]",
            "f([1, 2, 3, 4, 5, 6, 7, 8])",
            "5",
        ),
        ("pub fn f(a: u8) -> u8 = a << (~3 & 1)", "f(9)", "9"),
        (
            "pub fn f() -> u8 = match ~3 {\n  -4 => 1\n  _ => 2\n}",
            "f()",
            "1",
        ),
        ("pub fn f() -> u8 = ~3_u8", "f()", "252"),
        (
            "pub fn f() -> u64 = 1_u64 << 63",
            "f()",
            "9223372036854775808",
        ),
    ] {
        let src = format!("{}{}", decls, f);
        let got = match run_with(&src, call, Limits::default()) {
            Ok(v) => v.to_string(),
            Err(Stop::Trap(_)) => "trap".to_string(),
            Err(other) => other.to_string(),
        };
        assert_eq!(got, value, "{}", f);
    }
    // A literal outside `i64`, or shifted past it, traps where the operation narrows it into
    // `i64`, before the value reaches its place.
    for (f, trap) in [
        ("pub fn f() -> u64 = 1 << 63", "narrowing"),
        ("pub fn f() -> u64 = 1 | 9223372036854775808", "narrowing"),
        ("pub fn f() -> u128 = 1 << 64", "shift-amount"),
        ("pub fn f() -> u8 = ~3", "narrowing"),
        ("pub fn f() -> u16 = 1 << 15", "none"),
    ] {
        let got = match run_with(f, "f()", Limits::default()) {
            Ok(_) => "none".to_string(),
            Err(Stop::Trap(t)) => t.kind.name().to_string(),
            Err(other) => other.to_string(),
        };
        assert_eq!(got, trap, "{}", f);
    }
    let shifted = run("pub fn f() -> u64 = 1 << 63", "f()");
    assert!(shifted.contains("does not fit in `i64`"), "{}", shifted);
}

#[test]
fn test_bits_and_shifts_work_in_their_type() {
    let not = "pub fn not8(x: u8) -> u8 = ~x";
    assert_eq!(run(not, "not8(5)"), "250");
    let inot = "pub fn noti(x: i8) -> i8 = ~x";
    assert_eq!(run(inot, "noti(0)"), "-1");
    let bits = "pub fn bits(a: u8, b: u8) -> [3]u8 = [a & b, a | b, a ^ b]";
    assert_eq!(run(bits, "bits(12, 10)"), "[8, 14, 6]");
    let signed = "pub fn sbits(a: i8, b: i8) -> [3]i8 = [a & b, a | b, a ^ b]";
    assert_eq!(run(signed, "sbits(-1, 5)"), "[5, -1, -6]");
    let operand = "pub fn low(a: u8, b: u8) -> u8 = (a + b) & 1";
    assert_eq!(
        trapped(operand, "low(200, 200)"),
        ("narrowing".to_string(), 2, 35)
    );
    assert_eq!(run(operand, "low(2, 3)"), "1");
    let right = "pub fn rhs(a: u8, b: u8) -> u8 = 1 & (a + b)";
    assert_eq!(kind(right, "rhs(200, 200)"), "narrowing");
    let shl = "pub fn shl(x: u8, k: u8) -> u8 = x << k";
    assert_eq!(run(shl, "shl(1, 7)"), "128");
    assert_eq!(
        trapped(shl, "shl(1, 8)"),
        ("shift-amount".to_string(), 2, 34)
    );
    assert_eq!(
        run(shl, "shl(1, 8)"),
        "trap[shift-amount] at 2:34: shift amount 8 is outside 0 ..= 7"
    );
    assert_eq!(kind(shl, "shl(200, 1)"), "narrowing");
    let shr = "pub fn shr(x: i8, k: i8) -> i8 = x >> k";
    assert_eq!(run(shr, "shr(-8, 1)"), "-4");
    assert_eq!(run(shr, "shr(-7, 1)"), "-4");
    assert_eq!(kind(shr, "shr(8, -1)"), "shift-amount");
    // Literals alone are an `i64`, whatever place they land in (`core-calculus.md`, item 5):
    // `1 << 7` is 128, `~0` is -1 unless a suffix states `u8`, and `1 << 8` is 256, which the
    // binding narrows.
    let lit =
        "pub fn lit() -> u8 {\n  val m: u8 = 1 << 7\n  val k: u8 = ~0_u8\n  m - 1 + k - 255\n}";
    assert_eq!(run(lit, "lit()"), "127");
    let minus = "pub fn minus() -> u8 {\n  val k: u8 = ~0\n  k\n}";
    assert_eq!(
        run(minus, "minus()"),
        "trap[narrowing] at 3:15: -1 does not fit in `u8`"
    );
    let too_far = "pub fn far() -> u8 {\n  val m: u8 = 1 << 8\n  m\n}";
    assert_eq!(kind(too_far, "far()"), "narrowing");
    let wide = "pub fn wide(k: u8) -> i64 = 1 << k";
    assert_eq!(run(wide, "wide(40)"), "1099511627776");
    let huge = "pub fn huge(x: u128) -> u128 = x << 127";
    assert_eq!(kind(huge, "huge(2)"), "narrowing");
}

#[test]
fn test_logic_short_circuits_and_branches_choose() {
    let and = "pub fn get(xs: [2]u8, i: usize) -> bool = i < 2 and xs[i] > 0";
    assert_eq!(run(and, "get([1, 2], 5)"), "false");
    assert_eq!(run(and, "get([1, 2], 1)"), "true");
    let or = "pub fn get(xs: [2]u8, i: usize) -> bool = i >= 2 or xs[i] > 0";
    assert_eq!(run(or, "get([1, 0], 5)"), "true");
    assert_eq!(run(or, "get([1, 0], 1)"), "false");
    let not = "pub fn inv(c: bool) -> bool = not c";
    assert_eq!(run(not, "inv(true)"), "false");
    let sign = "pub fn sign(x: i32) -> i32 = if x < 0 { -1 } else if x == 0 { 0 } else { 1 }";
    assert_eq!(run(sign, "sign(-5)"), "-1");
    assert_eq!(run(sign, "sign(0)"), "0");
    assert_eq!(run(sign, "sign(9)"), "1");
    let guard = "pub fn pos(x: i32) -> i32 {\n  guard x > 0 else {\n    return 0\n  }\n  x\n}";
    assert_eq!(run(guard, "pos(-3)"), "0");
    assert_eq!(run(guard, "pos(3)"), "3");
    let unit = "pub fn nothing(x: i32) {\n  if x > 0 {\n    _ = x\n  }\n}";
    assert_eq!(run(unit, "nothing(1)"), "()");
    assert_eq!(run(unit, "nothing(-1)"), "()");
}

#[test]
fn test_loops_repeat_break_and_continue() {
    let sum = "pub fn sum(n: u32) -> u64 {\n  var total: u64 = 0\n  var i: u32 = 0\n  while i < n {\n    i += 1\n    if i % 2 == 0 {\n      continue\n    }\n    if i > 7 {\n      break\n    }\n    total += i\n  }\n  total\n}";
    assert_eq!(run(sum, "sum(5)"), "9");
    assert_eq!(run(sum, "sum(100)"), "16");
    assert_eq!(run(sum, "sum(0)"), "0");
    let early = "pub fn find(xs: [4]i32, x: i32) -> i32 {\n  var i: usize = 0\n  while i < 4 {\n    if xs[i] == x {\n      return i\n    }\n    i += 1\n  }\n  -1\n}";
    assert_eq!(run(early, "find([5, 6, 7, 8], 7)"), "2");
    assert_eq!(run(early, "find([5, 6, 7, 8], 9)"), "-1");
}

#[test]
fn test_arrays_are_values_indexed_within_their_length() {
    let copy =
        "pub fn copy() -> i32 {\n  val a = [1, 2]\n  var b = a\n  b[0] = 9\n  a[0] * 10 + b[0]\n}";
    assert_eq!(run(copy, "copy()"), "19");
    let read = "pub fn at(xs: [3]u8, i: usize) -> u8 = xs[i]";
    assert_eq!(run(read, "at([4, 5, 6], 2)"), "6");
    assert_eq!(
        trapped(read, "at([4, 5, 6], 3)"),
        ("index-out-of-bounds".to_string(), 2, 40)
    );
    assert_eq!(
        run(read, "at([4, 5, 6], 3)"),
        "trap[index-out-of-bounds] at 2:40: index 3 is outside an array of length 3"
    );
    let signed = "pub fn at(xs: [3]u8, i: i32) -> u8 = xs[i]";
    assert_eq!(kind(signed, "at([4, 5, 6], -1)"), "index-out-of-bounds");
    let store = "pub fn put(i: usize) -> [2]u8 {\n  var xs: [2]u8 = [0, 0]\n  xs[i] = 7\n  xs\n}";
    assert_eq!(run(store, "put(1)"), "[0, 7]");
    assert_eq!(
        trapped(store, "put(2)"),
        ("index-out-of-bounds".to_string(), 4, 3)
    );
    let grid = "pub fn grid(i: usize, j: usize) -> [2][2]u8 {\n  var g: [2][2]u8 = [[0, 0], [0, 0]]\n  g[i][j] += 5\n  g\n}";
    assert_eq!(run(grid, "grid(1, 0)"), "[[0, 0], [5, 0]]");
    assert_eq!(kind(grid, "grid(0, 2)"), "index-out-of-bounds");
    let elem = "pub fn big() -> [2]u8 = [1, 300]";
    assert_eq!(kind(elem, "big()"), "narrowing");
    let len = "pub fn len(xs: [5]bool) -> usize = xs.len";
    assert_eq!(run(len, "len([true, true, false, true, true])"), "5");
    let rep = "pub fn rep(a: u8) -> [3]u8 = [a; 3]";
    assert_eq!(run(rep, "rep(4)"), "[4, 4, 4]");
    let over = "pub fn over() -> u8 {\n  var x: u8 = 250\n  x += 10\n  x\n}";
    assert_eq!(trapped(over, "over()"), ("narrowing".to_string(), 4, 8));
}

#[test]
fn test_mut_parameters_copy_back_into_the_place_passed() {
    let calls = "pub fn bump(mut x: u8) {\n  x += 1\n}\npub fn early(mut x: u8, stop: bool) {\n  x = 9\n  if stop {\n    return\n  }\n  x = 1\n}\npub fn twice(mut xs: [2]u8, v: u8) -> u8 {\n  xs[0] = v\n  xs[1] = v\n  v\n}";
    let main = format!(
        "{}\npub fn main() -> [5]u8 {{\n  var v: u8 = 1\n  bump(mut v)\n  var xs: [2]u8 = [0, 0]\n  bump(mut xs[1])\n  var e: u8 = 0\n  early(mut e, true)\n  var f: u8 = 0\n  early(mut f, false)\n  var ys: [2]u8 = [0, 0]\n  _ = twice(mut ys, 3)\n  [v, xs[1], e, f, ys[0] + ys[1]]\n}}",
        calls
    );
    assert_eq!(run(&main, "main()"), "[2, 1, 9, 1, 6]");
    let index = format!(
        "{}\npub fn main(i: usize) -> u8 {{\n  var xs: [2]u8 = [0, 0]\n  bump(mut xs[i])\n  xs[0]\n}}",
        calls
    );
    assert_eq!(kind(&index, "main(2)"), "index-out-of-bounds");
    assert_eq!(run(&index, "main(0)"), "1");
    let overflow = format!(
        "{}\npub fn main() -> u8 {{\n  var v: u8 = 255\n  bump(mut v)\n  v\n}}",
        calls
    );
    assert_eq!(
        trapped(&overflow, "main()"),
        ("narrowing".to_string(), 3, 8)
    );
}

#[test]
fn test_calls_recurse_until_the_stack_runs_out() {
    let fact =
        "pub fn fact(n: u64) -> u64 {\n  if n == 0 {\n    return 1\n  }\n  n * fact(n - 1)\n}";
    assert_eq!(run(fact, "fact(20)"), "2432902008176640000");
    assert_eq!(kind(fact, "fact(21)"), "narrowing");
    let limits = Limits {
        max_depth: 50,
        ..Limits::default()
    };
    let down = "pub fn down(n: u32) -> u32 {\n  if n == 0 {\n    return 0\n  }\n  down(n - 1)\n}";
    let deep = run_with(down, "down(100)", limits);
    assert!(
        matches!(&deep, Err(Stop::Trap(t)) if t.kind == TrapKind::StackOverflow && t.span.line == 6),
        "{:?}",
        deep
    );
    assert_eq!(
        run_with(down, "down(40)", limits),
        Ok(Value::Int(Int::ZERO))
    );
    let args = "pub fn pick(a: u8, b: u8) -> u8 = b\npub fn f(x: u16) -> u8 = pick(1, x)";
    assert_eq!(trapped(args, "f(300)"), ("narrowing".to_string(), 3, 34));
    assert_eq!(run(args, "f(30)"), "30");
}

#[test]
fn test_todo_and_unreachable_trap_with_their_reasons() {
    let todo = "pub fn later() -> u8 = todo(\"not yet\")";
    assert_eq!(trapped(todo, "later()"), ("todo".to_string(), 2, 24));
    assert_eq!(
        run(todo, "later()"),
        "trap[todo] at 2:24: todo reached: not yet"
    );
    let never = "pub fn never(x: u8) -> u8 {\n  guard x < 10 else {\n    unreachable(\"small\")\n  }\n  x\n}";
    assert_eq!(
        run(never, "never(12)"),
        "trap[unreachable] at 4:5: unreachable reached: small"
    );
    assert_eq!(run(never, "never(2)"), "2");
}

#[test]
fn test_refinements_are_checked_where_values_enter_them() {
    let param = "pub fn f(i: usize[0..4]) -> usize = i";
    assert_eq!(run(param, "f(3)"), "3");
    assert_eq!(
        run(param, "f(4)"),
        "trap[refinement] at 2:8: 4 lies outside the refinement `[0..4]`"
    );
    let named = "pub fn g(n: usize, i: usize[0..n]) -> usize = i";
    assert_eq!(kind(named, "g(3, 3)"), "refinement");
    assert_eq!(run(named, "g(3, 2)"), "2");
    let inclusive = "pub fn h(i: i8[-2..=2]) -> i8 = i";
    assert_eq!(run(inclusive, "h(2)"), "2");
    assert_eq!(run(inclusive, "h(-2)"), "-2");
    assert_eq!(kind(inclusive, "h(3)"), "refinement");
    assert_eq!(kind(inclusive, "h(-3)"), "refinement");
    let result = "pub fn r(x: u8) -> u8[0..10] = x";
    assert_eq!(kind(result, "r(10)"), "refinement");
    let binding = "pub fn b(xs: [3]u8, x: usize) -> usize {\n  val i: usize[0..xs.len] = x\n  i\n}";
    assert_eq!(
        trapped(binding, "b([1, 2, 3], 3)"),
        ("refinement".to_string(), 3, 29)
    );
    assert_eq!(run(binding, "b([1, 2, 3], 2)"), "2");
    let store = "pub fn s(v: u8) -> [2]u8 {\n  var xs: [2]u8[0..5] = [0, 0]\n  xs[1] = v\n  xs\n}";
    assert_eq!(kind(store, "s(5)"), "refinement");
    assert_eq!(run(store, "s(4)"), "[0, 4]");
    let elems = "pub fn e(v: u8) -> [2]u8[0..5] = [v, 0]";
    assert_eq!(kind(elems, "e(9)"), "refinement");
}

#[test]
fn test_contracts_are_checked_at_calls_returns_and_loop_heads() {
    let req = "pub fn half(x: u8) -> u8\n  requires x % 2 == 0\n= x / 2\npub fn main(x: u8) -> u8 = half(x)";
    assert_eq!(run(req, "main(4)"), "2");
    assert_eq!(
        run(req, "main(3)"),
        "trap[requires] at 5:28: `requires (== (% x 2) 0)` is false at this call"
    );
    let ens = "pub fn inc(x: u8) -> u8\n  ensures result > x\n{\n  if x == 9 {\n    return x\n  }\n  x + 1\n}";
    assert_eq!(run(ens, "inc(3)"), "4");
    assert_eq!(trapped(ens, "inc(9)"), ("ensures".to_string(), 6, 12));
    let inv = "pub fn count(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n\n    invariant i <= 5\n  {\n    i += 1\n  }\n  i\n}";
    assert_eq!(run(inv, "count(5)"), "5");
    assert_eq!(trapped(inv, "count(7)"), ("invariant".to_string(), 5, 15));
    let dec = "pub fn spin(n: u8, step: u8) -> u8 {\n  var i: u8 = n\n  while i > 0\n    decreases i\n  {\n    i -= step\n  }\n  i\n}";
    assert_eq!(run(dec, "spin(4, 1)"), "0");
    assert_eq!(trapped(dec, "spin(4, 0)"), ("decreases".to_string(), 5, 15));
    let negative = "pub fn neg(n: i8) -> i8 {\n  var i: i8 = n\n  while i != 0\n    decreases i\n  {\n    i -= 1\n  }\n  i\n}";
    assert_eq!(kind(negative, "neg(-2)"), "decreases");
    assert_eq!(run(negative, "neg(2)"), "0");
    let rec = "pub fn r(n: u8, step: u8) -> u8\n  decreases n\n{\n  if n == 0 {\n    return 0\n  }\n  r(n - step, step)\n}";
    assert_eq!(run(rec, "r(4, 1)"), "0");
    assert_eq!(trapped(rec, "r(4, 0)"), ("decreases".to_string(), 8, 3));
    let other =
        "pub fn m(n: u8) -> u8\n  decreases n\n= n\npub fn caller(n: u8) -> u8 = m(n) + m(n)";
    assert_eq!(run(other, "caller(3)"), "6");
    let neg_rec = "pub fn nr(n: i8) -> i8\n  decreases n\n= n";
    assert_eq!(kind(neg_rec, "nr(-1)"), "decreases");
}

#[test]
fn test_runs_stop_at_their_limits() {
    let spin = "pub fn spin() -> u8 {\n  var i: u8 = 0\n  while true {\n    i = 1 - i\n  }\n  i\n}";
    let limits = Limits {
        max_steps: 1000,
        ..Limits::default()
    };
    assert_eq!(run_with(spin, "spin()", limits), Err(Stop::OutOfSteps));
    let line = "pub fn line(n: u8) -> u8 {\n  val a = n\n  val b = a\n  b\n}";
    let tight = Limits {
        max_steps: 3,
        ..Limits::default()
    };
    assert_eq!(
        run_with(line, "line(1)", tight),
        Ok(Value::Int(Int::from_u128(1)))
    );
    let tighter = Limits {
        max_steps: 2,
        ..Limits::default()
    };
    assert_eq!(run_with(line, "line(1)", tighter), Err(Stop::OutOfSteps));
    let big = "pub fn big() -> [100]u8 = [0; 100]";
    let small = Limits {
        max_array: 99,
        ..Limits::default()
    };
    assert_eq!(run_with(big, "big()", small), Err(Stop::TooLarge(100)));
    let exact = Limits {
        max_array: 100,
        ..Limits::default()
    };
    assert!(run_with(big, "big()", exact).is_ok());
}

#[test]
fn test_calls_from_outside_name_a_function_and_its_arguments() {
    let src = "pub fn id(x: u8) -> u8 = x\npub fn declared(x: u8) -> u8";
    assert_eq!(
        run(src, "missing(1)"),
        "no function `missing` with a body to run"
    );
    assert_eq!(
        run(src, "declared(1)"),
        "no function `declared` with a body to run"
    );
    assert_eq!(
        run(src, "id(1, 2)"),
        "`id` takes 1 arguments, but 2 were given"
    );
    assert_eq!(
        run(src, "id(300)"),
        "trap[narrowing] at 2:8: 300 does not fit in `u8`"
    );
    let main = "pub fn main() -> bool = true";
    assert_eq!(
        run_main(&module(main), Limits::default()),
        Ok(Value::Bool(true))
    );
}

#[test]
fn test_calls_and_values_read_and_print_canonically() {
    assert_eq!(
        parse_call(" f ( 1, -2, true, false, [1, [2, 3]], [] ) "),
        Some((
            "f".to_string(),
            vec![
                Value::Int(Int::from_u128(1)),
                Value::Int(Int::from_i128(-2)),
                Value::Bool(true),
                Value::Bool(false),
                Value::Array(vec![
                    Value::Int(Int::from_u128(1)),
                    Value::Array(vec![
                        Value::Int(Int::from_u128(2)),
                        Value::Int(Int::from_u128(3))
                    ])
                ]),
                Value::Array(Vec::new()),
            ]
        ))
    );
    assert_eq!(parse_call("g()"), Some(("g".to_string(), Vec::new())));
    assert_eq!(
        parse_call("h(1_000)"),
        Some(("h".to_string(), vec![Value::Int(Int::from_u128(1000))]))
    );
    for bad in [
        "g", "g(1", "1g(1)", "g(1 2)", "g(tru)", "g([1 2])", "g(1,)", "(1)", "g-h(1)", "g(x)",
    ] {
        assert_eq!(parse_call(bad), None, "{}", bad);
    }
    assert_eq!(Value::Unit.to_string(), "()");
    assert_eq!(
        Value::Array(vec![Value::Bool(true), Value::Int(Int::from_i128(-1))]).to_string(),
        "[true, -1]"
    );
    let kinds = [
        (
            TrapKind::Narrowing {
                value: Some(Int::from_u128(9)),
                into: "u8",
            },
            "narrowing",
        ),
        (
            TrapKind::Refinement {
                value: Int::ZERO,
                range: String::new(),
            },
            "refinement",
        ),
        (TrapKind::DivisionByZero, "division-by-zero"),
        (
            TrapKind::IndexOutOfBounds {
                index: Int::ZERO,
                len: 0,
            },
            "index-out-of-bounds",
        ),
        (
            TrapKind::ShiftAmount {
                amount: Int::ZERO,
                width: 8,
            },
            "shift-amount",
        ),
        (TrapKind::Todo(String::new()), "todo"),
        (TrapKind::Unreachable(String::new()), "unreachable"),
        (TrapKind::Requires(String::new()), "requires"),
        (TrapKind::Ensures(String::new()), "ensures"),
        (TrapKind::Invariant(String::new()), "invariant"),
        (TrapKind::Decreases(String::new()), "decreases"),
        (TrapKind::StackOverflow, "stack-overflow"),
        (TrapKind::IntermediateOverflow, "intermediate-overflow"),
    ];
    for (k, name) in kinds {
        assert_eq!(k.name(), name);
    }
    assert_eq!(TrapKind::DivisionByZero.to_string(), "division by zero");
    assert_eq!(
        TrapKind::StackOverflow.to_string(),
        "the call stack ran out"
    );
    assert_eq!(
        TrapKind::IntermediateOverflow.to_string(),
        "an intermediate result needs more than 128 bits"
    );
    assert_eq!(
        TrapKind::Invariant("x".to_string()).to_string(),
        "`invariant x` is false at the loop head"
    );
    assert_eq!(
        TrapKind::Decreases("x".to_string()).to_string(),
        "`decreases x` is negative or did not decrease"
    );
    assert_eq!(
        TrapKind::Ensures("x".to_string()).to_string(),
        "`ensures x` is false at this return"
    );
    assert_eq!(
        Stop::OutOfSteps.to_string(),
        "the run took more steps than its limit"
    );
    assert_eq!(
        Stop::TooLarge(5).to_string(),
        "an array of 5 elements is over the limit"
    );
    assert_eq!(Limits::default().max_depth, 10_000);
}

#[test]
fn test_calls_from_outside_mark_mut_arguments_and_see_what_they_leave() {
    assert_eq!(
        parse_call_modes("rev(mut [1, 2], 3, mut 4)"),
        Some((
            "rev".to_string(),
            vec![
                (
                    true,
                    Value::Array(vec![
                        Value::Int(Int::from_u128(1)),
                        Value::Int(Int::from_u128(2))
                    ])
                ),
                (false, Value::Int(Int::from_u128(3))),
                (true, Value::Int(Int::from_u128(4))),
            ]
        ))
    );
    assert_eq!(parse_call_modes("f(mutable)"), None);
    let src = "pub fn swap(mut xs: [2]u8, k: u8) -> u8 {\n  val t = xs[0]\n  xs[0] = xs[1]\n  xs[1] = t\n  k\n}";
    let leaving = call_leaving(
        &module(src),
        "swap",
        vec![
            Value::Array(vec![
                Value::Int(Int::from_u128(1)),
                Value::Int(Int::from_u128(2)),
            ]),
            Value::Int(Int::from_u128(7)),
        ],
        Limits::default(),
    );
    assert_eq!(
        leaving.map(|(v, finals)| (
            v.to_string(),
            finals.iter().map(|f| f.to_string()).collect::<Vec<_>>()
        )),
        Ok(("7".to_string(), vec!["[2, 1]".to_string(), "7".to_string()]))
    );
}

#[test]
fn test_depth_counts_nested_calls_only() {
    let down = "pub fn down(n: u32) -> u32 {\n  if n == 0 {\n    return 0\n  }\n  down(n - 1)\n}";
    let limits = Limits {
        max_depth: 50,
        ..Limits::default()
    };
    assert_eq!(
        run_with(down, "down(49)", limits),
        Ok(Value::Int(Int::ZERO))
    );
    assert!(matches!(
        run_with(down, "down(50)", limits),
        Err(Stop::Trap(Trap {
            kind: TrapKind::StackOverflow,
            ..
        }))
    ));
    // Calls one after another do not add up.
    let many = "pub fn one() -> u32 = 1\npub fn many() -> u32 {\n  var total: u32 = 0\n  var i: u32 = 0\n  while i < 20 {\n    total += one()\n    i += 1\n  }\n  total\n}";
    let shallow = Limits {
        max_depth: 3,
        ..Limits::default()
    };
    assert_eq!(
        run_with(many, "many()", shallow),
        Ok(Value::Int(Int::from_u128(20)))
    );
    assert_eq!(run(down, "down(5000)"), "0");
    // A call of a function without a body does not check (`E0224`), so no run meets one.
    // Only a call of a function by itself compares measures.
    let pair = "pub fn m(n: u8) -> u8\n  decreases n\n= n\npub fn caller(n: u8) -> u8\n  decreases n\n= m(n)";
    assert_eq!(run(pair, "caller(3)"), "3");
    let defaults = Limits::default();
    assert_eq!(
        (defaults.max_depth, defaults.max_steps, defaults.max_array),
        (10_000, 100_000_000, 1 << 24)
    );
}

#[test]
fn test_each_ensures_clause_reads_its_own_result_and_elements_take_their_type() {
    let two = "pub fn inc(x: u8) -> u8\n  ensures result > x\n  ensures result >= x + 1\n= x + 1";
    assert_eq!(run(two, "inc(3)"), "4");
    let shifted = "pub fn f(a: u8) -> [2]u8 = [a, 1 << 8]";
    assert_eq!(kind(shifted, "f(1)"), "shift-amount");
    assert_eq!(super::stack_bytes(0), 1 << 20);
    assert_eq!(super::stack_bytes(2), (1 << 20) + 2 * (64 << 10));
    assert_eq!(Int::from_i128(0), Int::ZERO);
    assert_eq!(Int::from_i128(-3).to_string(), "-3");
}

/// Tier 1 never contradicts the interpreter: a program it proves, which `check_program`
/// accepts, never trips the run-time check of a refinement, a `requires`, an `ensures`, an
/// invariant or a `decreases` measure, on any input tried ([`interpreter-checks-contracts`]).
///
/// [`interpreter-checks-contracts`]: ../../docs/design-review.md#interpreter-checks-contracts
#[test]
fn test_a_program_tier1_proves_never_trips_its_checks() {
    let programs = [
        "pub fn midpoint(lo: usize, hi: usize) -> usize\n  requires lo < hi\n  ensures lo <= result and result < hi\n= lo + (hi - lo) / 2\npub fn f(a: u8, b: u8) -> usize {\n  guard a < b else {\n    return 0\n  }\n  midpoint(a, b)\n}",
        "pub fn f(a: u8, b: u8) -> u8[0..=100] {\n  guard b > 0 and a <= b else {\n    return 0\n  }\n  val r: u64 = a * 100 / b\n  guard r <= 100 else {\n    return 0\n  }\n  r\n}",
        "pub fn f(a: u8, b: u8) -> u8 {\n  var i: u8 = 0\n  var total: u8[0..=200] = 0\n  while i < a invariant i <= a {\n    guard total < 200 else {\n      break\n    }\n    total += 1\n    i += 1\n  }\n  val m: u8[0..=b] = if total < b { total } else { b }\n  m\n}",
        "pub fn f(a: u8, b: u8) -> u8[0..16] {\n  guard b > 0 else {\n    return 0\n  }\n  val r: u8[0..b] = a % b\n  r & 15\n}",
        "pub fn g(x: u8, y: u8) -> u8\n  decreases y\n{\n  if y == 0 {\n    return x\n  }\n  g(y, x % y)\n}\npub fn f(a: u8, b: u8) -> u8 {\n  var x: u8 = a\n  var y: u8 = b\n  while y != 0 decreases y {\n    val t = x % y\n    x = y\n    y = t\n  }\n  x + g(a, b) - g(a, b)\n}",
        "pub fn f(a: u8, b: u8) -> u8[0..=255] {\n  guard a <= b else {\n    return 0\n  }\n  var lo: u8 = a\n  var hi: u8 = b\n  while lo < hi invariant lo <= hi {\n    val mid: u8 = lo + (hi - lo) / 2\n    if mid % 3 == 0 {\n      hi = mid\n    } else {\n      lo = mid + 1\n    }\n  }\n  lo\n}",
    ];
    let contracts = [
        "refinement",
        "requires",
        "ensures",
        "invariant",
        "decreases",
    ];
    for src in programs {
        let text = format!("module m\n{}\n", src);
        let m = match mente_sema::check_program(&text) {
            Ok(m) => m,
            Err(d) => panic!("Tier 1 should prove {}: {:?}", src, d),
        };
        for a in (0..=255u128).step_by(5) {
            for b in (0..=255u128).step_by(7) {
                let args = vec![Value::Int(Int::from_u128(a)), Value::Int(Int::from_u128(b))];
                if let Err(Stop::Trap(t)) = call(&m, "f", args, Limits::default()) {
                    assert!(
                        !contracts.contains(&t.kind.name()),
                        "{} trapped on f({}, {}): {}",
                        src,
                        a,
                        b,
                        t.kind.name()
                    );
                }
            }
        }
    }
}

#[test]
fn test_a_mut_parameter_hands_back_into_its_place_refinement() {
    let src = "pub fn bump(mut x: u8, by: u8) {\n  x += by\n}\npub fn main(by: u8) -> u8 {\n  var v: u8[0..10] = 8\n  bump(mut v, by)\n  v\n}";
    assert_eq!(run(src, "main(1)"), "9");
    assert_eq!(trapped(src, "main(2)"), ("refinement".to_string(), 7, 3));
    let elems = "pub fn bump(mut x: u8) {\n  x += 5\n}\npub fn main(i: usize) -> [2]u8 {\n  var xs: [2]u8[0..10] = [1, 6]\n  bump(mut xs[i])\n  xs\n}";
    assert_eq!(run(elems, "main(0)"), "[6, 6]");
    assert_eq!(kind(elems, "main(1)"), "refinement");
}

/// Several `decreases` clauses of a function form one measure, which a direct recursive call
/// makes smaller in lexicographic order.
#[test]
fn test_function_measures_are_lexicographic() {
    // The first part stays and the second gets smaller.
    let second = "pub fn r(a: u8, b: u8) -> u8\n  decreases a\n  decreases b\n{\n  if b == 0 {\n    return a\n  }\n  r(a, b - 1)\n}";
    assert_eq!(run(second, "r(2, 3)"), "2");
    // Neither gets smaller.
    let neither = "pub fn r(a: u8, b: u8) -> u8\n  decreases a\n  decreases b\n{\n  if b == 0 {\n    return a\n  }\n  r(a, b)\n}";
    assert_eq!(trapped(neither, "r(2, 3)"), ("decreases".to_string(), 9, 3));
    // The first gets smaller while the second grows.
    let first = "pub fn r(a: u8, b: u8) -> u8\n  decreases a\n  decreases b\n{\n  if a == 0 {\n    return b\n  }\n  r(a - 1, b + 10)\n}";
    assert_eq!(run(first, "r(3, 1)"), "31");
    // Each part is never negative.
    let negative = "pub fn r(a: i8, b: i8) -> i8\n  decreases a\n  decreases b\n= 0";
    assert_eq!(kind(negative, "r(1, -1)"), "decreases");
}

#[test]
fn test_an_argument_of_another_shape_is_refused_before_anything_runs() {
    let src = "pub fn h(xs: [2]u8) -> u8 = xs[0]\npub fn b(x: bool) -> bool = x";
    for (call, value) in [
        ("h([1])", "[1]"),
        ("h([1, 2, 3])", "[1, 2, 3]"),
        ("h([true, 2])", "[true, 2]"),
        ("h(5)", "5"),
        ("b(1)", "1"),
    ] {
        assert_eq!(
            run(src, call),
            format!("`{}` does not have its parameter's shape", value),
            "{}",
            call
        );
    }
    assert_eq!(run(src, "h([1, 2])"), "1");
    assert_eq!(run(src, "b(true)"), "true");
}

/// A struct is a value built field by field, each value narrowed into its field's type and
/// checked against its refinement where it lands, in the order written; binding, passing and
/// returning it copies it; a field of a `var` or a `mut` parameter is a place, read and stored
/// through indexes and other fields, and passed as `mut`, its final value copied back.
#[test]
fn test_structs_are_values_built_in_field_order() {
    let decls = "pub struct P {\n  a: u8\n  b: i8[-3..=3]\n}\npub struct Q {\n  p: P\n  xs: [2]P\n  f: bool\n}\n";
    let with = |f: &str| format!("{}{}", decls, f);
    let make = with("pub fn make(a: i16, b: i16) -> P = P(a: a, b: b)");
    assert_eq!(run(&make, "make(255, -3)"), "P(a: 255, b: -3)");
    // The first field's value traps before the second's is checked.
    assert_eq!(
        trapped(&make, "make(256, 9)"),
        ("narrowing".to_string(), 11, 41)
    );
    assert_eq!(
        trapped(&make, "make(2, 4)"),
        ("refinement".to_string(), 11, 47)
    );
    let places = with("pub fn set(mut q: Q, i: usize, v: u8) -> u8 {\n  q.p.a = v\n  q.xs[i].a += 1\n  q.xs[1] = q.p\n  q.f = not q.f\n  q.xs[0].a\n}");
    let q = "Q(p: P(a: 1, b: 0), xs: [P(a: 2, b: 1), P(a: 3, b: 2)], f: false)";
    assert_eq!(run(&places, &format!("set(mut {}, 0, 7)", q)), "3");
    assert_eq!(
        kind(&places, &format!("set(mut {}, 2, 7)", q)),
        "index-out-of-bounds"
    );
    let (v, finals) = call_leaving(
        &module(&places),
        "set",
        vec![
            parse_call(&format!("f({})", q)).unwrap().1.remove(0),
            Value::Int(Int::from_u128(1)),
            Value::Int(Int::from_u128(9)),
        ],
        Limits::default(),
    )
    .unwrap();
    assert_eq!(v.to_string(), "2");
    assert_eq!(
        finals[0].to_string(),
        "Q(p: P(a: 9, b: 0), xs: [P(a: 2, b: 1), P(a: 9, b: 0)], f: true)"
    );
    // A copy stays apart from its original, and a store into a refined field is checked.
    let copy = with("pub fn copy(p: P) -> [2]u8 {\n  var q = p\n  q.a = 9\n  [p.a, q.a]\n}");
    assert_eq!(run(&copy, "copy(P(a: 1, b: 0))"), "[1, 9]");
    let store = with("pub fn store(mut p: P, v: i8) {\n  p.b = v\n}");
    assert_eq!(kind(&store, "store(mut P(a: 1, b: 0), 5)"), "refinement");
    // A `mut` field argument takes the callee's final value back into its refinement.
    let back =
        with("fn grow(mut n: i8) {\n  n += 3\n}\npub fn field(mut p: P) {\n  grow(mut p.b)\n}");
    assert_eq!(kind(&back, "field(mut P(a: 1, b: 0))"), "()");
    assert_eq!(kind(&back, "field(mut P(a: 1, b: 1))"), "refinement");
}

/// A struct prints as Mente writes it, `Name(field: v, ...)`, in the order of its declaration,
/// and a call from outside reads it in that form; one whose fields are named otherwise, or lie
/// outside their types or refinements, is refused or traps as an argument does.
#[test]
fn test_a_struct_value_prints_and_reads_back_in_one_form() {
    let src = "pub struct P {\n  a: u8\n  b: i8[-3..=3]\n}\npub struct W {\n  p: P\n  wide: i128\n}\npub fn id(w: W) -> W = w";
    let w = "W(p: P(a: 255, b: -3), wide: -170141183460469231731687303715884105728)";
    assert_eq!(run(src, &format!("id({})", w)), w);
    assert_eq!(
        parse_call("f(P(a: 1, b: -2), [P(a: 0, b: 0)])").map(|(_, a)| a.len()),
        Some(2)
    );
    assert_eq!(parse_call("f(P(a: 1 b: 2))"), None);
    assert_eq!(parse_call("f(P(1, 2))"), None);
    // A field's name is a name: not empty, and not beginning with a digit.
    assert_eq!(parse_call("f(P(: 1))"), None);
    assert_eq!(parse_call("f(P(1a: 1))"), None);
    assert_eq!(parse_call("f(P(a: 1, : 2))"), None);
    assert!(parse_call("f(P(a1: 1, _b: 2))").is_some());
    // A value has its parameter's shape: its struct, each field once in order, of its type.
    for wrong in [
        "id(W(wide: 0, p: P(a: 1, b: 0)))",
        "id(V(p: P(a: 1, b: 0), wide: 0))",
        "id(W(p: P(a: 1, c: 0), wide: 0))",
        "id(W(p: P(a: true, b: 0), wide: 0))",
        "id(W(p: P(a: 1, b: 0), wide: [0]))",
        "id(W(p: P(a: 1), wide: 0))",
        "id(W(p: P(a: 1, b: 0, b: 0), wide: 0))",
    ] {
        assert!(
            run(src, wrong).contains("does not have its parameter's shape"),
            "{}",
            wrong
        );
    }
    assert_eq!(kind(src, "id(W(p: P(a: 256, b: 0), wide: 0))"), "narrowing");
    assert_eq!(kind(src, "id(W(p: P(a: 1, b: 4), wide: 0))"), "refinement");
}

/// A value keeps the value it had when it was evaluated (`core-calculus.md`, items 10 and 12):
/// an argument read from a binding that a later argument changes, and a field read through an
/// index that changes the binding the index reads; and a field's path is evaluated left to
/// right, each index checked before the next is evaluated.
#[test]
fn test_a_value_keeps_the_value_it_was_evaluated_to() {
    let src = "pub struct D {\n  d: u8\n}\npub struct S {\n  xs: [2]D\n}\npub struct O {\n  ys: [2]D\n}\nfn get(p: D, k: u8) -> u8 = p.d + k\nfn set(mut p: D) -> u8 {\n  p.d = 9\n  0\n}\npub fn argument() -> u8 {\n  var p = D(d: 1)\n  get(p, set(mut p))\n}\nfn one(mut xs: [2]D) -> usize {\n  xs[1].d = 50\n  1\n}\npub fn through_index() -> u8 {\n  var xs: [2]D = [D(d: 1), D(d: 2)]\n  xs[one(mut xs)].d\n}\npub fn element() -> D {\n  var xs: [2]D = [D(d: 1), D(d: 2)]\n  xs[one(mut xs)]\n}\nfn reset(mut s: S) -> usize {\n  s.xs[0].d = 99\n  0\n}\npub fn through_field() -> u8 {\n  var s = S(xs: [D(d: 1), D(d: 2)])\n  s.xs[reset(mut s)].d\n}\npub fn rd(xs: [2]O, i: usize, j: usize, k: usize) -> u8 = xs[i].ys[j / k].d";
    assert_eq!(run(src, "argument()"), "1");
    assert_eq!(run(src, "through_index()"), "2");
    assert_eq!(run(src, "element()"), "D(d: 2)");
    assert_eq!(run(src, "through_field()"), "1");
    let o = "O(ys: [D(d: 1), D(d: 2)])";
    assert_eq!(
        kind(src, &format!("rd([{}, {}], 5, 1, 0)", o, o)),
        "index-out-of-bounds"
    );
    assert_eq!(
        kind(src, &format!("rd([{}, {}], 1, 1, 0)", o, o)),
        "division-by-zero"
    );
    assert_eq!(run(src, &format!("rd([{}, {}], 1, 1, 1)", o, o)), "2");
}

/// A variant is built as a struct is, each value narrowed into its field's type and checked
/// against its refinement in the order written; a `match` evaluates its scrutinee once and runs
/// the first arm whose pattern matches, its bindings reading the payload.
#[test]
fn test_enums_are_built_and_matched() {
    let src = "pub enum Shape {\n  Dot\n  Rect(w: u8, h: i8[-3..=3])\n}\nfn count(mut n: u8) -> u8 {\n  n += 1\n  n\n}\npub fn area(s: Shape) -> i16 = match s {\n  Shape.Rect(w: w, h: h) => w * h\n  Shape.Dot => 0\n}\npub fn make(w: u16, h: i8) -> Shape = Shape.Rect(w: w, h: h)\npub fn sign(x: i8) -> i8 = match x {\n  -128 => -2\n  0 => 0\n  _ => if x < 0 { -1 } else { 1 }\n}\npub fn once() -> u8 {\n  var n: u8 = 0\n  val k = match count(mut n) {\n    1 => 10\n    _ => 20\n  }\n  n + k\n}\npub fn flip(b: bool) -> bool = match b {\n  true => false\n  false => true\n}";
    assert_eq!(run(src, "area(Shape.Rect(w: 5, h: -3))"), "-15");
    assert_eq!(run(src, "area(Shape.Dot)"), "0");
    assert_eq!(run(src, "make(7, 2)"), "Shape.Rect(w: 7, h: 2)");
    assert_eq!(kind(src, "make(256, 0)"), "narrowing");
    assert_eq!(kind(src, "make(1, 4)"), "refinement");
    assert_eq!(run(src, "sign(-128)"), "-2");
    assert_eq!(run(src, "sign(-7)"), "-1");
    assert_eq!(run(src, "sign(0)"), "0");
    // The scrutinee is evaluated once.
    assert_eq!(run(src, "once()"), "11");
    assert_eq!(run(src, "flip(false)"), "true");
    // A `return` in an arm or a branch of a function whose body is an expression ends that
    // call alone: its caller goes on with the value returned.
    // An integer scrutinee is narrowed into its type: a value outside it traps as a narrowing
    // does, before any arm, whatever the arms cover.
    let src = "pub fn f(x: u8) -> u8 = match x + 1 {\n  0 => 1\n  255 => 3\n  _ => 2\n}\npub fn g(x: i8) -> i8 = match -x {\n  -128 => 0\n  _ => 1\n}";
    assert_eq!(trapped(src, "f(255)"), ("narrowing".to_string(), 2, 31));
    assert_eq!(run(src, "f(254)"), "3");
    assert_eq!(kind(src, "g(-128)"), "narrowing");
    assert_eq!(run(src, "g(-127)"), "1");
    let src = "pub enum E {\n  A\n  B\n}\npub fn ifr(c: bool) -> u8 = if c {\n  return 1\n} else {\n  return 2\n}\npub fn arm(e: E) -> u8 = match e {\n  E.A => {\n    return 7\n  }\n  E.B => 3\n}\npub fn operand(x: u8) -> u8 = 5 + (match x {\n  0 => {\n    return 9\n  }\n  _ => 1\n})\npub fn caller() -> u8 = ifr(true) + arm(E.A) + arm(E.B) + operand(0) + operand(1)";
    assert_eq!(run(src, "caller()"), "26");
}

/// A variant prints as Mente writes it, `Name.A` or `Name.B(field: v, ...)`, and reads back so as
/// an argument; one of another enum, an unknown variant, or a payload not named in order is no
/// value of its parameter.
#[test]
fn test_an_enum_value_prints_and_reads_back_in_one_form() {
    let src = "pub struct P {\n  a: u8\n}\npub enum E {\n  A\n  B(p: P, xs: [2]i8[-1..=1])\n}\npub fn id(e: E) -> E = e\npub fn all(es: [2]E) -> [2]E = es";
    for v in ["E.A", "E.B(p: P(a: 255), xs: [-1, 1])"] {
        assert_eq!(run(src, &format!("id({})", v)), v);
    }
    assert_eq!(
        run(src, "all([E.A, E.B(p: P(a: 1), xs: [0, 0])])"),
        "[E.A, E.B(p: P(a: 1), xs: [0, 0])]"
    );
    assert_eq!(
        parse_call("f(E.B(p: P(a: 1), xs: [0, 0]), E.A)").map(|(_, a)| a),
        Some(vec![
            Value::Enum(
                "E".into(),
                "B".into(),
                vec![
                    (
                        "p".into(),
                        Value::Struct(
                            "P".into(),
                            vec![("a".into(), Value::Int(Int::from_u128(1)))]
                        )
                    ),
                    (
                        "xs".into(),
                        Value::Array(vec![Value::Int(Int::ZERO), Value::Int(Int::ZERO)])
                    ),
                ]
            ),
            Value::Enum("E".into(), "A".into(), Vec::new()),
        ])
    );
    assert_eq!(parse_call("f(E.)"), None);
    assert_eq!(parse_call("f(E.1A)"), None);
    assert_eq!(parse_call("f(E.B(1))"), None);
    for wrong in [
        "id(E.C)",
        "id(F.A)",
        "id(E.A(p: P(a: 1)))",
        "id(E.B(xs: [0, 0], p: P(a: 1)))",
        "id(E.B(p: P(a: 1)))",
        "id(E.B)",
        "id(P(a: 1))",
        // Fields named as declared, a value of another shape in one.
        "id(E.B(p: 5, xs: [0, 0]))",
    ] {
        assert!(
            run(src, wrong).contains("does not have its parameter's shape"),
            "{}",
            wrong
        );
    }
    assert_eq!(kind(src, "id(E.B(p: P(a: 256), xs: [0, 0]))"), "narrowing");
    assert_eq!(kind(src, "id(E.B(p: P(a: 1), xs: [0, 2]))"), "refinement");
    // A variant written with empty parentheses is found in a call's text, after spaces too,
    // so a refused call can name it; a call of a function, or parentheses after no variant,
    // are none.
    for (text, found) in [
        ("id(M.Z())", Some("M.Z")),
        ("id(Mx.Z( ))", Some("Mx.Z")),
        ("id(M.Z ())", Some("M.Z")),
        ("M.Z()", Some("M.Z")),
        ("id(M.Z(a: 1), N_2.Y_3())", Some("N_2.Y_3")),
        ("Z()", None),
        (" ()", None),
        ("id(M.())", None),
        ("id(g())", None),
        ("id(.Z())", None),
    ] {
        assert_eq!(empty_payload(text).as_deref(), found, "{}", text);
    }
}

/// The interpreter is Tier 1's independent judge, so a state the checks rule out stops its run
/// as broken, rather than letting it go on with a value made up for it. Each module here passed
/// the checks, and then has one fact of its typing, resolution or tree changed by hand.
#[test]
fn test_a_state_the_checks_rule_out_stops_the_run() {
    use mente_sema::types::Ty;
    use mente_syntax::tree::{ExprKind, TypeKind};

    let broken_with =
        |src: &str, call_text: &str, why: &str, tamper: &dyn Fn(&mut mente_sema::CheckedFn)| {
            let mut m = module(src);
            let f = m.functions.last_mut().and_then(Option::as_mut).unwrap();
            tamper(f);
            let (name, args) = parse_call(call_text).unwrap();
            match call(&m, &name, args, Limits::default()) {
                Err(Stop::Broken(found)) if found.contains(why) => {}
                other => panic!("{}: {:?}, not a run broken by {}", src, other, why),
            }
        };
    let broken = |src: &str, call_text: &str, tamper: &dyn Fn(&mut mente_sema::CheckedFn)| {
        broken_with(src, call_text, "", tamper)
    };
    let expr = |f: &mente_sema::CheckedFn, kind: fn(&ExprKind) -> bool| {
        f.syntax
            .tree
            .exprs
            .iter()
            .position(|e| kind(&e.kind))
            .unwrap()
    };
    // A name that resolves to nothing, which the interpreter read as `()`.
    broken("pub fn f(x: u8) -> u8 = x", "f(3)", &|f| {
        let x = expr(f, |k| matches!(k, ExprKind::Name(_)));
        f.resolution.names[x] = None;
    });
    // A call that resolves to no function, likewise.
    broken(
        "pub fn g() -> u8 = 1\npub fn f() -> u8 = g()",
        "f()",
        &|f| {
            let c = expr(f, |k| matches!(k, ExprKind::Call { .. }));
            f.resolution.calls[c] = None;
        },
    );
    // An index into an integer, which gave the integer itself.
    broken(
        "pub fn f(n: u8, xs: [2]u8) -> u8 = xs[0]",
        "f(3, [1, 2])",
        &|f| {
            let xs = expr(f, |k| matches!(k, ExprKind::Name(n) if n == "xs"));
            let n = f.resolution.bindings.iter().position(|b| b.name == "n");
            f.resolution.names[xs] = n.map(|i| mente_sema::names::BindingId(i as u32));
        },
    );
    // An integer narrowed into an array literal whose element type says `bool`, which went
    // through unnarrowed.
    broken(
        "pub fn f(x: u8) -> u8 {\n  val a: [1]u8 = [x]\n  a[0]\n}",
        "f(3)",
        &|f| {
            let a = expr(f, |k| matches!(k, ExprKind::Array(_)));
            f.typing.exprs[a] = Ty::Array(1, Box::new(Ty::Bool));
        },
    );
    // A value checked against a declared type of another shape, whose refinement was skipped.
    broken(
        "pub fn f(x: u8) -> u8 {\n  val y: u8[0..9] = x\n  y\n}",
        "f(3)",
        &|f| {
            let t = f
                .syntax
                .tree
                .types
                .iter()
                .position(|t| {
                    matches!(
                        t.kind,
                        TypeKind::Int {
                            refinement: Some(_),
                            ..
                        }
                    )
                })
                .unwrap();
            f.syntax.tree.types[t].kind = TypeKind::Bool;
        },
    );
    // A refinement bound that names no binding, which read as 0.
    broken("pub fn f(n: u8, x: u8[0..=n]) -> u8 = x", "f(5, 3)", &|f| {
        for b in &mut f.resolution.bindings {
            if b.name == "n" {
                b.name = "renamed".to_string();
            }
        }
    });
}

/// The states the second review found the interpreter running on through: a binding statement
/// that declares nothing, a place whose binding holds no value, and a pattern of another kind
/// than its value, each of which went on with `()` or the next arm.
#[test]
fn test_a_pattern_or_a_place_the_checks_rule_out_stops_the_run() {
    use mente_syntax::tree::{ExprKind, PatternKind, StmtKind};

    let broken_with =
        |src: &str, call_text: &str, why: &str, tamper: &dyn Fn(&mut mente_sema::CheckedFn)| {
            let mut m = module(src);
            let f = m.functions.last_mut().and_then(Option::as_mut).unwrap();
            tamper(f);
            let (name, args) = parse_call(call_text).unwrap();
            match call(&m, &name, args, Limits::default()) {
                Err(Stop::Broken(found)) if found.contains(why) => {}
                other => panic!("{}: {:?}, not a run broken by {}", src, other, why),
            }
        };
    // A binding statement that declares no binding, which gave `()`.
    broken_with(
        "pub fn f(x: u8) -> u8 {\n  val y: u8 = x\n  y\n}",
        "f(3)",
        "declares no binding",
        &|f| {
            let s = f
                .syntax
                .tree
                .stmts
                .iter()
                .position(|s| matches!(s.kind, StmtKind::Binding { .. }))
                .unwrap();
            f.resolution.stmts[s] = None;
        },
    );
    // A `mut` argument whose binding holds no value yet, read as `()`; the callee only writes it.
    broken_with(
        "fn set(mut z: u8) {\n  z = 1\n}\npub fn f(x: u8) -> u8 {\n  var y: u8 = x\n  set(mut y)\n  var w: u8 = y\n  w += 1\n  w\n}",
        "f(3)",
        "holds no value",
        &|f| {
            let call = f
                .syntax
                .tree
                .exprs
                .iter()
                .position(|e| matches!(e.kind, ExprKind::Call { .. }))
                .unwrap();
            let ExprKind::Call { args, .. } = &f.syntax.tree.exprs[call].kind else {
                unreachable!("a call")
            };
            let y = args[0].value.0 as usize;
            let w = f.resolution.bindings.iter().position(|b| b.name == "w");
            f.resolution.names[y] = w.map(|i| mente_sema::names::BindingId(i as u32));
        },
    );
    // A pattern of another kind than its scrutinee's value, which fell to the next arm.
    broken_with(
        "pub fn f(x: u8) -> u8 = match x {\n  0 => 1\n  _ => 2\n}",
        "f(5)",
        "a pattern of another kind",
        &|f| {
            let m = f
                .syntax
                .tree
                .exprs
                .iter()
                .position(|e| matches!(e.kind, ExprKind::Match { .. }))
                .unwrap();
            if let ExprKind::Match { arms, .. } = &mut f.syntax.tree.exprs[m].kind {
                arms[0].pattern.kind = PatternKind::Bool(true);
            }
        },
    );
}
