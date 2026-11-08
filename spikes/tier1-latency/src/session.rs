//! An in-memory module that takes edits and re-checks what they can affect, the shape of the
//! `mn check --watch` loop the Tier-1 protocol times.
//!
//! A body edit re-parses only the function it falls in and re-runs Tier 1 on it: callers see a
//! function only through its signature and contracts, so they are re-checked only when the
//! edited function's header changes (`spec/holographic-module.md`).

use crate::ast::{Expr, ExprKind, Function, Stmt, StmtKind};
use crate::parser::{parse_function_at, parse_module};
use crate::tier1::{check_function, Report, Sigs};
use crate::Diag;

pub struct Module {
    pub src: String,
    pub funcs: Vec<Function>,
    pub sigs: Sigs,
    pub reports: Vec<Report>,
    /// A parse error in a function, which then keeps its last good tree.
    pub broken: Vec<Option<Diag>>,
}

/// What one edit cost: the functions re-checked and the diagnostics they produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub rechecked: usize,
    pub parse_error: bool,
    pub must_failures: usize,
    pub diags: usize,
}

fn signatures(funcs: &[Function]) -> Sigs {
    funcs
        .iter()
        .map(|f| (f.sig.name.clone(), f.sig.clone()))
        .collect()
}

/// Whether `f`'s body or contracts call `name`.
fn calls(f: &Function, name: &str) -> bool {
    fn expr(e: &Expr, name: &str) -> bool {
        match &e.kind {
            ExprKind::Call(n, args) => n == name || args.iter().any(|a| expr(a, name)),
            ExprKind::Unary(_, a) | ExprKind::Len(a) | ExprKind::ArrayRepeat(a, _) => expr(a, name),
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => expr(a, name) || expr(b, name),
            ExprKind::Int(_) | ExprKind::Bool(_) | ExprKind::Name(_) => false,
        }
    }
    fn stmts(ss: &[Stmt], name: &str) -> bool {
        ss.iter().any(|s| match &s.kind {
            StmtKind::Let { init, .. } => expr(init, name),
            StmtKind::Assign { target, value, .. } => {
                target.indices.iter().any(|i| expr(i, name)) || expr(value, name)
            }
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => {
                expr(cond, name)
                    || invariants.iter().chain(decreases).any(|i| expr(i, name))
                    || stmts(body, name)
            }
            StmtKind::If { cond, then, els } => {
                expr(cond, name) || stmts(then, name) || stmts(els, name)
            }
            StmtKind::Guard { cond, body } => expr(cond, name) || stmts(body, name),
            StmtKind::Return(Some(e)) | StmtKind::Expr(e) => expr(e, name),
            StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => false,
        })
    }
    f.sig
        .requires
        .iter()
        .chain(&f.sig.ensures)
        .any(|e| expr(e, name))
        || stmts(&f.body, name)
}

impl Module {
    pub fn load(src: &str) -> Result<Module, Diag> {
        let funcs = parse_module(src)?;
        let sigs = signatures(&funcs);
        let reports = funcs.iter().map(|f| check_function(f, &sigs)).collect();
        let broken = vec![None; funcs.len()];
        Ok(Module {
            src: src.to_string(),
            funcs,
            sigs,
            reports,
            broken,
        })
    }

    /// The index of the function whose span holds `offset`.
    pub fn function_at(&self, offset: usize) -> Option<usize> {
        let k = self.funcs.partition_point(|f| f.span.end <= offset);
        (k < self.funcs.len() && self.funcs[k].span.start <= offset).then_some(k)
    }

    /// Applies an edit inside one function's body: re-parses that function alone and
    /// re-checks it, and its callers if its header changed.
    pub fn edit(&mut self, start: usize, end: usize, text: &str) -> Outcome {
        let Some(k) = self.function_at(start) else {
            return Outcome::default();
        };
        let delta = text.len() as isize - (end - start) as isize;
        self.src.replace_range(start..end, text);
        let shift = |v: usize| (v as isize + delta) as usize;
        self.funcs[k].span.end = shift(self.funcs[k].span.end);
        for f in &mut self.funcs[k + 1..] {
            f.span.start = shift(f.span.start);
            f.body_start = shift(f.body_start);
            f.span.end = shift(f.span.end);
        }
        let span = self.funcs[k].span;
        match parse_function_at(&self.src, span.start, span.end) {
            Err(d) => {
                self.broken[k] = Some(d);
                Outcome {
                    rechecked: 0,
                    parse_error: true,
                    must_failures: 0,
                    diags: 1,
                }
            }
            Ok(f) => {
                self.broken[k] = None;
                let header_changed = f.header_hash != self.funcs[k].header_hash;
                self.funcs[k] = f;
                let mut todo = vec![k];
                if header_changed {
                    let name = self.funcs[k].sig.name.clone();
                    self.sigs.insert(name.clone(), self.funcs[k].sig.clone());
                    todo.extend(
                        (0..self.funcs.len()).filter(|&c| c != k && calls(&self.funcs[c], &name)),
                    );
                }
                self.recheck(&todo)
            }
        }
    }

    /// The same edit, but the whole module is lexed and parsed again, and the functions whose
    /// digests changed are re-checked: the cost of an architecture without per-function parsing.
    pub fn edit_reparsing_module(&mut self, start: usize, end: usize, text: &str) -> Outcome {
        self.src.replace_range(start..end, text);
        match parse_module(&self.src) {
            Err(_) => Outcome {
                rechecked: 0,
                parse_error: true,
                must_failures: 0,
                diags: 1,
            },
            Ok(funcs) => {
                let mut todo = Vec::new();
                let mut header_changed = false;
                for (k, (new, old)) in funcs.iter().zip(&self.funcs).enumerate() {
                    if new.body_hash != old.body_hash || new.header_hash != old.header_hash {
                        todo.push(k);
                        header_changed |= new.header_hash != old.header_hash;
                    }
                }
                self.funcs = funcs;
                if header_changed {
                    self.sigs = signatures(&self.funcs);
                    todo = (0..self.funcs.len()).collect();
                }
                self.recheck(&todo)
            }
        }
    }

    fn recheck(&mut self, todo: &[usize]) -> Outcome {
        let mut out = Outcome {
            rechecked: todo.len(),
            ..Outcome::default()
        };
        for &k in todo {
            let r = check_function(&self.funcs[k], &self.sigs);
            out.must_failures += r.obligations.iter().filter(|o| o.must && !o.proven).count();
            out.diags += r.diags.len();
            self.reports[k] = r;
        }
        out
    }

    /// Every diagnostic of the module.
    pub fn diagnostics(&self) -> Vec<Diag> {
        let mut out: Vec<Diag> = self.broken.iter().flatten().cloned().collect();
        out.extend(self.reports.iter().flat_map(|r| r.diags.iter().cloned()));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::module;
    use crate::tier1::ObKind;

    #[test]
    fn test_every_generated_function_has_20_obligations_and_the_module_no_diagnostic() {
        let m = Module::load(&module(20, 3)).unwrap();
        for (f, r) in m.funcs.iter().zip(&m.reports) {
            assert_eq!(
                r.obligations.len(),
                20,
                "{}: {:?}",
                f.sig.name,
                r.obligations
            );
            let unproven: Vec<_> = r.obligations.iter().filter(|o| !o.proven).collect();
            // Only the addition to the i64 sum stays a run-time check.
            assert_eq!(unproven.len(), 1, "{}: {unproven:?}", f.sig.name);
            assert!(unproven.iter().all(|o| o.kind == ObKind::Narrow && !o.must));
        }
        assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    }

    #[test]
    fn test_an_edit_and_its_inverse_restore_the_module() {
        let src = module(5, 11);
        let mut m = Module::load(&src).unwrap();
        // `requires lo < hi` of the callee is no longer established once the guard is weakened.
        let at = src.find("guard lo + 1 < hi").unwrap() + "guard lo + 1 ".len();
        let o = m.edit(at, at + 1, "<=");
        assert_eq!(o.rechecked, 1);
        assert!(o.must_failures >= 1, "{o:?}");
        assert!(
            m.diagnostics()
                .iter()
                .any(|d| d.message.contains("requires lo < hi of f")),
            "{:?}",
            m.diagnostics()
        );
        let back = m.edit(at, at + 2, "<");
        assert_eq!(back.must_failures, 0, "{back:?}");
        assert_eq!(m.src, src);
        assert!(m.diagnostics().is_empty());
    }

    #[test]
    fn test_a_contract_edit_rechecks_the_callers_and_a_body_edit_does_not() {
        let src = module(5, 13);
        let mut m = Module::load(&src).unwrap();
        // f2 calls f1. Weakening f1's `requires` changes its header, so f2 is re-checked too.
        let f1 = m.funcs[1].span;
        let at =
            f1.start + src[f1.start..].find("requires lo < hi").unwrap() + "requires lo ".len();
        let o = m.edit(at, at + 1, "<=");
        assert_eq!(o.rechecked, 2, "{o:?}");
        assert_eq!(o.must_failures, 0, "{o:?}");
        let body =
            f1.start + src[f1.start..].find("var s: i64 = 0").unwrap() + "var s: i64 = ".len();
        let o = m.edit(body + 1, body + 2, "1");
        assert_eq!(o.rechecked, 1, "{o:?}");
    }

    #[test]
    fn test_a_parse_error_is_reported_and_undone() {
        let src = module(3, 5);
        let mut m = Module::load(&src).unwrap();
        let at = src.find("var s").unwrap();
        let o = m.edit(at, at + 3, "+");
        assert!(o.parse_error);
        assert_eq!(m.diagnostics().len(), 1);
        m.edit(at, at + 1, "var");
        assert!(m.diagnostics().is_empty());
        assert_eq!(m.src, src);
    }

    #[test]
    fn test_both_architectures_agree_on_every_edit() {
        let src = module(8, 9);
        let mut a = Module::load(&src).unwrap();
        let mut b = Module::load(&src).unwrap();
        let mut r = crate::generate::Rng(77);
        for _ in 0..100 {
            let f = &a.funcs[r.below(a.funcs.len())];
            let e = crate::generate::random_edit(&a.src, f.body_start, f.span.end, &mut r).unwrap();
            let old = a.src[e.start..e.end].to_string();
            let x = a.edit(e.start, e.end, &e.text);
            let y = b.edit_reparsing_module(e.start, e.end, &e.text);
            assert_eq!(
                (x.parse_error, x.must_failures),
                (y.parse_error, y.must_failures),
                "{e:?}"
            );
            a.edit(e.start, e.start + e.text.len(), &old);
            b.edit_reparsing_module(e.start, e.start + e.text.len(), &old);
            assert_eq!(a.src, src);
        }
    }
}
