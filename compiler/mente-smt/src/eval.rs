//! An evaluator of the queries this crate writes, under values given for their inputs, as
//! SMT-LIB defines their operations: the tests check a query against the reference interpreter
//! with it, input by input, without a solver, and `mn smt --confirm` checks that a solver's model
//! satisfies the query it answered before it runs the model.
//!
//! It reads the text of a query, not the table that wrote it, so a slip in the writing shows: its
//! `declare-const`s, its `define-fun`s of no parameter, each read once in order, and its
//! `assert`s. It knows the operations [`crate::term`] writes and no others. A divisor of zero,
//! which SMT-LIB leaves to the model, gives 0 here: the encoding reads a quotient only on paths
//! where its divisor is not zero.

use std::collections::{BTreeMap, HashMap};

use crate::num::Big;

/// A value of a query's term.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Int(Big),
    Bool(bool),
    /// An array: its elements set apart from the rest, which are all `default`.
    Array(Box<Value>, BTreeMap<Big, Value>),
}

/// An S-expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sexp {
    Atom(String),
    List(Vec<Sexp>),
}

/// The S-expressions of a text, its `;` comments left out.
pub fn parse(text: &str) -> Result<Vec<Sexp>, String> {
    let mut stack: Vec<Vec<Sexp>> = vec![Vec::new()];
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ';' => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '(' => stack.push(Vec::new()),
            ')' => {
                let done = stack.pop().ok_or("an unopened `)`")?;
                stack
                    .last_mut()
                    .ok_or("an unopened `)`")?
                    .push(Sexp::List(done));
            }
            c if c.is_whitespace() => {}
            c => {
                let mut atom = String::from(c);
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || n == '(' || n == ')' || n == ';' {
                        break;
                    }
                    atom.push(n);
                    chars.next();
                }
                stack
                    .last_mut()
                    .ok_or("an unopened `)`")?
                    .push(Sexp::Atom(atom));
            }
        }
    }
    match stack.len() {
        1 => Ok(stack.pop().unwrap_or_default()),
        _ => Err("an unclosed `(`".to_string()),
    }
}

/// Whether every `assert` of `query` holds when its inputs take `inputs`; an error names an
/// input with no value, or an operation this evaluator does not know.
pub fn holds(query: &str, inputs: &HashMap<String, Value>) -> Result<bool, String> {
    run(&parse(query)?, inputs).map(|(_, all)| all)
}

/// The value of each input and each definition of the parsed query `query` when its inputs
/// take `inputs`, and whether every `assert` holds.
pub fn run(
    query: &[Sexp],
    inputs: &HashMap<String, Value>,
) -> Result<(HashMap<String, Value>, bool), String> {
    let mut env: HashMap<String, Value> = HashMap::new();
    let mut all = true;
    for s in query {
        let Sexp::List(items) = s else {
            return Err(format!("a top-level atom: {:?}", s));
        };
        match items.first() {
            Some(Sexp::Atom(head)) => match head.as_str() {
                "declare-const" => {
                    let name = atom(items.get(1))?;
                    let v = inputs
                        .get(name)
                        .ok_or_else(|| format!("no value for the input `{}`", name))?;
                    env.insert(name.to_string(), v.clone());
                }
                "define-fun" => {
                    let name = atom(items.get(1))?;
                    match items.get(2) {
                        Some(Sexp::List(params)) if params.is_empty() => {}
                        _ => return Err(format!("`{}` takes parameters", name)),
                    }
                    let body = items.get(4).ok_or("a define-fun with no body")?;
                    let v = eval(body, &env)?;
                    env.insert(name.to_string(), v);
                }
                "assert" => {
                    let v = eval(items.get(1).ok_or("an empty assert")?, &env)?;
                    all &= v == Value::Bool(true);
                }
                "check-sat" | "get-value" | "set-logic" | "set-option" => {}
                other => return Err(format!("an unknown command `{}`", other)),
            },
            _ => return Err("a command that is not a list headed by its name".to_string()),
        }
    }
    Ok((env, all))
}

fn atom(s: Option<&Sexp>) -> Result<&str, String> {
    match s {
        Some(Sexp::Atom(a)) => Ok(a),
        Some(Sexp::List(_)) | None => Err("a name expected".to_string()),
    }
}

fn int(v: Value) -> Result<Big, String> {
    match v {
        Value::Int(i) => Ok(i),
        Value::Bool(_) | Value::Array(..) => Err("an integer expected".to_string()),
    }
}

fn boolean(v: Value) -> Result<bool, String> {
    match v {
        Value::Bool(b) => Ok(b),
        Value::Int(_) | Value::Array(..) => Err("a boolean expected".to_string()),
    }
}

/// The element `i` of an array value.
fn element(a: Value, i: &Big) -> Result<Value, String> {
    match a {
        Value::Array(default, items) => Ok(items.get(i).cloned().unwrap_or(*default)),
        Value::Int(_) | Value::Bool(_) => Err("an array expected".to_string()),
    }
}

/// An array value written in one form for each array, so that equal arrays are equal values:
/// the elements equal to the default left out.
fn normal(v: Value) -> Value {
    match v {
        Value::Array(default, items) => {
            let default = normal(*default);
            let items = items
                .into_iter()
                .map(|(k, x)| (k, normal(x)))
                .filter(|(_, x)| *x != default)
                .collect();
            Value::Array(Box::new(default), items)
        }
        other @ (Value::Int(_) | Value::Bool(_)) => other,
    }
}

/// The width of an `(_ int2bv N)` indexed operator.
fn int2bv_width(s: &Sexp) -> Option<u32> {
    match s {
        Sexp::List(parts) => match parts.as_slice() {
            [Sexp::Atom(u), Sexp::Atom(f), Sexp::Atom(n)] if u == "_" && f == "int2bv" => {
                n.parse().ok()
            }
            _ => None,
        },
        Sexp::Atom(_) => None,
    }
}

/// The operand of `((_ int2bv N) x)`: its width and its unsigned value in that width.
fn bitvector(s: &Sexp, env: &HashMap<String, Value>) -> Result<(u32, Big), String> {
    if let Sexp::List(parts) = s {
        if let [head, x] = parts.as_slice() {
            if let Some(n) = int2bv_width(head) {
                return Ok((n, int(eval(x, env)?)?.mod_pow2(n)));
            }
        }
    }
    Err("a bit-vector operand not written `((_ int2bv N) x)`".to_string())
}

/// The bits of an unsigned value below `2^width`, least significant first.
fn bits_of(v: &Big, width: u32) -> Vec<bool> {
    let mut out = Vec::with_capacity(width as usize);
    let mut rest = v.clone();
    let two = Big::from_u128(2);
    for _ in 0..width {
        let (q, r) = rest.div_mod(&two).unwrap_or((Big::zero(), Big::zero()));
        out.push(!r.is_zero());
        rest = q;
    }
    out
}

fn from_bits(bits: &[bool]) -> Big {
    bits.iter().rev().fold(Big::zero(), |acc, &b| {
        let twice = acc.add(&acc);
        if b {
            twice.add(&Big::from_u128(1))
        } else {
            twice
        }
    })
}

/// `(bv2nat (op ((_ int2bv N) x) ...))`.
fn bv2nat(s: &Sexp, env: &HashMap<String, Value>) -> Result<Value, String> {
    let Sexp::List(parts) = s else {
        return Err("bv2nat of an atom".to_string());
    };
    let (op, args) = match parts.split_first() {
        Some((Sexp::Atom(op), args)) => (op.as_str(), args),
        _ => return Err("a bit-vector operation with no name".to_string()),
    };
    let operands: Vec<(u32, Big)> = args
        .iter()
        .map(|a| bitvector(a, env))
        .collect::<Result<_, _>>()?;
    let width = operands.first().map(|o| o.0).ok_or("no operand")?;
    if operands.iter().any(|o| o.0 != width) {
        return Err("operands of different widths".to_string());
    }
    let bits: Vec<Vec<bool>> = operands.iter().map(|o| bits_of(&o.1, width)).collect();
    let pick = |f: fn(bool, bool) -> bool| -> Result<Vec<bool>, String> {
        match bits.as_slice() {
            [a, b] => Ok(a.iter().zip(b).map(|(&x, &y)| f(x, y)).collect()),
            _ => Err("a binary bit operation without two operands".to_string()),
        }
    };
    let out = match op {
        "bvand" => pick(|x, y| x & y)?,
        "bvor" => pick(|x, y| x | y)?,
        "bvxor" => pick(|x, y| x ^ y)?,
        "bvnot" => match bits.as_slice() {
            [a] => a.iter().map(|&x| !x).collect(),
            _ => return Err("bvnot without one operand".to_string()),
        },
        other => return Err(format!("an unknown bit-vector operation `{}`", other)),
    };
    Ok(Value::Int(from_bits(&out)))
}

/// The value of a term.
pub fn eval(s: &Sexp, env: &HashMap<String, Value>) -> Result<Value, String> {
    let items = match s {
        Sexp::Atom(a) => {
            return match a.as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ => match Big::parse(a) {
                    Some(v) if !a.starts_with('-') => Ok(Value::Int(v)),
                    _ => env
                        .get(a)
                        .cloned()
                        .ok_or_else(|| format!("an unknown name `{}`", a)),
                },
            };
        }
        Sexp::List(items) => items,
    };
    let (head, args) = items.split_first().ok_or("an empty term")?;
    // `((as const (Array Int S)) v)`.
    if let Sexp::List(h) = head {
        if matches!(h.first(), Some(Sexp::Atom(a)) if a == "as")
            && matches!(h.get(1), Some(Sexp::Atom(c)) if c == "const")
        {
            let v = eval(args.first().ok_or("a constant array of nothing")?, env)?;
            return Ok(Value::Array(Box::new(v), BTreeMap::new()));
        }
        return Err(format!("an unknown indexed operator {:?}", head));
    }
    let Sexp::Atom(op) = head else {
        return Err("a term with no operator".to_string());
    };
    if op == "bv2nat" {
        return bv2nat(args.first().ok_or("bv2nat of nothing")?, env);
    }
    let vals: Vec<Value> = match op.as_str() {
        // `ite` reads the branch it takes alone; `and` and `or` read every operand, below.
        "ite" | "and" | "or" => Vec::new(),
        _ => args
            .iter()
            .map(|a| eval(a, env))
            .collect::<Result<_, _>>()?,
    };
    let two_ints = |vals: Vec<Value>| -> Result<(Big, Big), String> {
        let mut it = vals.into_iter();
        match (it.next(), it.next(), it.next()) {
            (Some(a), Some(b), None) => Ok((int(a)?, int(b)?)),
            _ => Err(format!("`{}` takes two operands", op)),
        }
    };
    Ok(match op.as_str() {
        "+" => Value::Int(
            vals.into_iter()
                .try_fold(Big::zero(), |acc, v| int(v).map(|x| acc.add(&x)))?,
        ),
        "-" => match vals.len() {
            1 => Value::Int(int(vals.into_iter().next().ok_or("")?)?.neg()),
            _ => {
                let (a, b) = two_ints(vals)?;
                Value::Int(a.sub(&b))
            }
        },
        "*" => {
            let (a, b) = two_ints(vals)?;
            Value::Int(a.mul(&b))
        }
        "div" => {
            let (a, b) = two_ints(vals)?;
            Value::Int(a.div_mod(&b).map_or(Big::zero(), |(q, _)| q))
        }
        "mod" => {
            let (a, b) = two_ints(vals)?;
            Value::Int(a.div_mod(&b).map_or(Big::zero(), |(_, r)| r))
        }
        "<=" => {
            let (a, b) = two_ints(vals)?;
            Value::Bool(a <= b)
        }
        "<" => {
            let (a, b) = two_ints(vals)?;
            Value::Bool(a < b)
        }
        "=" => {
            let mut it = vals.into_iter().map(normal);
            match (it.next(), it.next(), it.next()) {
                (Some(a), Some(b), None) => Value::Bool(a == b),
                _ => return Err("`=` takes two operands".to_string()),
            }
        }
        "not" => match vals.as_slice() {
            [Value::Bool(b)] => Value::Bool(!b),
            _ => return Err("`not` takes one boolean".to_string()),
        },
        "and" => {
            let mut all = true;
            for a in args {
                all &= boolean(eval(a, env)?)?;
            }
            Value::Bool(all)
        }
        "or" => {
            let mut any = false;
            for a in args {
                any |= boolean(eval(a, env)?)?;
            }
            Value::Bool(any)
        }
        "ite" => {
            let [c, a, b] = args else {
                return Err("`ite` takes three operands".to_string());
            };
            if boolean(eval(c, env)?)? {
                eval(a, env)?
            } else {
                eval(b, env)?
            }
        }
        "select" => {
            let mut it = vals.into_iter();
            match (it.next(), it.next()) {
                (Some(a), Some(i)) => element(a, &int(i)?)?,
                _ => return Err("`select` takes two operands".to_string()),
            }
        }
        "store" => {
            let mut it = vals.into_iter();
            match (it.next(), it.next(), it.next()) {
                (Some(Value::Array(default, mut items)), Some(i), Some(v)) => {
                    items.insert(int(i)?, v);
                    Value::Array(default, items)
                }
                _ => return Err("`store` takes an array, an index and a value".to_string()),
            }
        }
        other => return Err(format!("an unknown operation `{}`", other)),
    })
}

/// The values a solver's answer to `(get-value (...))` gives: `((p.x 5) (p.b true) (p.y (- 3)))`.
pub fn model(answer: &str) -> Result<HashMap<String, Value>, String> {
    let mut out = HashMap::new();
    for s in parse(answer)? {
        let Sexp::List(pairs) = s else {
            continue;
        };
        for pair in pairs {
            let Sexp::List(kv) = pair else {
                return Err("a value that is not a pair".to_string());
            };
            let [Sexp::Atom(name), value] = kv.as_slice() else {
                return Err("a value that is not a name and a value".to_string());
            };
            let v = eval(value, &HashMap::new())?;
            out.insert(name.clone(), v);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(v: i128) -> Value {
        Value::Int(Big::from_i128(v))
    }

    fn holds_with(query: &str, inputs: &[(&str, Value)]) -> Result<bool, String> {
        let map = inputs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        holds(query, &map)
    }

    #[test]
    fn test_the_evaluator_reads_each_operation_as_smt_lib_defines_it() {
        let q = "; source: here\n\
                 (declare-const p.x Int)\n\
                 (declare-const p.b Bool)\n\
                 (define-fun t1 () Int (+ p.x 3))\n\
                 (define-fun t2 () Int (div (- 7) 2))\n\
                 (define-fun t3 () Int (mod (- 7) 2))\n\
                 (define-fun t4 () Bool (and (= t2 (- 4)) (= t3 1) (< t1 10) (<= t1 9)))\n\
                 (define-fun t5 () Int (ite p.b (* t1 2) (- t1)))\n\
                 (define-fun t6 () (Array Int Int) (store ((as const (Array Int Int)) 0) 1 t5))\n\
                 (define-fun t7 () Bool (or (= (select t6 1) 12) (not p.b)))\n\
                 (define-fun t8 () Int (bv2nat (bvand ((_ int2bv 8) (- 1)) ((_ int2bv 8) 300))))\n\
                 (define-fun t9 () Int (bv2nat (bvnot ((_ int2bv 8) 0))))\n\
                 (define-fun t10 () Int (bv2nat (bvor ((_ int2bv 4) 5) ((_ int2bv 4) 10))))\n\
                 (define-fun t11 () Int (bv2nat (bvxor ((_ int2bv 4) 6) ((_ int2bv 4) 3))))\n\
                 (define-fun t12 () Bool (and (= t8 44) (= t9 255) (= t10 15) (= t11 5)))\n\
                 (define-fun t13 () Int (div 5 0))\n\
                 (assert (and t4 t7 t12 (= (select t6 0) 0) (= t13 0)))\n\
                 (check-sat)\n\
                 (get-value (p.x p.b))\n";
        assert_eq!(
            holds_with(q, &[("p.x", int(3)), ("p.b", Value::Bool(true))]),
            Ok(true)
        );
        // t1 = 9; t5 = 18, so the store's element is not 12.
        assert_eq!(
            holds_with(q, &[("p.x", int(6)), ("p.b", Value::Bool(true))]),
            Ok(false)
        );
        assert_eq!(
            holds_with(q, &[("p.x", int(6)), ("p.b", Value::Bool(false))]),
            Ok(true)
        );
        assert_eq!(
            holds_with(q, &[("p.x", int(7)), ("p.b", Value::Bool(false))]),
            Ok(false)
        );
        assert!(holds_with(q, &[("p.x", int(3))])
            .unwrap_err()
            .contains("p.b"));
        assert!(holds_with("(assert (frob 1))", &[])
            .unwrap_err()
            .contains("frob"));
        assert!(holds_with("(assert (+ 1", &[]).is_err());
        assert!(holds_with("(assert 1))", &[]).is_err());
        assert!(holds_with("(push 1)", &[]).is_err());
    }

    #[test]
    fn test_equal_arrays_are_equal_however_they_were_built() {
        let q = "(define-fun a () (Array Int Int) (store ((as const (Array Int Int)) 0) 2 0))\n\
                 (define-fun b () (Array Int Int) ((as const (Array Int Int)) 0))\n\
                 (define-fun c () (Array Int Int) (store b 2 1))\n\
                 (assert (and (= a b) (not (= a c))))\n";
        assert_eq!(holds_with(q, &[]), Ok(true));
    }

    #[test]
    fn test_a_model_gives_each_input_its_value() {
        let m = model("((p.x 5)\n (p.y (- 3))\n (p.b true))").unwrap();
        assert_eq!(m.get("p.x"), Some(&int(5)));
        assert_eq!(m.get("p.y"), Some(&int(-3)));
        assert_eq!(m.get("p.b"), Some(&Value::Bool(true)));
        assert!(model("((p.x))").is_err());
        assert!(model("(5)").is_err());
    }

    /// A query defines constants alone, a function with parameters refused; and an indexed
    /// operator is read as `int2bv` only as `(_ int2bv N)`.
    #[test]
    fn test_a_query_defines_constants_alone_and_reads_int2bv_by_its_index() {
        let none = HashMap::new();
        assert_eq!(
            holds("(define-fun f () Int 3)\n(assert (= f 3))\n", &none),
            Ok(true)
        );
        assert_eq!(
            holds("(define-fun f ((x Int)) Int x)\n(assert true)\n", &none),
            Err("`f` takes parameters".to_string())
        );
        let width = |text: &str| int2bv_width(&parse(text).unwrap()[0]);
        assert_eq!(width("(_ int2bv 8)"), Some(8));
        for other in ["(_ zero_extend 8)", "(x int2bv 8)", "(_ int2bv)", "int2bv"] {
            assert_eq!(width(other), None, "{}", other);
        }
    }

    /// A numeral is never negative, as SMT-LIB writes it: `-5` is a name, and minus five is
    /// `(- 5)`; only `(as const ...)` makes a constant array; and `ite` reads the branch it takes
    /// alone, while `and` and `or` read every operand.
    #[test]
    fn test_numerals_constant_arrays_and_lazy_operators_read_as_smt_lib_writes_them() {
        let none = HashMap::new();
        assert_eq!(
            holds("(assert (= -5 -5))\n", &none),
            Err("an unknown name `-5`".to_string())
        );
        assert_eq!(holds("(assert (= (- 5) (- 5)))\n", &none), Ok(true));
        for head in ["(as nothing (Array Int Int))", "(_ const (Array Int Int))"] {
            let q = format!(
                "(define-fun a () (Array Int Int) ({} 0))\n(assert true)\n",
                head
            );
            let found = holds(&q, &none);
            assert!(
                found
                    .as_ref()
                    .is_err_and(|e| e.starts_with("an unknown indexed operator")),
                "{}: {:?}",
                head,
                found
            );
        }
        for q in [
            "(assert (= (ite true 1 nothing) 1))\n",
            "(assert (= (ite false nothing 2) 2))\n",
        ] {
            assert_eq!(holds(q, &none), Ok(true), "{}", q);
        }
        for q in [
            "(assert (not (and false nothing)))\n",
            "(assert (or true nothing))\n",
        ] {
            assert_eq!(
                holds(q, &none),
                Err("an unknown name `nothing`".to_string()),
                "{}",
                q
            );
        }
    }
}
