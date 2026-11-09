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
    let cube = "pub fn h(a: u64) -> u128 = a * a * a";
    assert_eq!(
        trapped(cube, "h(18446744073709551615)"),
        ("intermediate-overflow".to_string(), 2, 28)
    );
    assert_eq!(run(cube, "h(1000)"), "1000000000");
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
    let lit = "pub fn lit() -> u8 {\n  val m: u8 = 1 << 7\n  val k: u8 = ~0\n  m - 1 + k - 255\n}";
    assert_eq!(run(lit, "lit()"), "127");
    let too_far = "pub fn far() -> u8 {\n  val m: u8 = 1 << 8\n  m\n}";
    assert_eq!(kind(too_far, "far()"), "shift-amount");
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
    let declared = "pub fn later(x: u8) -> u8\npub fn now() -> u8 = later(1)";
    assert_eq!(
        run(declared, "now()"),
        "no function `later` with a body to run"
    );
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
/// accepts, never trips the run-time check of a refinement, a `requires`, an `ensures` or an
/// invariant, on any input tried ([`interpreter-checks-contracts`]).
///
/// [`interpreter-checks-contracts`]: ../../docs/design-review.md#interpreter-checks-contracts
#[test]
fn test_a_program_tier1_proves_never_trips_its_checks() {
    let programs = [
        "pub fn midpoint(lo: usize, hi: usize) -> usize\n  requires lo < hi\n  ensures lo <= result and result < hi\n= lo + (hi - lo) / 2\npub fn f(a: u8, b: u8) -> usize {\n  guard a < b else {\n    return 0\n  }\n  midpoint(a, b)\n}",
        "pub fn f(a: u8, b: u8) -> u8[0..=100] {\n  guard b > 0 and a <= b else {\n    return 0\n  }\n  val r: u64 = a * 100 / b\n  guard r <= 100 else {\n    return 0\n  }\n  r\n}",
        "pub fn f(a: u8, b: u8) -> u8 {\n  var i: u8 = 0\n  var total: u8[0..=200] = 0\n  while i < a invariant i <= a {\n    guard total < 200 else {\n      break\n    }\n    total += 1\n    i += 1\n  }\n  val m: u8[0..=b] = if total < b { total } else { b }\n  m\n}",
        "pub fn f(a: u8, b: u8) -> u8[0..16] {\n  guard b > 0 else {\n    return 0\n  }\n  val r: u8[0..b] = a % b\n  r & 15\n}",
        "pub fn f(a: u8, b: u8) -> u8[0..=255] {\n  guard a <= b else {\n    return 0\n  }\n  var lo: u8 = a\n  var hi: u8 = b\n  while lo < hi invariant lo <= hi {\n    val mid: u8 = lo + (hi - lo) / 2\n    if mid % 3 == 0 {\n      hi = mid\n    } else {\n      lo = mid + 1\n    }\n  }\n  lo\n}",
    ];
    let contracts = ["refinement", "requires", "ensures", "invariant"];
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
