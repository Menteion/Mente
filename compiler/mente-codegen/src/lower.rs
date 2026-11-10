//! Lowering a checked module into MIR (`spec/mir-and-codegen.md` item 2).
//!
//! Each function with a body becomes a MIR function, and each also gets an entry, which takes
//! arguments from outside the program, each already narrowed into its parameter's type by
//! whoever calls the entry ([`crate::native::refusal`]), checks the rest as a call does, and
//! then calls it. The lowering follows
//! the reference interpreter (`compiler/mente-interp`) step for step, so that the MIR
//! interpreter and native code trap where it does and say what it says:
//!
//! - integer arithmetic is exact, each operation in a machine type wide enough for its static
//!   range (`core-calculus.md` item 4), and an operation with an `i128` or `u128` operand is
//!   checked at the operator;
//! - a value is narrowed, and checked against its refinement, where it lands;
//! - at a call, the caller narrows the arguments at their own positions, checks the
//!   parameters' refinements, the callee's `requires` at the call, and its `decreases` measure,
//!   because those traps are located in the caller; the callee checks its `ensures` at each
//!   return;
//! - a `var` and every array live in memory, and a `mut` argument is the address of its place.
//!
//! Every check is kept: erasing those Tier 1 proves is a separate step.

use mente_mir::cfg::{
    self, ArithOp, BitOp, BlockId, Builder, CmpOp, FuncId, IntTy, Loc, Num, Op, Term, Ty,
};
use mente_mir::NodeId;
use mente_sema::names::BindingKind;
use mente_sema::tier1::{Obligation, ObligationKind, Tag};
use mente_sema::types::{Int as SemaInt, IntTy as SemaIntTy, Ty as SemaTy};
use mente_sema::wide::{self, Interval};
use mente_sema::{CheckedFn, CheckedModule};
use mente_syntax::tree::{
    AssignOp, BinaryOp, BlockId as AstBlock, Body, Bound, ContractKind, Else, ExprId, ExprKind,
    Refinement, StmtId, StmtKind, Tree, TypeId, TypeKind, UnaryOp,
};
use mente_syntax::Span;

/// A lowered module: its MIR, and for each function with a body its entry.
#[derive(Clone, Debug)]
pub struct Lowered {
    pub module: cfg::Module,
    pub entries: Vec<Entry>,
}

/// What calling a function from outside the program takes: its entry, and the shapes of its
/// parameters, each with whether it is `mut`, and of its result.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub func: FuncId,
    pub params: Vec<(bool, Shape)>,
    pub result: Shape,
    /// The function's name in the source, where a call from outside the program reports an
    /// argument its parameter's type cannot hold.
    pub at: Loc,
}

/// The shape of a value: an integer of a type, a boolean, an array, or nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    Int(SemaIntTy),
    Bool,
    Array(u32, Box<Shape>),
    Unit,
}

impl Shape {
    pub fn of(t: &SemaTy) -> Result<Shape, String> {
        Ok(match t {
            SemaTy::Int(i) => Shape::Int(int_ty(i).unwrap_or(I64)),
            SemaTy::Bool => Shape::Bool,
            SemaTy::Array(n, e) => Shape::Array(
                u32::try_from(*n).map_err(|_| format!("an array of {} elements", n))?,
                Box::new(Shape::of(e)?),
            ),
            SemaTy::Unit => Shape::Unit,
            SemaTy::Any => return Err("a value of no type".into()),
        })
    }

    /// Bytes in memory.
    pub fn size(&self) -> u32 {
        match self {
            Shape::Int(t) => t.bits / 8,
            Shape::Bool => 1,
            Shape::Array(n, e) => n * e.size(),
            Shape::Unit => 0,
        }
    }

    /// The MIR type of a value of this shape: an array is the address of its first element.
    pub fn ty(&self) -> Ty {
        match self {
            Shape::Int(t) => Ty::Int(mir(*t)),
            Shape::Bool => Ty::Bool,
            Shape::Array(..) => Ty::Addr,
            Shape::Unit => Ty::Unit,
        }
    }
}

const I64: SemaIntTy = SemaIntTy {
    name: "i64",
    signed: true,
    bits: 64,
};
const USIZE: IntTy = IntTy::new(64, false);
/// The type a loop keeps its measure in, which is never negative there.
const MEASURE: IntTy = IntTy::new(128, false);

fn int_ty(i: &SemaInt) -> Option<SemaIntTy> {
    match i {
        SemaInt::One(t) | SemaInt::Mixed(Some(t)) => Some(*t),
        _ => None,
    }
}

/// The MIR type of an integer type.
pub fn mir(t: SemaIntTy) -> IntTy {
    IntTy::new(t.bits as u8, t.signed)
}

/// The narrowest of `i64`, `u64`, `i128` and `u128` that holds every value from `lo` to `hi`.
fn machine(lo: Num, hi: Num) -> Option<IntTy> {
    [
        IntTy::new(64, true),
        IntTy::new(64, false),
        IntTy::new(128, true),
        IntTy::new(128, false),
    ]
    .into_iter()
    .find(|t| t.holds(lo) && t.holds(hi))
}

/// A lowered value.
#[derive(Clone, Copy, Debug)]
enum V {
    Int(NodeId, IntTy),
    Bool(NodeId),
    /// An array: the address of its first element.
    Arr(NodeId),
    Unit,
    /// Control left: a trap, a return, a `break` or a `continue`.
    Never,
}

/// Where a binding's value is: an SSA value, or memory of a shape.
#[derive(Clone, Debug)]
enum Local {
    Value(V),
    Place(NodeId, Shape),
}

/// How a function is called: its MIR function, its parameters as `(mut, shape)`, and its
/// result.
#[derive(Clone, Debug)]
struct Sig {
    func: FuncId,
    params: Vec<(bool, Shape)>,
    result: Shape,
}

/// An argument of a call: a value with the span it is narrowed at and its expression's shape,
/// or for a `mut` parameter the address of a place, with its declared type.
#[derive(Clone, Debug)]
enum Arg {
    Value(V, Span, Shape),
    Place(NodeId, Shape, Option<TypeId>, Span),
}

/// Lowers every function of `m` that has a body, and gives each its entry.
pub fn lower(m: &CheckedModule) -> Result<Lowered, String> {
    let mut sigs = Vec::new();
    let mut count = 0u32;
    for f in &m.functions {
        sigs.push(match f {
            Some(f) => {
                let tree = &f.syntax.tree;
                let params = f
                    .syntax
                    .params
                    .iter()
                    .map(|p| Ok((p.is_mut, Shape::of(&SemaTy::of(tree, p.ty))?)))
                    .collect::<Result<Vec<_>, String>>()?;
                let result = match f.syntax.result {
                    Some(t) => Shape::of(&SemaTy::of(tree, t))?,
                    None => Shape::Unit,
                };
                count += 1;
                Some(Sig {
                    func: FuncId(count - 1),
                    params,
                    result,
                })
            }
            None => None,
        });
    }
    let mut module = cfg::Module::default();
    for (i, f) in m.functions.iter().enumerate() {
        if let Some(f) = f {
            let lowered = Lower::new(m, &sigs, i, false)?
                .function()
                .map_err(|e| format!("fn {}: {}", f.syntax.name, e))?;
            module.functions.push(lowered);
        }
    }
    let mut entries = Vec::new();
    for (i, f) in m.functions.iter().enumerate() {
        if let (Some(f), Some(sig)) = (f, &sigs[i]) {
            let func = FuncId(module.functions.len() as u32);
            let entry = Lower::new(m, &sigs, i, true)?
                .entry()
                .map_err(|e| format!("entry of {}: {}", f.syntax.name, e))?;
            module.functions.push(entry);
            entries.push(Entry {
                name: f.syntax.name.clone(),
                func,
                params: sig.params.clone(),
                result: sig.result.clone(),
                at: Loc {
                    line: f.syntax.name_span.line,
                    col: f.syntax.name_span.col,
                },
            });
        }
    }
    cfg::verify(&module)?;
    Ok(Lowered { module, entries })
}

/// The MIR parameters of a function of `sig`: an array result's address first, then each
/// parameter, a `mut` one or an array as an address; and its MIR result.
fn mir_params(sig: &Sig) -> (Vec<Ty>, Option<Ty>) {
    let mut params = Vec::new();
    if matches!(sig.result, Shape::Array(..)) {
        params.push(Ty::Addr);
    }
    for (is_mut, shape) in &sig.params {
        params.push(if *is_mut { Ty::Addr } else { shape.ty() });
    }
    let result = match &sig.result {
        Shape::Int(_) | Shape::Bool => Some(sig.result.ty()),
        _ => None,
    };
    (params, result)
}

/// The context expressions are lowered in: a function's syntax, the static ranges of its
/// expressions and where its bindings are. A callee's contract clauses are lowered in the
/// callee's context, its parameters bound to the call's arguments.
#[derive(Clone)]
struct Scope<'a> {
    index: usize,
    f: &'a CheckedFn,
    iv: Vec<Option<Interval>>,
    locals: Vec<Option<Local>>,
}

impl<'a> Scope<'a> {
    fn new(m: &'a CheckedModule, index: usize) -> Result<Scope<'a>, String> {
        let f = m.functions[index]
            .as_ref()
            .ok_or("a call of a function without a body")?;
        Ok(Scope {
            index,
            f,
            iv: wide::intervals(&f.syntax, &f.resolution, &f.typing),
            locals: vec![None; f.resolution.bindings.len()],
        })
    }
}

/// Where Tier 1 records the obligation of a check: its place, or any place for a written
/// invariant, whose conjuncts it records each at its own; its tag; and whether it is one of a
/// call's own checks, made in the callee's scope but recorded in the caller's.
#[derive(Clone, Copy)]
struct Key {
    at: Option<Span>,
    tag: Tag,
    call: bool,
}

impl Key {
    fn at(at: Span, tag: Tag) -> Key {
        Key {
            at: Some(at),
            tag,
            call: false,
        }
    }

    /// A call's own check, at the call or an argument.
    fn call(at: Span, tag: Tag) -> Key {
        Key {
            at: Some(at),
            tag,
            call: true,
        }
    }

    /// A written invariant, `clause`.
    fn invariant(clause: Span) -> Key {
        Key {
            at: None,
            tag: Tag::Clause(clause),
            call: false,
        }
    }
}

/// An index of a place: its value, the span of its indexing, where it traps, and the span of the
/// index itself, where Tier 1 records it.
type Indexing = (V, Span, Span);

/// One function's lowering.
struct Lower<'a> {
    m: &'a CheckedModule,
    sigs: &'a [Option<Sig>],
    s: Scope<'a>,
    b: Builder,
    /// Whether each block has its terminator; one that never gets one is unreachable.
    ended: Vec<bool>,
    /// The enclosing loops' heads and exits.
    loops: Vec<(BlockId, BlockId)>,
    /// Where an array result goes.
    sret: Option<NodeId>,
    /// This function's `decreases` measure on entry, for its recursive calls.
    measure: Option<Vec<V>>,
    /// What Tier 1 recorded in the function, which tells the checks it proves; nothing for an
    /// entry, whose arguments come from outside the program.
    obligations: &'a [Obligation],
    /// Whether the lowering is making a call's checks in its callee's scope, whose checks are
    /// never left out.
    in_callee: bool,
}

impl<'a> Lower<'a> {
    fn new(
        m: &'a CheckedModule,
        sigs: &'a [Option<Sig>],
        index: usize,
        entry: bool,
    ) -> Result<Lower<'a>, String> {
        let s = Scope::new(m, index)?;
        let sig = sigs[index]
            .as_ref()
            .ok_or("a function without a signature")?;
        let (params, result) = mir_params(sig);
        let obligations: &'a [Obligation] = if entry { &[] } else { &s.f.obligations };
        let decl = s.f.syntax.name_span;
        let b = Builder::declared_at(
            &s.f.syntax.name,
            &params,
            result,
            Loc {
                line: decl.line,
                col: decl.col,
            },
        );
        Ok(Lower {
            m,
            sigs,
            s,
            b,
            ended: vec![false],
            loops: Vec::new(),
            sret: None,
            measure: None,
            obligations,
            in_callee: false,
        })
    }

    /// Whether Tier 1 proves the check of `kind` it records under `key`: some obligation has
    /// that kind, place and tag, and every one that does is proven. In a callee's scope, only a
    /// call's own checks; in an entry, which has no obligations, none.
    fn proven(&self, kind: ObligationKind, key: Key) -> bool {
        if self.in_callee && !key.call {
            return false;
        }
        let mut found = self
            .obligations
            .iter()
            .filter(|o| o.kind == kind && key.at.is_none_or(|at| o.span == at) && o.tag == key.tag)
            .peekable();
        found.peek().is_some() && found.all(|o| o.proven)
    }

    fn sig_of(&self, index: usize) -> Result<&'a Sig, String> {
        self.sigs[index]
            .as_ref()
            .ok_or_else(|| "a call of a function without a body".to_string())
    }

    fn tree(&self) -> &'a Tree {
        &self.s.f.syntax.tree
    }

    fn at(&mut self, span: Span) {
        self.b.at(span.line, span.col);
    }

    fn block(&mut self, params: &[Ty]) -> BlockId {
        let id = self.b.block(params);
        self.ended.push(false);
        id
    }

    fn end(&mut self, term: Term) {
        self.ended[self.b.current().0 as usize] = true;
        self.b.end(term);
    }

    /// Continues in a new block nothing jumps to, after control has left.
    fn dead(&mut self) -> V {
        let dead = self.block(&[]);
        self.b.switch(dead);
        V::Never
    }

    /// Ends the current block in a trap at `span`.
    fn trap(&mut self, kind: &str, text: &str, span: Span) -> V {
        self.at(span);
        let site = self.b.trap_site(kind, &[text]);
        self.end(Term::Trap(site));
        self.dead()
    }

    fn jump(&mut self, to: BlockId, args: Vec<NodeId>) {
        self.end(Term::Jump(to, args));
    }

    fn int(&mut self, t: IntTy, n: Num) -> NodeId {
        self.b.int(t, n)
    }

    fn inst(&mut self, op: Op, ty: Ty, args: &[NodeId]) -> NodeId {
        self.b.inst(op, ty, args)
    }

    fn bool(&mut self, b: bool) -> NodeId {
        self.inst(Op::Bool(b), Ty::Bool, &[])
    }

    /// Traps at `span` with the pieces of `text` around `values` unless `cond` holds.
    fn check(&mut self, cond: NodeId, kind: &str, text: &[&str], values: &[NodeId], span: Span) {
        self.at(span);
        let site = self.b.trap_site(kind, text);
        let mut args = vec![cond];
        args.extend_from_slice(values);
        self.inst(Op::Check { trap: site }, Ty::Unit, &args);
    }

    fn finish(mut self) -> cfg::Function {
        let unreachable = self.b.trap_site(
            "unreachable",
            &["unreachable reached: control left the code the checker allows"],
        );
        for i in 0..self.ended.len() {
            if !self.ended[i] {
                self.b.switch(BlockId(i as u32));
                self.b.end(Term::Trap(unreachable));
            }
        }
        self.b.finish()
    }

    // -- functions -------------------------------------------------------------------------

    fn function(mut self) -> Result<cfg::Function, String> {
        let sig = self.sig_of(self.s.index)?;
        let mut params = self.b.params(BlockId(0)).into_iter();
        if matches!(sig.result, Shape::Array(..)) {
            self.sret = params.next();
        }
        for (i, (is_mut, shape)) in sig.params.iter().enumerate() {
            let p = params.next().ok_or("too few parameters")?;
            self.s.locals[i] = Some(match (is_mut, shape) {
                (false, Shape::Int(t)) => Local::Value(V::Int(p, mir(*t))),
                (false, Shape::Bool) => Local::Value(V::Bool(p)),
                (_, shape) => Local::Place(p, shape.clone()),
            });
        }
        // The measure on entry, unchecked: the caller checked it.
        let clauses = self.clauses(ContractKind::Decreases);
        if !clauses.is_empty() {
            let mut values = Vec::new();
            for c in clauses {
                values.push(self.expr(c)?);
            }
            self.measure = Some(values);
        }
        let tree = self.tree();
        match self.s.f.syntax.body {
            Some(Body::Expr(e)) => {
                let v = self.expr(e)?;
                let shape = self.shape(e)?;
                self.ret(v, shape, tree.expr(e).span)?;
            }
            Some(Body::Block(b)) => {
                let block = tree.block(b);
                let span = block
                    .stmts
                    .last()
                    .map_or(block.span, |&s| tree.stmt(s).span);
                let shape = match block.stmts.last().map(|&s| &tree.stmt(s).kind) {
                    Some(StmtKind::Expr(e)) => self.shape(*e)?,
                    _ => Shape::Unit,
                };
                let v = self.body(b)?;
                self.ret(v, shape, span)?;
            }
            None => {
                self.ret(V::Unit, Shape::Unit, self.s.f.syntax.name_span)?;
            }
        }
        Ok(self.finish())
    }

    fn clauses(&self, kind: ContractKind) -> Vec<ExprId> {
        self.s
            .f
            .syntax
            .contracts
            .iter()
            .filter(|c| c.kind == kind)
            .map(|c| c.expr)
            .collect()
    }

    /// The entry: a function taking what the lowered one takes, each argument already in its
    /// parameter's type, which checks the rest as a call does, at the function's name, and calls
    /// it.
    fn entry(mut self) -> Result<cfg::Function, String> {
        let span = self.s.f.syntax.name_span;
        let sig = self.sig_of(self.s.index)?;
        let mut params = self.b.params(BlockId(0)).into_iter();
        let sret = if matches!(sig.result, Shape::Array(..)) {
            params.next()
        } else {
            None
        };
        let mut args = Vec::new();
        for (i, (is_mut, shape)) in sig.params.iter().enumerate() {
            let p = params.next().ok_or("too few parameters")?;
            let declared = Some(self.s.f.syntax.params[i].ty);
            args.push(match (is_mut, shape) {
                (false, Shape::Int(t)) => Arg::Value(V::Int(p, mir(*t)), span, shape.clone()),
                (false, Shape::Bool) => Arg::Value(V::Bool(p), span, shape.clone()),
                (false, _) => Arg::Value(V::Arr(p), span, shape.clone()),
                (true, shape) => Arg::Place(p, shape.clone(), declared, span),
            });
        }
        let v = self.call(self.s.index, args, span, sret, false)?;
        let term = match v {
            V::Int(n, _) | V::Bool(n) => Term::Return(Some(n)),
            _ => Term::Return(None),
        };
        self.end(term);
        let mut f = self.finish();
        f.name = format!("enter {}", f.name);
        Ok(f)
    }

    /// Returns `v`, whose expression has `shape`, from this function: narrowed into the result
    /// type at `span`, checked against its refinement, and against each `ensures` at `span`.
    fn ret(&mut self, v: V, shape: Shape, span: Span) -> Result<(), String> {
        if matches!(v, V::Never) {
            return Ok(());
        }
        let result = self.sig_of(self.s.index)?.result.clone();
        let declared = self.s.f.syntax.result;
        let v = self.land(v, &shape, &result, declared, span)?;
        let tree = self.tree();
        for c in self.clauses(ContractKind::Ensures) {
            // Each `ensures` clause has its own `result` binding, declared at the clause.
            let clause = tree.expr(c).span;
            if let Some(id) = self
                .s
                .f
                .resolution
                .bindings
                .iter()
                .position(|b| b.kind == BindingKind::Result && b.span == clause)
            {
                self.s.locals[id] = Some(match v {
                    V::Arr(a) => Local::Place(a, result.clone()),
                    v => Local::Value(v),
                });
            }
            let text = format!("`ensures {}` is false at this return", tree.render_expr(c));
            let key = Key::at(span, Tag::Clause(tree.expr(c).span));
            self.clause(c, ("ensures", ObligationKind::Ensures), &text, span, key)?;
        }
        match v {
            V::Int(n, _) | V::Bool(n) => self.end(Term::Return(Some(n))),
            V::Arr(a) => {
                let to = self.sret.ok_or("an array result with nowhere to go")?;
                self.at(span);
                self.inst(
                    Op::Copy {
                        bytes: result.size(),
                    },
                    Ty::Unit,
                    &[to, a],
                );
                self.end(Term::Return(None));
            }
            _ => self.end(Term::Return(None)),
        }
        self.dead();
        Ok(())
    }

    /// Traps at `span` with `text` unless the clause `c`, of the current scope, holds, or
    /// Tier 1 proves it under `key`. The clause is evaluated either way, so that a trap inside
    /// it stays where the reference interpreter has it.
    fn clause(
        &mut self,
        c: ExprId,
        (kind, obligation): (&str, ObligationKind),
        text: &str,
        span: Span,
        key: Key,
    ) -> Result<(), String> {
        if let V::Bool(cond) = self.expr(c)? {
            if !self.proven(obligation, key) {
                self.check(cond, kind, &[text], &[], span);
            }
        }
        Ok(())
    }

    // -- statements ------------------------------------------------------------------------

    /// The value of a block: its last statement's, when that is an expression.
    fn body(&mut self, b: AstBlock) -> Result<V, String> {
        let tree = self.tree();
        let stmts = &tree.block(b).stmts;
        let mut last = V::Unit;
        for (i, &s) in stmts.iter().enumerate() {
            last = self.stmt(s, i + 1 == stmts.len())?;
            if matches!(last, V::Never) {
                return Ok(V::Never);
            }
        }
        Ok(last)
    }

    fn stmt(&mut self, s: StmtId, last: bool) -> Result<V, String> {
        let tree = self.tree();
        let span = tree.stmt(s).span;
        self.at(span);
        match &tree.stmt(s).kind {
            StmtKind::Expr(e) => {
                let v = self.expr(*e)?;
                return Ok(if last || matches!(v, V::Never) {
                    v
                } else {
                    V::Unit
                });
            }
            StmtKind::Discard(e) => {
                if matches!(self.expr(*e)?, V::Never) {
                    return Ok(V::Never);
                }
            }
            StmtKind::Binding { ty, init, .. } => {
                let Some(id) = self.s.f.resolution.stmts[s.0 as usize] else {
                    return Ok(V::Unit);
                };
                let id = id.0 as usize;
                let shape = Shape::of(&self.s.f.typing.bindings[id])?;
                let v = self.expr(*init)?;
                let from = self.shape(*init)?;
                let v = self.land(v, &from, &shape, *ty, tree.expr(*init).span)?;
                if matches!(v, V::Never) {
                    return Ok(V::Never);
                }
                let is_var = self.s.f.resolution.bindings[id].kind == BindingKind::Var;
                let local = match (v, is_var) {
                    (V::Arr(a), _) => Local::Place(self.copy(a, &shape), shape),
                    (v, false) => Local::Value(v),
                    (v, true) => {
                        let slot = self.inst(
                            Op::Slot {
                                bytes: shape.size(),
                            },
                            Ty::Addr,
                            &[],
                        );
                        self.store(slot, v);
                        Local::Place(slot, shape)
                    }
                };
                self.s.locals[id] = Some(local);
            }
            StmtKind::Assign { place, op, value } => return self.assign(*place, *op, *value),
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => return self.while_loop(*cond, invariants, decreases, *body),
            StmtKind::Guard { cond, otherwise } => {
                let V::Bool(c) = self.expr(*cond)? else {
                    return Ok(V::Never);
                };
                let pass = self.block(&[]);
                let fail = self.block(&[]);
                self.end(Term::Branch {
                    cond: c,
                    then: (pass, vec![]),
                    otherwise: (fail, vec![]),
                });
                self.b.switch(fail);
                // The `else` leaves, so its end is never reached.
                self.body(*otherwise)?;
                self.b.switch(pass);
            }
            StmtKind::Return(value) => {
                match value {
                    Some(e) => {
                        let v = self.expr(*e)?;
                        let shape = self.shape(*e)?;
                        self.ret(v, shape, tree.expr(*e).span)?;
                    }
                    None => self.ret(V::Unit, Shape::Unit, span)?,
                }
                return Ok(V::Never);
            }
            StmtKind::Break | StmtKind::Continue => {
                let (head, exit) = *self.loops.last().ok_or("a jump outside a loop")?;
                let to = if matches!(tree.stmt(s).kind, StmtKind::Break) {
                    exit
                } else {
                    head
                };
                self.jump(to, vec![]);
                return Ok(self.dead());
            }
        }
        Ok(V::Unit)
    }

    fn while_loop(
        &mut self,
        cond: ExprId,
        invariants: &[ExprId],
        decreases: &[ExprId],
        body: AstBlock,
    ) -> Result<V, String> {
        let tree = self.tree();
        // The measure of the round before, in memory, and whether there was one.
        let before = if decreases.is_empty() {
            None
        } else {
            let seen = self.inst(Op::Slot { bytes: 1 }, Ty::Addr, &[]);
            let no = self.bool(false);
            self.inst(Op::Store, Ty::Unit, &[seen, no]);
            let slots: Vec<NodeId> = decreases
                .iter()
                .map(|_| self.inst(Op::Slot { bytes: 16 }, Ty::Addr, &[]))
                .collect();
            Some((seen, slots))
        };
        let head = self.block(&[]);
        let round = self.block(&[]);
        let exit = self.block(&[]);
        self.jump(head, vec![]);
        self.b.switch(head);
        for &i in invariants {
            let text = format!(
                "`invariant {}` is false at the loop head",
                tree.render_expr(i)
            );
            let (at, key) = (tree.expr(i).span, Key::invariant(tree.expr(i).span));
            self.clause(i, ("invariant", ObligationKind::Invariant), &text, at, key)?;
        }
        let V::Bool(c) = self.expr(cond)? else {
            return Ok(V::Never);
        };
        self.end(Term::Branch {
            cond: c,
            then: (round, vec![]),
            otherwise: (exit, vec![]),
        });
        self.b.switch(round);
        if let (Some((seen, slots)), Some(&first)) = (before, decreases.first()) {
            let now = self.measure(decreases, None)?;
            // A measure of any integer type fits u128 once it is not negative.
            let prev: Vec<V> = slots
                .iter()
                .map(|s| V::Int(self.inst(Op::Load, Ty::Int(MEASURE), &[*s]), MEASURE))
                .collect();
            let smaller = self.lex_less(&now, &prev)?;
            let had = self.inst(Op::Load, Ty::Bool, &[seen]);
            let first_round = self.inst(Op::Not, Ty::Bool, &[had]);
            let ok = self.inst(Op::Bits(BitOp::Or), Ty::Bool, &[first_round, smaller]);
            let text = format!(
                "`decreases {}` is negative or did not decrease",
                tree.render_expr(first)
            );
            let fspan = tree.expr(first).span;
            if !self.proven(ObligationKind::Decreases, Key::at(fspan, Tag::Smaller)) {
                self.check(ok, "decreases", &[&text], &[], fspan);
            }
            let yes = self.bool(true);
            self.inst(Op::Store, Ty::Unit, &[seen, yes]);
            for (v, s) in now.iter().zip(&slots) {
                if let V::Int(n, t) = v {
                    let w = self.cast(*n, *t, MEASURE);
                    self.inst(Op::Store, Ty::Unit, &[*s, w]);
                }
            }
        }
        self.loops.push((head, exit));
        let v = self.body(body)?;
        self.loops.pop();
        if !matches!(v, V::Never) {
            self.jump(head, vec![]);
        }
        self.b.switch(exit);
        Ok(V::Unit)
    }

    /// The values of a measure, each checked not to be negative at its clause, unless Tier 1
    /// proves it: at the clause in a loop, at the call `call` for a callee's.
    fn measure(&mut self, exprs: &[ExprId], call: Option<Span>) -> Result<Vec<V>, String> {
        let tree = self.tree();
        let mut values = Vec::new();
        for &e in exprs {
            let v = self.expr(e)?;
            let clause = Tag::Clause(tree.expr(e).span);
            let key = call.map_or(Key::at(tree.expr(e).span, clause), |at| {
                Key::call(at, clause)
            });
            if let (V::Int(n, t), false) = (v, self.proven(ObligationKind::Decreases, key)) {
                let zero = self.int(t, Num::ZERO);
                let ok = self.inst(Op::Cmp(CmpOp::Ge), Ty::Bool, &[n, zero]);
                let text = format!(
                    "`decreases {}` is negative or did not decrease",
                    tree.render_expr(e)
                );
                self.check(ok, "decreases", &[&text], &[], tree.expr(e).span);
            }
            values.push(v);
        }
        Ok(values)
    }

    /// Whether `a` is smaller than `b` in lexicographic order, both of one length.
    fn lex_less(&mut self, a: &[V], b: &[V]) -> Result<NodeId, String> {
        let mut result = self.bool(false);
        // From the last part back: a < b when a0 < b0, or a0 == b0 and the rest is smaller.
        for (x, y) in a.iter().zip(b).rev() {
            let (V::Int(x, tx), V::Int(y, ty)) = (x, y) else {
                return Err("a measure that is not an integer".into());
            };
            let lt = self.compare(CmpOp::Lt, (*x, *tx), (*y, *ty))?;
            let eq = self.compare(CmpOp::Eq, (*x, *tx), (*y, *ty))?;
            let rest = self.inst(Op::Bits(BitOp::And), Ty::Bool, &[eq, result]);
            result = self.inst(Op::Bits(BitOp::Or), Ty::Bool, &[lt, rest]);
        }
        Ok(result)
    }

    fn assign(&mut self, place: ExprId, op: AssignOp, value: ExprId) -> Result<V, String> {
        let tree = self.tree();
        let span = tree.expr(value).span;
        let (root, indexes) = self.place_path(place)?;
        let shape = self.shape(place)?;
        let declared = self.place_type(root, indexes.len());
        let (new, from) = match op {
            AssignOp::Set => (self.expr(value)?, self.shape(value)?),
            AssignOp::Compound(op) => {
                let at = self.address(root, &indexes)?;
                let current = self.load(at, &shape);
                let rhs = self.expr(value)?;
                let iv = Self::place_interval(&shape);
                let viv = self.interval(value);
                let combined = match (iv, viv) {
                    (Some(a), Some(b)) => wide::combine(op, a, b),
                    _ => None,
                };
                let v = self.binary(op, (current, iv, place), (rhs, viv, value), combined, span)?;
                (v, shape.clone())
            }
        };
        let new = self.land(new, &from, &shape, declared, span)?;
        if matches!(new, V::Never) {
            return Ok(V::Never);
        }
        let at = self.address(root, &indexes)?;
        self.store_shaped(at, new, &shape);
        Ok(V::Unit)
    }

    /// The root binding of a place and its indexes, evaluated left to right, each with the
    /// span of its indexing and of the index itself.
    fn place_path(&mut self, e: ExprId) -> Result<(usize, Vec<Indexing>), String> {
        let tree = self.tree();
        match &tree.expr(e).kind {
            ExprKind::Index { base, index } => {
                let (root, mut indexes) = self.place_path(*base)?;
                let i = self.expr(*index)?;
                indexes.push((i, tree.expr(e).span, tree.expr(*index).span));
                Ok((root, indexes))
            }
            _ => Ok((
                self.s.f.resolution.names[e.0 as usize]
                    .map(|b| b.0 as usize)
                    .ok_or("a place without a binding")?,
                Vec::new(),
            )),
        }
    }

    /// The declared type of a place, when its root binding states one.
    fn place_type(&self, root: usize, depth: usize) -> Option<TypeId> {
        let tree = self.tree();
        let mut ty = self.s.f.resolution.bindings.get(root)?.ty?;
        for _ in 0..depth {
            match &tree.ty(ty).kind {
                TypeKind::Array { elem, .. } => ty = *elem,
                _ => return None,
            }
        }
        Some(ty)
    }

    /// The address of a place, each index checked against its array's length.
    fn address(&mut self, root: usize, indexes: &[Indexing]) -> Result<NodeId, String> {
        let local = self.s.locals[root].clone().ok_or("a place never bound")?;
        let Local::Place(mut at, mut shape) = local else {
            return Err("a place that is not in memory".into());
        };
        for (i, span, key) in indexes {
            let Shape::Array(len, elem) = shape else {
                return Err("an index into a value that is not an array".into());
            };
            at = self.element(at, len, &elem, *i, (*span, *key))?;
            shape = *elem;
        }
        Ok(at)
    }

    /// The address of element `i` of an array of `len` elements of `elem` at `base`; an index
    /// outside it traps at `span`, unless Tier 1 proves at `key`, the index's own span, that it
    /// lies inside.
    fn element(
        &mut self,
        base: NodeId,
        len: u32,
        elem: &Shape,
        i: V,
        (span, key): (Span, Span),
    ) -> Result<NodeId, String> {
        let V::Int(i, t) = i else {
            return Err("an index that is not an integer".into());
        };
        self.at(span);
        if !self.proven(ObligationKind::Index, Key::at(key, Tag::None)) {
            let zero = self.int(t, Num::ZERO);
            let ge = self.inst(Op::Cmp(CmpOp::Ge), Ty::Bool, &[i, zero]);
            let n = Num::new(false, u128::from(len));
            let below = if t.holds(n) {
                let n = self.int(t, n);
                self.inst(Op::Cmp(CmpOp::Lt), Ty::Bool, &[i, n])
            } else {
                self.bool(true)
            };
            let ok = self.inst(Op::Bits(BitOp::And), Ty::Bool, &[ge, below]);
            let after = format!(" is outside an array of length {}", len);
            self.check(ok, "index-out-of-bounds", &["index ", &after], &[i], span);
        }
        let index = self.cast(i, t, USIZE);
        Ok(self.inst(
            Op::Elem {
                stride: elem.size(),
            },
            Ty::Addr,
            &[base, index],
        ))
    }

    // -- memory ----------------------------------------------------------------------------

    fn load(&mut self, at: NodeId, shape: &Shape) -> V {
        match shape {
            Shape::Int(t) => V::Int(self.inst(Op::Load, Ty::Int(mir(*t)), &[at]), mir(*t)),
            Shape::Bool => V::Bool(self.inst(Op::Load, Ty::Bool, &[at])),
            Shape::Array(..) => V::Arr(at),
            Shape::Unit => V::Unit,
        }
    }

    fn store(&mut self, at: NodeId, v: V) {
        if let V::Int(n, _) | V::Bool(n) = v {
            self.inst(Op::Store, Ty::Unit, &[at, n]);
        }
    }

    fn store_shaped(&mut self, at: NodeId, v: V, shape: &Shape) {
        match v {
            V::Arr(a) => {
                self.inst(
                    Op::Copy {
                        bytes: shape.size(),
                    },
                    Ty::Unit,
                    &[at, a],
                );
            }
            v => self.store(at, v),
        }
    }

    /// A new slot holding a copy of the array at `a`.
    fn copy(&mut self, a: NodeId, shape: &Shape) -> NodeId {
        let bytes = shape.size();
        let slot = self.inst(Op::Slot { bytes }, Ty::Addr, &[]);
        self.inst(Op::Copy { bytes }, Ty::Unit, &[slot, a]);
        slot
    }

    /// Runs `each` on every index of an array of `len` elements, in a loop.
    fn each(
        &mut self,
        len: u32,
        each: &mut dyn FnMut(&mut Lower<'a>, NodeId) -> Result<(), String>,
    ) -> Result<(), String> {
        let head = self.block(&[Ty::Int(USIZE)]);
        let round = self.block(&[]);
        let exit = self.block(&[]);
        let zero = self.int(USIZE, Num::ZERO);
        self.jump(head, vec![zero]);
        self.b.switch(head);
        let i = self.b.params(head)[0];
        let n = self.int(USIZE, Num::new(false, u128::from(len)));
        let more = self.inst(Op::Cmp(CmpOp::Lt), Ty::Bool, &[i, n]);
        self.end(Term::Branch {
            cond: more,
            then: (round, vec![]),
            otherwise: (exit, vec![]),
        });
        self.b.switch(round);
        each(self, i)?;
        let one = self.int(USIZE, Num::new(false, 1));
        let next = self.inst(
            Op::Arith {
                op: ArithOp::Add,
                trap: None,
            },
            Ty::Int(USIZE),
            &[i, one],
        );
        self.jump(head, vec![next]);
        self.b.switch(exit);
        Ok(())
    }

    // -- landing ---------------------------------------------------------------------------

    /// A value, whose expression has shape `from`, landing in a place of shape `to` whose
    /// declared type is `declared`: narrowed into the place's integer types, then checked
    /// against the declared type's refinements, both at `span`.
    fn land(
        &mut self,
        v: V,
        from: &Shape,
        to: &Shape,
        declared: Option<TypeId>,
        span: Span,
    ) -> Result<V, String> {
        let v = self.narrow(v, from, to, span, Some(span))?;
        if let Some(d) = declared {
            self.refined(v, to, d, span, Key::at(span, Tag::None))?;
        }
        Ok(v)
    }

    /// A value of shape `from` narrowed into `to`, elementwise for an array, trapping at
    /// `span`; `key` is where Tier 1 records the narrowing, when it does.
    fn narrow(
        &mut self,
        v: V,
        from: &Shape,
        to: &Shape,
        span: Span,
        key: Option<Span>,
    ) -> Result<V, String> {
        Ok(match (v, from, to) {
            (V::Int(n, t), _, Shape::Int(into)) => {
                V::Int(self.narrow_int(n, t, *into, span, key), mir(*into))
            }
            (V::Arr(a), Shape::Array(_, fe), Shape::Array(len, te)) => {
                // Elements of the same type stay where they are; of another type, they are
                // narrowed into a new array.
                if fe == te {
                    return Ok(V::Arr(a));
                }
                let slot = self.inst(Op::Slot { bytes: to.size() }, Ty::Addr, &[]);
                let (fe, te, len) = ((**fe).clone(), (**te).clone(), *len);
                self.each(len, &mut |l, i| {
                    let src = l.inst(Op::Elem { stride: fe.size() }, Ty::Addr, &[a, i]);
                    let dst = l.inst(Op::Elem { stride: te.size() }, Ty::Addr, &[slot, i]);
                    let x = l.load(src, &fe);
                    let x = l.narrow(x, &fe, &te, span, key)?;
                    l.store_shaped(dst, x, &te);
                    Ok(())
                })?;
                V::Arr(slot)
            }
            (v, _, _) => v,
        })
    }

    /// `n` of type `from` narrowed into `to`: a check that it fits, where it may not and Tier 1
    /// does not prove it at `key`, then a cast.
    fn narrow_int(
        &mut self,
        n: NodeId,
        from: IntTy,
        to: SemaIntTy,
        span: Span,
        key: Option<Span>,
    ) -> NodeId {
        let into = mir(to);
        let proven =
            key.is_some_and(|k| self.proven(ObligationKind::Range, Key::at(k, Tag::Into(to.name))));
        if from == into || proven {
            return self.cast(n, from, into);
        }
        let (lo, hi) = (into.min(), into.max());
        let mut parts = Vec::new();
        // A bound needs a check where the source type reaches past it.
        if !into.holds(from.min()) {
            let c = self.int(from, lo);
            parts.push(self.inst(Op::Cmp(CmpOp::Ge), Ty::Bool, &[n, c]));
        }
        if !into.holds(from.max()) {
            let c = self.int(from, hi);
            parts.push(self.inst(Op::Cmp(CmpOp::Le), Ty::Bool, &[n, c]));
        }
        if let Some(&first) = parts.first() {
            let ok = parts[1..].iter().fold(first, |a, b| {
                self.inst(Op::Bits(BitOp::And), Ty::Bool, &[a, *b])
            });
            let after = format!(" does not fit in `{}`", to.name);
            self.check(ok, "narrowing", &["", &after], &[n], span);
        }
        self.cast(n, from, into)
    }

    fn cast(&mut self, n: NodeId, from: IntTy, to: IntTy) -> NodeId {
        if from == to {
            n
        } else {
            self.inst(Op::Cast, Ty::Int(to), &[n])
        }
    }

    /// Checks a value of `shape` against the refinements of the declared type `d`, elementwise
    /// for an array, trapping at `span`, unless Tier 1 proves under `key` that it fits them.
    fn refined(
        &mut self,
        v: V,
        shape: &Shape,
        d: TypeId,
        span: Span,
        key: Key,
    ) -> Result<(), String> {
        if self.proven(ObligationKind::Refinement, key) {
            return Ok(());
        }
        let tree = self.tree();
        match (v, &tree.ty(d).kind, shape) {
            (
                V::Int(x, t),
                TypeKind::Int {
                    refinement: Some(r),
                    ..
                },
                _,
            ) => self.in_range(x, t, r, span),
            (V::Arr(a), TypeKind::Array { elem, .. }, Shape::Array(len, es)) => {
                if leaf_refinement(tree, *elem).is_none() {
                    return Ok(());
                }
                let (elem, es, len) = (*elem, (**es).clone(), *len);
                self.each(len, &mut |l, i| {
                    let at = l.inst(Op::Elem { stride: es.size() }, Ty::Addr, &[a, i]);
                    let x = l.load(at, &es);
                    l.refined(x, &es, elem, span, key)
                })
            }
            _ => Ok(()),
        }
    }

    fn in_range(&mut self, x: NodeId, t: IntTy, r: &Refinement, span: Span) -> Result<(), String> {
        let lo = self.bound(&r.lo)?;
        let hi = self.bound(&r.hi)?;
        let ge = self.compare(CmpOp::Ge, (x, t), lo)?;
        let below = self.compare(if r.inclusive { CmpOp::Le } else { CmpOp::Lt }, (x, t), hi)?;
        let ok = self.inst(Op::Bits(BitOp::And), Ty::Bool, &[ge, below]);
        let dots = if r.inclusive { "..=" } else { ".." };
        self.check(
            ok,
            "refinement",
            &["", " lies outside the refinement `[", dots, "]`"],
            &[x, lo.0, hi.0],
            span,
        );
        Ok(())
    }

    /// The value of a refinement bound in the current scope.
    fn bound(&mut self, b: &Bound) -> Result<(NodeId, IntTy), String> {
        let slot = |l: &Lower, name: &str| {
            l.s.f
                .resolution
                .bindings
                .iter()
                .position(|binding| binding.name == name)
        };
        Ok(match b {
            Bound::Int { negative, value } => {
                let n = Num::new(*negative, *value);
                let t = machine(n, n).ok_or("a bound beyond 128 bits")?;
                (self.int(t, n), t)
            }
            Bound::Name(n) => match slot(self, n).and_then(|i| self.s.locals[i].clone()) {
                Some(Local::Value(V::Int(v, t))) => (v, t),
                // A bound names a `val` or a read-only parameter, never a place in memory.
                _ => return Err(format!("a bound `{}` that is not an integer value", n)),
            },
            Bound::Len(n) => match slot(self, n).and_then(|i| self.s.locals[i].clone()) {
                Some(Local::Place(_, Shape::Array(len, _))) => {
                    (self.int(USIZE, Num::new(false, u128::from(len))), USIZE)
                }
                _ => return Err(format!("a bound `{}.len` that is not an array's", n)),
            },
        })
    }

    // -- comparisons -----------------------------------------------------------------------

    /// Compares two integers of any types, as exact values.
    fn compare(
        &mut self,
        op: CmpOp,
        (a, ta): (NodeId, IntTy),
        (b, tb): (NodeId, IntTy),
    ) -> Result<NodeId, String> {
        if let Some(c) = common(ta, tb) {
            let a = self.cast(a, ta, c);
            let b = self.cast(b, tb, c);
            return Ok(self.inst(Op::Cmp(op), Ty::Bool, &[a, b]));
        }
        // A u128 against a signed type: a negative value is below every u128, and any other
        // fits u128.
        let (u, s, swapped) = if ta.signed {
            (b, a, true)
        } else {
            (a, b, false)
        };
        let (su, ss) = if swapped { (tb, ta) } else { (ta, tb) };
        let zero = self.int(ss, Num::ZERO);
        let negative = self.inst(Op::Cmp(CmpOp::Lt), Ty::Bool, &[s, zero]);
        let known = self.block(&[]);
        let cast = self.block(&[]);
        let join = self.block(&[Ty::Bool]);
        self.end(Term::Branch {
            cond: negative,
            then: (known, vec![]),
            otherwise: (cast, vec![]),
        });
        self.b.switch(known);
        // With s < u: u > s, u >= s and u != s hold; with the operands swapped, s < u.
        let holds = match op {
            CmpOp::Ne => true,
            CmpOp::Eq => false,
            CmpOp::Gt | CmpOp::Ge => !swapped,
            CmpOp::Lt | CmpOp::Le => swapped,
        };
        let r = self.bool(holds);
        self.jump(join, vec![r]);
        self.b.switch(cast);
        let s = self.cast(s, ss, su);
        let (x, y) = if swapped { (s, u) } else { (u, s) };
        let r = self.inst(Op::Cmp(op), Ty::Bool, &[x, y]);
        self.jump(join, vec![r]);
        self.b.switch(join);
        Ok(self.b.params(join)[0])
    }

    // -- expressions -----------------------------------------------------------------------

    /// The shape of `e`'s value. An expression of no type, such as a `todo` or an `if` whose
    /// branches all leave, gives no value, `V::Never`, and its shape is never used: it is
    /// given as `Unit`.
    fn shape(&self, e: ExprId) -> Result<Shape, String> {
        match &self.s.f.typing.exprs[e.0 as usize] {
            SemaTy::Any => Ok(Shape::Unit),
            SemaTy::Array(n, elem) if self.mixed(elem) => {
                // An array literal of integers of no one type keeps its elements exact until it
                // lands, in the machine type that holds them all (`wide.rs`).
                let tree = self.tree();
                let items: Vec<ExprId> = match &tree.expr(e).kind {
                    ExprKind::Array(items) => items.clone(),
                    ExprKind::Repeat { value, .. } => vec![*value],
                    _ => return Shape::of(&self.s.f.typing.exprs[e.0 as usize]),
                };
                let mut lo: Option<Num> = None;
                let mut hi: Option<Num> = None;
                for &i in &items {
                    let r = self
                        .interval(i)
                        .ok_or("an element without a static range")?;
                    let (l, h) = (Num::new(r.lo.0, r.lo.1), Num::new(r.hi.0, r.hi.1));
                    lo = Some(lo.map_or(l, |x| x.min(l)));
                    hi = Some(hi.map_or(h, |x| x.max(h)));
                }
                let m = match (lo, hi) {
                    (Some(lo), Some(hi)) => machine(lo, hi),
                    _ => Some(IntTy::new(64, true)),
                }
                .ok_or("an array literal beyond 128 bits")?;
                let name = format!("{}{}", if m.signed { "i" } else { "u" }, m.bits);
                let t = SemaIntTy::named(&name).ok_or("a machine type without a name")?;
                let n = u32::try_from(*n).map_err(|_| format!("an array of {} elements", n))?;
                Ok(Shape::Array(n, Box::new(Shape::Int(t))))
            }
            t => Shape::of(t),
        }
    }

    /// Whether `t` is an integer of no one type, the element of an array literal whose elements
    /// have different integer types.
    fn mixed(&self, t: &SemaTy) -> bool {
        matches!(t, SemaTy::Int(i) if int_ty(i).is_none())
    }

    fn interval(&self, e: ExprId) -> Option<Interval> {
        self.s.iv.get(e.0 as usize).copied().flatten()
    }

    /// The range of a place of `shape`: its type's, which holds every value it can take.
    fn place_interval(shape: &Shape) -> Option<Interval> {
        let Shape::Int(t) = shape else {
            return None;
        };
        let t = mir(*t);
        let (lo, hi) = (t.min(), t.max());
        Some(Interval {
            lo: (lo.neg, lo.mag),
            hi: (hi.neg, hi.mag),
        })
    }

    fn expr(&mut self, e: ExprId) -> Result<V, String> {
        let tree = self.tree();
        let span = tree.expr(e).span;
        self.at(span);
        Ok(match &tree.expr(e).kind {
            ExprKind::Int { value, .. } => {
                let n = Num::new(false, *value);
                let t = machine(n, n).ok_or("a literal beyond 128 bits")?;
                V::Int(self.int(t, n), t)
            }
            ExprKind::Bool(b) => V::Bool(self.bool(*b)),
            ExprKind::Name(_) => {
                let id = self.s.f.resolution.names[e.0 as usize].ok_or("a name never resolved")?;
                match self.s.locals[id.0 as usize].clone() {
                    Some(Local::Value(v)) => v,
                    Some(Local::Place(at, shape)) => self.load(at, &shape),
                    None => return Err("a name read before it is bound".into()),
                }
            }
            ExprKind::Todo(reason) => self.trap("todo", &format!("todo reached: {}", reason), span),
            ExprKind::Unreachable(reason) => self.trap(
                "unreachable",
                &format!("unreachable reached: {}", reason),
                span,
            ),
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Not => match self.expr(*operand)? {
                    V::Bool(b) => V::Bool(self.inst(Op::Not, Ty::Bool, &[b])),
                    v => v,
                },
                UnaryOp::Neg => self.negate(e, *operand, span)?,
                UnaryOp::BitNot => {
                    let t = self.int_of(*operand).unwrap_or(I64);
                    let v = self.expr(*operand)?;
                    let V::Int(x, from) = v else {
                        return Ok(v);
                    };
                    let x = self.narrow_int(x, from, t, tree.expr(*operand).span, None);
                    V::Int(self.inst(Op::Not, Ty::Int(mir(t)), &[x]), mir(t))
                }
            },
            ExprKind::Binary { op, lhs, rhs } => match op {
                BinaryOp::And | BinaryOp::Or => self.logic(*op, *lhs, *rhs)?,
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => {
                    let l = self.expr(*lhs)?;
                    let r = self.expr(*rhs)?;
                    let op = match op {
                        BinaryOp::Eq => CmpOp::Eq,
                        BinaryOp::Ne => CmpOp::Ne,
                        BinaryOp::Lt => CmpOp::Lt,
                        BinaryOp::Le => CmpOp::Le,
                        BinaryOp::Gt => CmpOp::Gt,
                        _ => CmpOp::Ge,
                    };
                    match (l, r) {
                        (V::Int(a, ta), V::Int(b, tb)) => {
                            V::Bool(self.compare(op, (a, ta), (b, tb))?)
                        }
                        (V::Bool(a), V::Bool(b)) => {
                            V::Bool(self.inst(Op::Cmp(op), Ty::Bool, &[a, b]))
                        }
                        (V::Never, _) | (_, V::Never) => V::Never,
                        _ => return Err("a comparison of values that are not alike".into()),
                    }
                }
                _ => {
                    let l = self.expr(*lhs)?;
                    let r = self.expr(*rhs)?;
                    let ctx = self.interval(e);
                    let (li, ri) = (self.interval(*lhs), self.interval(*rhs));
                    self.binary(*op, (l, li, *lhs), (r, ri, *rhs), ctx, span)?
                }
            },
            ExprKind::Index { base, index } => {
                let b = self.expr(*base)?;
                let i = self.expr(*index)?;
                let V::Arr(a) = b else {
                    return Ok(V::Never);
                };
                let Shape::Array(len, elem) = self.shape(*base)? else {
                    return Err("an index into a value that is not an array".into());
                };
                let at = self.element(a, len, &elem, i, (span, tree.expr(*index).span))?;
                self.load(at, &elem)
            }
            ExprKind::Field { base, .. } => match self.shape(*base)? {
                // M1's only field is `.len`.
                Shape::Array(len, _) => {
                    self.expr(*base)?;
                    let n = Num::new(false, u128::from(len));
                    V::Int(self.int(USIZE, n), USIZE)
                }
                _ => return Err("a field of a value that is not an array".into()),
            },
            ExprKind::Call { args, .. } => {
                let index =
                    self.s.f.resolution.calls[e.0 as usize].ok_or("a call never resolved")?;
                let callee = self.sig_of(index)?;
                let mut values = Vec::new();
                for (a, (_, shape)) in args.iter().zip(&callee.params) {
                    let aspan = tree.expr(a.value).span;
                    values.push(if a.is_mut {
                        let (root, indexes) = self.place_path(a.value)?;
                        let at = self.address(root, &indexes)?;
                        Arg::Place(
                            at,
                            shape.clone(),
                            self.place_type(root, indexes.len()),
                            aspan,
                        )
                    } else {
                        let v = self.expr(a.value)?;
                        if matches!(v, V::Never) {
                            return Ok(V::Never);
                        }
                        Arg::Value(v, aspan, self.shape(a.value)?)
                    });
                }
                let sret = match &callee.result {
                    Shape::Array(..) => Some(self.inst(
                        Op::Slot {
                            bytes: callee.result.size(),
                        },
                        Ty::Addr,
                        &[],
                    )),
                    _ => None,
                };
                let recursive = index == self.s.index;
                self.call(index, values, span, sret, recursive)?
            }
            ExprKind::Array(items) => {
                let shape = self.shape(e)?;
                let Shape::Array(_, elem) = &shape else {
                    return Err("an array literal that is not an array".into());
                };
                let slot = self.inst(
                    Op::Slot {
                        bytes: shape.size(),
                    },
                    Ty::Addr,
                    &[],
                );
                // Elements of no one type are kept exact, each within the machine type of the
                // array, which holds them all; elements of one type are narrowed into it.
                let mixed = matches!(
                    &self.s.f.typing.exprs[e.0 as usize],
                    SemaTy::Array(_, t) if self.mixed(t)
                );
                for (k, &item) in items.iter().enumerate() {
                    let v = self.expr(item)?;
                    let from = self.shape(item)?;
                    let key = Some(tree.expr(item).span);
                    let v = match (mixed, v, elem.as_ref()) {
                        (true, V::Int(n, t), Shape::Int(to)) => {
                            let into = mir(*to);
                            V::Int(self.cast(n, t, into), into)
                        }
                        (_, v, _) => self.narrow(v, &from, elem, span, key)?,
                    };
                    if matches!(v, V::Never) {
                        return Ok(V::Never);
                    }
                    let idx = self.int(USIZE, Num::new(false, k as u128));
                    let at = self.inst(
                        Op::Elem {
                            stride: elem.size(),
                        },
                        Ty::Addr,
                        &[slot, idx],
                    );
                    self.store_shaped(at, v, elem);
                }
                V::Arr(slot)
            }
            ExprKind::Repeat { value, .. } => {
                let shape = self.shape(e)?;
                let Shape::Array(len, elem) = &shape else {
                    return Err("an array literal that is not an array".into());
                };
                let v = self.expr(*value)?;
                let from = self.shape(*value)?;
                let key = Some(tree.expr(*value).span);
                let v = self.narrow(v, &from, elem, span, key)?;
                if matches!(v, V::Never) {
                    return Ok(V::Never);
                }
                let slot = self.inst(
                    Op::Slot {
                        bytes: shape.size(),
                    },
                    Ty::Addr,
                    &[],
                );
                let (elem, len) = ((**elem).clone(), *len);
                self.each(len, &mut |l, i| {
                    let at = l.inst(
                        Op::Elem {
                            stride: elem.size(),
                        },
                        Ty::Addr,
                        &[slot, i],
                    );
                    l.store_shaped(at, v, &elem);
                    Ok(())
                })?;
                V::Arr(slot)
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => self.if_expr(e, *cond, *then, *otherwise)?,
            ExprKind::Error => return Err("an expression the parser could not read".into()),
        })
    }

    /// The integer type of an expression, as the interpreter's operations take it.
    fn int_of(&self, e: ExprId) -> Option<SemaIntTy> {
        match &self.s.f.typing.exprs[e.0 as usize] {
            SemaTy::Int(i) => int_ty(i),
            _ => None,
        }
    }

    fn logic(&mut self, op: BinaryOp, lhs: ExprId, rhs: ExprId) -> Result<V, String> {
        let V::Bool(l) = self.expr(lhs)? else {
            return Ok(V::Never);
        };
        let right = self.block(&[]);
        let join = self.block(&[Ty::Bool]);
        // `and` stops at false, `or` at true, with that value.
        let (then, otherwise) = if op == BinaryOp::And {
            ((right, vec![]), (join, vec![l]))
        } else {
            ((join, vec![l]), (right, vec![]))
        };
        self.end(Term::Branch {
            cond: l,
            then,
            otherwise,
        });
        self.b.switch(right);
        if let V::Bool(r) = self.expr(rhs)? {
            self.jump(join, vec![r]);
        }
        self.b.switch(join);
        Ok(V::Bool(self.b.params(join)[0]))
    }

    fn if_expr(
        &mut self,
        e: ExprId,
        cond: ExprId,
        then: AstBlock,
        otherwise: Option<Else>,
    ) -> Result<V, String> {
        let V::Bool(c) = self.expr(cond)? else {
            return Ok(V::Never);
        };
        let shape = self.shape(e)?;
        // An integer value meets in one machine type, from the `if`'s static range.
        let join_ty = match (&shape, self.interval(e)) {
            (Shape::Int(_), Some(iv)) => Some(Ty::Int(
                machine(Num::new(iv.lo.0, iv.lo.1), Num::new(iv.hi.0, iv.hi.1))
                    .ok_or("an `if` beyond 128 bits")?,
            )),
            (Shape::Int(t), None) => Some(Ty::Int(mir(*t))),
            (Shape::Bool | Shape::Array(..), _) => Some(shape.ty()),
            _ => None,
        };
        let yes = self.block(&[]);
        let no = self.block(&[]);
        let join = self.block(join_ty.as_slice());
        self.end(Term::Branch {
            cond: c,
            then: (yes, vec![]),
            otherwise: (no, vec![]),
        });
        self.b.switch(yes);
        let v = self.body(then)?;
        self.meet(v, join, join_ty);
        self.b.switch(no);
        let v = match otherwise {
            Some(Else::Block(b)) => self.body(b)?,
            Some(Else::If(i)) => self.expr(i)?,
            None => V::Unit,
        };
        self.meet(v, join, join_ty);
        self.b.switch(join);
        // An `if` of no type has branches that all leave, so nothing reaches its end.
        if matches!(self.s.f.typing.exprs[e.0 as usize], SemaTy::Any) {
            return Ok(V::Never);
        }
        Ok(match (join_ty, self.b.params(join).first()) {
            (Some(Ty::Int(t)), Some(&p)) => V::Int(p, t),
            (Some(Ty::Bool), Some(&p)) => V::Bool(p),
            (Some(Ty::Addr), Some(&p)) => V::Arr(p),
            _ => V::Unit,
        })
    }

    /// Jumps from a branch of an `if` to `join` with its value.
    fn meet(&mut self, v: V, join: BlockId, ty: Option<Ty>) {
        let args = match (v, ty) {
            (V::Never, _) => return,
            (V::Int(n, t), Some(Ty::Int(to))) => vec![self.cast(n, t, to)],
            (V::Bool(n) | V::Arr(n), Some(_)) => vec![n],
            _ => vec![],
        };
        self.jump(join, args);
    }

    fn negate(&mut self, e: ExprId, operand: ExprId, span: Span) -> Result<V, String> {
        // A literal's negation is a constant. `-170141183460469231731687303715884105728` is
        // `i128`'s smallest value, though its literal lies outside `i128` (`core-calculus.md`
        // item 5) and no operation could hold both; and on a literal of any type but `u128`,
        // whose negation the operator checks, it never traps.
        if let ExprKind::Int { value, .. } = self.tree().expr(operand).kind {
            if self
                .int_of(operand)
                .is_none_or(|w| w.signed || w.bits < 128)
            {
                let n = Num::new(value != 0, value);
                let m = machine(n, n).ok_or("a literal beyond 128 bits")?;
                return Ok(V::Int(self.int(m, n), m));
            }
        }
        let v = self.expr(operand)?;
        let V::Int(x, t) = v else {
            return Ok(v);
        };
        if let Some(w) = self.int_of(operand).filter(|w| w.bits == 128) {
            // On an i128 or u128, checked at the operator.
            let w = mir(w);
            let x = self.cast(x, t, w);
            let after = format!(
                " does not fit in `{}`",
                self.int_of(operand).map_or("", |t| t.name)
            );
            self.at(span);
            let site = self.b.trap_site("narrowing", &["", &after]);
            return Ok(V::Int(
                self.inst(Op::Neg { trap: Some(site) }, Ty::Int(w), &[x]),
                w,
            ));
        }
        let m = self.op_type(&[self.interval(operand), self.interval(e)], t)?;
        let x = self.cast(x, t, m);
        Ok(V::Int(
            self.inst(Op::Neg { trap: None }, Ty::Int(m), &[x]),
            m,
        ))
    }

    /// The machine type of an exact operation: the narrowest that holds its operands' and its
    /// result's ranges.
    fn op_type(&self, ranges: &[Option<Interval>], fallback: IntTy) -> Result<IntTy, String> {
        let mut lo: Option<Num> = None;
        let mut hi: Option<Num> = None;
        for r in ranges {
            let Some(r) = r else {
                return Ok(fallback);
            };
            let (l, h) = (Num::new(r.lo.0, r.lo.1), Num::new(r.hi.0, r.hi.1));
            lo = Some(lo.map_or(l, |x| x.min(l)));
            hi = Some(hi.map_or(h, |x| x.max(h)));
        }
        match (lo, hi) {
            (Some(lo), Some(hi)) => {
                machine(lo, hi).ok_or_else(|| "an operation beyond 128 bits".into())
            }
            _ => Ok(fallback),
        }
    }

    /// An arithmetic, bit or shift operation on two lowered operands, each with its range and
    /// expression; `ctx` is the operation's own static type and range.
    fn binary(
        &mut self,
        op: BinaryOp,
        (l, li, le): (V, Option<Interval>, ExprId),
        (r, ri, re): (V, Option<Interval>, ExprId),
        ctx: Option<Interval>,
        span: Span,
    ) -> Result<V, String> {
        let (V::Int(a, ta), V::Int(b, tb)) = (l, r) else {
            return Ok(if matches!(l, V::Never) || matches!(r, V::Never) {
                V::Never
            } else {
                V::Unit
            });
        };
        let tree = self.tree();
        let lt = self.int_of(le);
        let rt = self.int_of(re);
        self.at(span);
        match op {
            BinaryOp::Shl | BinaryOp::Shr => {
                let t = lt.unwrap_or(I64);
                let w = mir(t);
                // The amount lies within the type's width, unless Tier 1 proves it does.
                if !self.proven(
                    ObligationKind::Shift,
                    Key::at(tree.expr(re).span, Tag::None),
                ) {
                    let zero = self.int(tb, Num::ZERO);
                    let ge = self.inst(Op::Cmp(CmpOp::Ge), Ty::Bool, &[b, zero]);
                    let width = Num::new(false, u128::from(t.bits));
                    let below = if tb.holds(width) {
                        let n = self.int(tb, width);
                        self.inst(Op::Cmp(CmpOp::Lt), Ty::Bool, &[b, n])
                    } else {
                        self.bool(true)
                    };
                    let ok = self.inst(Op::Bits(BitOp::And), Ty::Bool, &[ge, below]);
                    let after = format!(" is outside 0 ..= {}", t.bits - 1);
                    self.check(ok, "shift-amount", &["shift amount ", &after], &[b], span);
                }
                let k = self.cast(b, tb, USIZE);
                if op == BinaryOp::Shl {
                    let after = format!(" does not fit in `{}`", t.name);
                    let site = self.b.trap_site("narrowing", &["", &after]);
                    Ok(V::Int(
                        self.inst(Op::Shl { trap: site }, Ty::Int(w), &[a, k]),
                        w,
                    ))
                } else {
                    let s = self.inst(Op::Shr, Ty::Int(ta), &[a, k]);
                    Ok(V::Int(self.narrow_int(s, ta, t, span, Some(span)), w))
                }
            }
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                let t = lt.or(rt).unwrap_or(I64);
                let (ls, rs) = (tree.expr(le).span, tree.expr(re).span);
                let x = self.narrow_int(a, ta, t, ls, Some(ls));
                let y = self.narrow_int(b, tb, t, rs, Some(rs));
                let bit = match op {
                    BinaryOp::BitAnd => BitOp::And,
                    BinaryOp::BitOr => BitOp::Or,
                    _ => BitOp::Xor,
                };
                self.at(span);
                Ok(V::Int(
                    self.inst(Op::Bits(bit), Ty::Int(mir(t)), &[x, y]),
                    mir(t),
                ))
            }
            _ => {
                let arith = match op {
                    BinaryOp::Add => ArithOp::Add,
                    BinaryOp::Sub => ArithOp::Sub,
                    BinaryOp::Mul => ArithOp::Mul,
                    BinaryOp::Div => ArithOp::Div,
                    BinaryOp::Rem => ArithOp::Rem,
                    _ => return Err(format!("an operation {:?} on integers", op)),
                };
                let wide = [lt, rt].into_iter().flatten().find(|t| t.bits == 128);
                let m = match wide {
                    Some(w) => mir(w),
                    None => self.op_type(&[li, ri, ctx], ta)?,
                };
                // An operand whose type the operation's does not hold, an `i32` beside a `u128`,
                // keeps its own: only a checked operation, which computes its exact result
                // first, takes operands of other types.
                let keeps =
                    |t: IntTy| wide.is_some() && !(m.min() <= t.min() && t.max() <= m.max());
                let mixed = keeps(ta) || keeps(tb);
                let x = if keeps(ta) { a } else { self.cast(a, ta, m) };
                let (y, ty) = if keeps(tb) {
                    (b, tb)
                } else {
                    (self.cast(b, tb, m), m)
                };
                let divides = matches!(arith, ArithOp::Div | ArithOp::Rem);
                let divisor = Key::at(tree.expr(re).span, Tag::None);
                if divides && !self.proven(ObligationKind::Divisor, divisor) {
                    let zero = self.int(ty, Num::ZERO);
                    let ok = self.inst(Op::Cmp(CmpOp::Ne), Ty::Bool, &[y, zero]);
                    self.check(ok, "division-by-zero", &["division by zero"], &[], span);
                }
                // An operation on a 128-bit operand is checked unless Tier 1 proves its result
                // fits, and always when its operands have other types.
                let trap = wide
                    .filter(|w| {
                        mixed
                            || !self.proven(ObligationKind::Range, Key::at(span, Tag::Into(w.name)))
                    })
                    .map(|w| {
                        let after = format!(" does not fit in `{}`", w.name);
                        self.b.trap_site("narrowing", &["", &after])
                    });
                Ok(V::Int(
                    self.inst(Op::Arith { op: arith, trap }, Ty::Int(m), &[x, y]),
                    m,
                ))
            }
        }
    }

    // -- calls -----------------------------------------------------------------------------

    /// Calls function `index` with `args`, as the reference interpreter does: each argument
    /// narrowed into its parameter's type at its own span, the parameters' refinements checked,
    /// then the callee's `requires` and its `decreases` measure, at `at`; for a `recursive`
    /// call, the measure must be smaller than this function's. After the call, a `mut` place
    /// is checked against its declared refinement.
    fn call(
        &mut self,
        index: usize,
        args: Vec<Arg>,
        at: Span,
        sret: Option<NodeId>,
        recursive: bool,
    ) -> Result<V, String> {
        let sig = self.sig_of(index)?;
        let mut callee = Scope::new(self.m, index)?;
        // Each argument narrowed into its parameter's type, at its own span.
        let mut values = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let shape = &sig.params[i].1;
            values.push(match arg {
                Arg::Value(v, span, from) => {
                    let v = self.narrow(*v, from, shape, *span, Some(*span))?;
                    if matches!(v, V::Never) {
                        return Ok(V::Never);
                    }
                    callee.locals[i] = Some(match v {
                        V::Arr(a) => Local::Place(a, shape.clone()),
                        v => Local::Value(v),
                    });
                    match v {
                        V::Int(n, _) | V::Bool(n) | V::Arr(n) => n,
                        _ => return Err("an argument without a value".into()),
                    }
                }
                Arg::Place(a, shape, _, _) => {
                    callee.locals[i] = Some(Local::Place(*a, shape.clone()));
                    *a
                }
            });
        }
        // Then each parameter's refinement, in the callee's scope, since a bound may name
        // another parameter.
        let caller = self.measure.clone();
        let saved = std::mem::replace(&mut self.s, callee);
        let was = std::mem::replace(&mut self.in_callee, true);
        let outcome = self.callee_checks(&args, at, recursive, caller);
        self.in_callee = was;
        self.s = saved;
        outcome?;
        let mut mir_args = Vec::new();
        mir_args.extend(sret);
        mir_args.extend(values);
        let (_, result_ty) = mir_params(sig);
        self.at(at);
        let r = self.inst(Op::Call(sig.func), result_ty.unwrap_or(Ty::Unit), &mir_args);
        // A `mut` place takes back what the callee left, which enters its refinement.
        for arg in &args {
            if let Arg::Place(a, shape, Some(d), arg_span) = arg {
                let v = self.load(*a, shape);
                self.refined(v, shape, *d, at, Key::at(*arg_span, Tag::Back))?;
            }
        }
        Ok(match (&sig.result, sret) {
            (Shape::Int(t), _) => V::Int(r, mir(*t)),
            (Shape::Bool, _) => V::Bool(r),
            (Shape::Array(..), Some(slot)) => V::Arr(slot),
            _ => V::Unit,
        })
    }

    /// The checks a call makes in the callee's scope: the parameters' refinements, `requires`
    /// and the `decreases` measure, against `caller`'s for a recursive call.
    fn callee_checks(
        &mut self,
        args: &[Arg],
        at: Span,
        recursive: bool,
        caller: Option<Vec<V>>,
    ) -> Result<(), String> {
        let tree = self.tree();
        for (i, (p, arg)) in self.s.f.syntax.params.clone().iter().zip(args).enumerate() {
            let (Arg::Value(_, span, _) | Arg::Place(_, _, _, span)) = arg;
            let span = *span;
            if let Some(local) = self.s.locals[i].clone() {
                let shape = Shape::of(&SemaTy::of(tree, p.ty))?;
                let v = match local {
                    Local::Value(v) => v,
                    Local::Place(a, s) => self.load(a, &s),
                };
                self.refined(v, &shape, p.ty, span, Key::call(span, Tag::None))?;
            }
        }
        for c in self.clauses(ContractKind::Requires) {
            let text = format!("`requires {}` is false at this call", tree.render_expr(c));
            let key = Key::call(at, Tag::Clause(tree.expr(c).span));
            self.clause(c, ("requires", ObligationKind::Requires), &text, at, key)?;
        }
        let clauses = self.clauses(ContractKind::Decreases);
        if let Some(&first) = clauses.first() {
            let now = self.measure(&clauses, Some(at))?;
            let proven = self.proven(ObligationKind::Decreases, Key::call(at, Tag::Smaller));
            if let (true, Some(before), false) = (recursive, caller, proven) {
                let smaller = self.lex_less(&now, &before)?;
                let text = format!(
                    "`decreases {}` is negative or did not decrease",
                    tree.render_expr(first)
                );
                self.check(smaller, "decreases", &[&text], &[], at);
            }
        }
        Ok(())
    }
}

/// The refinement of the integer type at the bottom of `t`'s arrays, when it has one.
fn leaf_refinement(tree: &Tree, t: TypeId) -> Option<&Refinement> {
    match &tree.ty(t).kind {
        TypeKind::Int { refinement, .. } => refinement.as_ref(),
        TypeKind::Array { elem, .. } => leaf_refinement(tree, *elem),
        _ => None,
    }
}

/// The integer type in which two of `a` and `b` compare, when there is one: a u128 and a
/// signed type have none.
fn common(a: IntTy, b: IntTy) -> Option<IntTy> {
    if a == b {
        return Some(a);
    }
    match (a.signed, b.signed) {
        (false, false) | (true, true) => Some(IntTy::new(a.bits.max(b.bits), a.signed)),
        _ => {
            let (u, s) = if a.signed { (b, a) } else { (a, b) };
            if u.bits == 128 {
                return None;
            }
            Some(IntTy::new(s.bits.max(u.bits * 2), true))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U16: SemaIntTy = SemaIntTy {
        name: "u16",
        signed: false,
        bits: 16,
    };

    #[test]
    fn test_shapes_have_the_size_and_type_of_their_values() {
        assert_eq!(Shape::Int(U16).size(), 2);
        assert_eq!(Shape::Bool.size(), 1);
        let grid = Shape::Array(3, Box::new(Shape::Array(2, Box::new(Shape::Int(U16)))));
        assert_eq!(grid.size(), 12);
        assert_eq!(Shape::Unit.size(), 0);
        assert_eq!(grid.ty(), Ty::Addr);
        assert_eq!(Shape::Int(U16).ty(), Ty::Int(IntTy::new(16, false)));
        assert_eq!(Shape::Bool.ty(), Ty::Bool);
        assert_eq!(
            Shape::of(&SemaTy::Array(2, Box::new(SemaTy::Bool))),
            Ok(Shape::Array(2, Box::new(Shape::Bool)))
        );
        assert!(Shape::of(&SemaTy::Any).is_err());
    }

    /// An array literal of integers of no standard type together keeps its elements exact, in
    /// the machine type that holds them all, and narrows them only where it lands.
    #[test]
    fn test_an_array_literal_of_mixed_integers_keeps_its_elements_exact() {
        let src = "module m
pub fn m(a: u128, b: i8[0..10]) -> [2]u128 = [a, b]
";
        let lowered = lower(&mente_sema::check_program(src).unwrap()).unwrap();
        // Kept in `u128`, which holds both: no narrowing at the literal, none where it lands.
        assert_eq!(checks(&lowered, "m"), Vec::<String>::new());
        let text = lowered.module.to_string();
        assert!(text.contains("slot 32"), "{}", text);
    }

    /// An expression of no type gives no value: a `todo` body, and an `if` whose branches all
    /// return, lower, and nothing reaches their end but the trap that marks it unreachable.
    #[test]
    fn test_an_expression_of_no_type_lowers_to_no_value() {
        let source = "module m
pub fn f() -> i32 = todo(\"x\")
pub fn g(c: bool) -> i32 {
  if c {
    return 1
  } else {
    return 2
  }
}
";
        let module = mente_sema::check_program(source).unwrap();
        let lowered = lower(&module).unwrap();
        let text = lowered.module.to_string();
        assert!(text.contains("todo reached: x"), "{}", text);
        let g = lowered
            .module
            .functions
            .iter()
            .find(|f| f.name == "g")
            .unwrap();
        // Both branches return; the block after the `if` only traps.
        let returns = g
            .blocks
            .iter()
            .filter(|b| matches!(b.term, Term::Return(_)))
            .count();
        assert_eq!(returns, 2, "{}", g);
    }

    #[test]
    fn test_machine_types_are_the_narrowest_that_hold_a_range() {
        let n = Num::from_i128;
        assert_eq!(machine(n(-1), n(5)), Some(IntTy::new(64, true)));
        assert_eq!(
            machine(n(0), Num::new(false, u128::from(u64::MAX))),
            Some(IntTy::new(64, false))
        );
        assert_eq!(
            machine(n(-1), Num::new(false, u128::from(u64::MAX))),
            Some(IntTy::new(128, true))
        );
        assert_eq!(
            machine(n(0), Num::new(false, u128::MAX)),
            Some(IntTy::new(128, false))
        );
        assert_eq!(machine(n(-1), Num::new(false, u128::MAX)), None);
        assert_eq!(
            common(IntTy::new(8, false), IntTy::new(64, true)),
            Some(IntTy::new(64, true))
        );
        assert_eq!(
            common(IntTy::new(64, false), IntTy::new(8, true)),
            Some(IntTy::new(128, true))
        );
        assert_eq!(common(IntTy::new(128, false), IntTy::new(8, true)), None);
        assert_eq!(
            common(IntTy::new(16, true), IntTy::new(32, true)),
            Some(IntTy::new(32, true))
        );
    }

    /// The kinds of the checks in the lowered function `name`, sorted: each `Check`'s trap kind,
    /// and `checked` for an operation that traps by itself.
    fn checks(lowered: &Lowered, name: &str) -> Vec<String> {
        let f = lowered
            .module
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("no function {name}"));
        let mut kinds: Vec<String> = f
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Check { trap } => Some(f.traps[trap.0 as usize].kind.clone()),
                Op::Arith { trap: Some(_), .. } | Op::Neg { trap: Some(_) } | Op::Shl { .. } => {
                    Some("checked".to_string())
                }
                _ => None,
            })
            .collect();
        kinds.sort();
        kinds
    }

    /// Where Tier 1 proves a narrowing, an index, a divisor, a shift amount or a 128-bit
    /// operation, its check is left out of MIR; where it does not, the check stays, and so does
    /// every check of an entry, whose arguments come from outside the program, already narrowed.
    #[test]
    fn test_tier1_proofs_leave_bounds_and_overflow_checks_out_of_mir() {
        let src = "module m
pub fn get(xs: [4]u8, i: usize[0..4]) -> u8 = xs[i]
pub fn any(xs: [4]u8, i: usize) -> u8 = xs[i]
pub fn half(x: u8) -> u8 = x / 2
pub fn div(x: u8, y: u8) -> u8 = x / y
pub fn sh(x: u8, k: u8[0..8]) -> u8 = x >> k
pub fn shk(x: u8, k: u8) -> u8 = x >> k
pub fn sum(a: u8[0..100], b: u8[0..100]) -> u8 = a + b
pub fn add(a: u8, b: u8) -> u8 = a + b
pub fn big(a: i128[0..10], b: i128[0..10]) -> i128 = a * b
pub fn huge(a: i128, b: i128) -> i128 = a * b
pub fn mix(xs: [4]u8, i: usize, k: usize[0..4]) -> u8 {
  val a: u8 = xs[k]
  val b: u8 = xs[i]
  a & b
}
pub fn mask(a: u8[0..100], b: u8) -> u8 = (a + 1) & b
pub fn mask2(a: u8, b: u8) -> u8 = (a + 1) & b
pub fn arr(x: u8[0..10]) -> [2]u8 = [x + 1, 2]
pub fn rep(x: u8[0..10]) -> [3]u8 = [x + 1; 3]
fn take(v: u8) -> u8 = v
pub fn pass(x: u8[0..10]) -> u8 = take(x + 1)
pub fn pass2(x: u8) -> u8 = take(x + 1)
";
        let proven = lower(&mente_sema::check_program(src).unwrap()).unwrap();
        let unproven = lower(&mente_sema::check_program_without_proofs(src).unwrap()).unwrap();
        let both = |name: &str| (checks(&proven, name), checks(&unproven, name));
        let kinds = |k: &[&str]| k.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let index = "index-out-of-bounds";
        for (name, kept, all) in [
            ("get", &[][..], &[index][..]),
            ("any", &[index], &[index]),
            ("half", &[], &["division-by-zero", "narrowing"]),
            (
                "div",
                &["division-by-zero", "narrowing"],
                &["division-by-zero", "narrowing"],
            ),
            ("sh", &[], &["shift-amount"]),
            ("shk", &["shift-amount"], &["shift-amount"]),
            ("sum", &[], &["narrowing"]),
            ("add", &["narrowing"], &["narrowing"]),
            ("big", &[], &["checked"]),
            ("huge", &["checked"], &["checked"]),
            ("mix", &[index], &[index, index]),
            ("mask", &[], &["narrowing"]),
            ("mask2", &["narrowing"], &["narrowing"]),
            ("arr", &[], &["narrowing", "narrowing"]),
            ("rep", &[], &["narrowing"]),
            ("pass", &[], &["narrowing"]),
            ("pass2", &["narrowing"], &["narrowing"]),
        ] {
            assert_eq!(both(name), (kinds(kept), kinds(all)), "{name}");
        }
        // An entry takes its arguments already narrowed into their types, by whoever calls it.
        assert_eq!(checks(&proven, "enter half"), kinds(&[]));
    }

    /// Where Tier 1 proves a refinement, a `requires` or `ensures` clause, a loop invariant or
    /// a `decreases` measure, its check is left out of MIR, clause by clause: a clause outside
    /// the Tier-1 fragment, which it does not prove, keeps its check, and so does an entry.
    #[test]
    fn test_tier1_proofs_leave_contract_checks_out_of_mir() {
        let src = "module m
fn need(x: u8) -> u8
  requires x > 2
= x - 1
pub fn use_need(x: u8[5..10]) -> u8 = need(x)
fn small(k: u8[0..10]) -> u8 = k
pub fn give(x: u8[0..5]) -> u8 = small(x)
pub fn land(x: u8[0..5]) -> u8[0..10] = x
fn bump(mut k: u8[0..10]) {
  guard k < 9 else {
    return
  }
  k += 1
}
pub fn twice(x: u8[0..5]) -> u8 {
  var k: u8[0..10] = x
  bump(mut k)
  k
}
pub fn count(n: u8) -> u8 {
  var i: u8 = 0
  while i < n invariant i <= n and n >= 0 decreases n - i {
    i += 1
  }
  i
}
fn down(n: u8) -> u8
  decreases n
= if n == 0 { 0 } else { down(n - 1) }
pub fn start(n: u8) -> u8 = down(n)
pub fn pos(x: u8) -> u8
  ensures result >= x
= x
@checked
fn both(x: u8, y: u8) -> u8
  requires x * y < 100
  requires x < 50
= x
pub fn call_both(x: u8[0..5], y: u8[0..5]) -> u8 = both(x, y)
";
        let proven = lower(&mente_sema::check_program(src).unwrap()).unwrap();
        let unproven = lower(&mente_sema::check_program_without_proofs(src).unwrap()).unwrap();
        let both = |name: &str| (checks(&proven, name), checks(&unproven, name));
        let kinds = |k: &[&str]| k.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (refinement, decreases) = ("refinement", "decreases");
        for (name, kept, all) in [
            ("use_need", &[][..], &["requires"][..]),
            ("give", &[], &[refinement]),
            ("land", &[], &[refinement]),
            ("twice", &[], &[refinement, refinement, refinement]),
            (
                "count",
                &[],
                &[decreases, decreases, "invariant", "narrowing", "narrowing"],
            ),
            (
                "down",
                &[],
                &[decreases, decreases, "narrowing", "narrowing"],
            ),
            ("start", &[], &[decreases]),
            ("pos", &[], &["ensures"]),
            ("call_both", &["requires"], &["requires", "requires"]),
        ] {
            assert_eq!(both(name), (kinds(kept), kinds(all)), "{name}");
        }
        // An entry checks its arguments' refinements whatever Tier 1 proves; their narrowing
        // into their types is its caller's.
        assert_eq!(checks(&proven, "enter give"), kinds(&[refinement]));
    }
}
