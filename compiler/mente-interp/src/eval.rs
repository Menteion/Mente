//! The interpreter proper (`spec/core-calculus.md`, items 4, 6, 7, 9 and 10).
//!
//! A [`Frame`] holds one call's bindings in slots indexed like the function's resolution, which
//! names each binding once (no shadowing), so a block needs no scope of its own. Control leaves
//! a statement by an [`Exit`]: a `return`, a `break`, a `continue`, or a stop.

use mente_sema::frames::BUDGET;
use mente_sema::names::BindingId;
use mente_sema::structs::Structs;
use mente_sema::types::{Int as IntType, IntTy, Ty, I64};
use mente_sema::{CheckedFn, CheckedModule};
use mente_syntax::tree::{
    Arg, Arm, ArmBody, AssignOp, BinaryOp, BlockId, Body, Bound, ContractKind, Else, ExprId,
    ExprKind, FieldInit, PatternKind, Payload, Refinement, StmtId, StmtKind, Tree, TypeId,
    TypeKind, UnaryOp,
};
use mente_syntax::Span;
use std::sync::Arc;

use crate::host::{self, Host};
use crate::kinds::{kinds, Kind};
use crate::{Event, Int, Limits, Observer, Shown, Stop, Tracer, Trap, TrapKind, Value};

/// A point of a function's walk that a traced run shows ([`Event`]).
#[derive(Clone, Copy, Debug)]
enum Point {
    Before(StmtId),
    End(BlockId),
    Head(StmtId),
}

/// How control leaves a statement other than by its end.
enum Exit {
    Return(Value, Span),
    Break,
    Continue,
    Stop(Stop),
}

impl From<Stop> for Box<Exit> {
    fn from(s: Stop) -> Box<Exit> {
        Box::new(Exit::Stop(s))
    }
}

/// The outcome of a step: its value, or how control left it. The way out is boxed, so that an
/// outcome takes no more of the host's stack than its value does, in every frame that holds one
/// on the way (`crate::host`).
type Flow<T> = Result<T, Box<Exit>>;

/// A way out that stops the run.
fn stopped(s: Stop) -> Box<Exit> {
    Box::new(Exit::Stop(s))
}

/// The outcome of a call's run, as its caller sees it: what stopped the run stops the caller.
fn ended(outcome: Flow<(Value, Vec<Value>)>) -> Result<(Value, Vec<Value>), Stop> {
    match outcome {
        Ok(v) => Ok(v),
        Err(e) => match *e {
            Exit::Stop(s) => Err(s),
            // A checked body never lets `break` or `continue` leave it.
            Exit::Return(..) | Exit::Break | Exit::Continue => Ok((Value::Unit, Vec::new())),
        },
    }
}

/// Where the block body `b` of `f` ends: its last statement, or the block where it is empty.
fn end_of(f: &CheckedFn, b: BlockId) -> Span {
    let tree = &f.syntax.tree;
    tree.block(b)
        .stmts
        .last()
        .map_or(tree.block(b).span, |&s| tree.stmt(s).span)
}

/// The way out of `e`, where it is a `return`: the value returned and where.
fn returned(e: Exit) -> Result<(Value, Span), Box<Exit>> {
    match e {
        Exit::Return(v, span) => Ok((v, span)),
        other @ (Exit::Break | Exit::Continue | Exit::Stop(_)) => Err(Box::new(other)),
    }
}

fn trap<T>(kind: TrapKind, span: Span) -> Flow<T> {
    Err(stopped(Stop::Trap(Trap { kind, span })))
}

/// A state the checks rule out, reported rather than run on with a made-up value.
fn broken<T>(what: &str) -> Flow<T> {
    Err(stopped(Stop::Broken(what.to_string())))
}

/// The integer type a static type names: its one type, or the type that holds a mix.
fn int_ty(ty: &Ty) -> Option<IntTy> {
    match ty {
        Ty::Int(IntType::One(t)) | Ty::Int(IntType::Mixed(Some(t))) => Some(*t),
        Ty::Int(IntType::Literal | IntType::Mixed(None))
        | Ty::Bool
        | Ty::Array(..)
        | Ty::Struct(_)
        | Ty::Enum(_)
        | Ty::Unit
        | Ty::Any => None,
    }
}

/// The type literals work in under `~`, a bit operation or a shift, in context `ctx`: its
/// type, or `i64` for literals alone. Integers that no standard type holds give none, which
/// `E0212` rejects; none is chosen for them here (`core-calculus.md`, item 5).
fn worked_in(ctx: Kind, what: &str) -> Flow<IntTy> {
    match ctx {
        Kind::Typed(Some(t)) => Ok(t),
        Kind::Literal | Kind::Other => Ok(I64),
        Kind::Typed(None) => broken(what),
    }
}

/// Whether `e` is an operator of one expression with its operands, whose literals take the type
/// of every operand of that expression that has one (`core-calculus.md`, item 5): arithmetic,
/// unary `-`, and `~`, a bit operation or a shift of literals alone; and a literal. A bit
/// operation with an operand of a type, or a shift whose shifted operand has one, works in that
/// type, so the expression of its literals ends there, as it does at a comparison, a call's
/// argument, a narrowing point, a branch's value and an index. Derived here from the tree and the interpreter's own kinds, as
/// the checker derives it from its types, so that the comparison of the paths checks the
/// checker's table.
fn joins(tree: &Tree, kinds: &[Kind], e: ExprId) -> bool {
    let untyped = kinds[e.0 as usize] == Kind::Literal;
    match &tree.expr(e).kind {
        ExprKind::Int { .. } => true,
        ExprKind::Unary { op, .. } => match op {
            UnaryOp::Neg => true,
            UnaryOp::BitNot => untyped,
            UnaryOp::Not => false,
        },
        ExprKind::Binary { op, .. } => match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => true,
            BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::Shl
            | BinaryOp::Shr => untyped,
            BinaryOp::And
            | BinaryOp::Or
            | BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => false,
        },
        ExprKind::Bool(_)
        | ExprKind::Name(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Index { .. }
        | ExprKind::Field { .. }
        | ExprKind::Call { .. }
        | ExprKind::Construct { .. }
        | ExprKind::Variant { .. }
        | ExprKind::Match { .. }
        | ExprKind::Array(_)
        | ExprKind::Repeat { .. }
        | ExprKind::If { .. }
        | ExprKind::Error => false,
    }
}

/// Whether `v` has the shape of a value of type `ty`: an integer, a `bool`, an array of its
/// length whose elements have its element's shape, or a struct of its name whose fields are
/// named as its declaration names them, in order, each of its field's shape, or a variant of an
/// enum of its name whose payload is so. Its range is not checked here.
fn shaped(v: &Value, ty: &Ty, structs: &Structs) -> bool {
    match (v, ty) {
        (Value::Int(_), Ty::Int(_)) | (Value::Bool(_), Ty::Bool) => true,
        (Value::Array(items), Ty::Array(n, elem)) => {
            items.len() as u128 == *n && items.iter().all(|i| shaped(i, elem, structs))
        }
        (Value::Struct(name, fields), Ty::Struct(s)) if name == s => {
            structs.get(s).is_some_and(|def| {
                def.fields.len() == fields.len()
                    && def
                        .fields
                        .iter()
                        .zip(fields)
                        .all(|(d, (n, v))| d.name == *n && shaped(v, &d.ty, structs))
            })
        }
        (Value::Enum(name, variant, fields), Ty::Enum(e)) if name == e => structs
            .get_enum(e)
            .and_then(|def| def.variant(variant))
            .is_some_and(|(_, v)| {
                v.fields.len() == fields.len()
                    && v.fields
                        .iter()
                        .zip(fields)
                        .all(|(d, (n, x))| d.name == *n && shaped(x, &d.ty, structs))
            }),
        _ => false,
    }
}

/// A declared type: a type node in the tree it lives in, a function's or a struct's.
type Declared<'t> = Option<(&'t Tree, TypeId)>;

/// One call: its function and the values of its bindings.
struct Frame<'m> {
    index: usize,
    f: &'m CheckedFn,
    slots: Vec<Option<Value>>,
    /// The function's `decreases` measure at this call, for the recursive calls it makes.
    measure: Option<Vec<Int>>,
    /// The integer type of each of the function's expressions, as the interpreter derives it.
    kinds: Arc<[Kind]>,
}

/// A step from a place's root binding: an index with the span of its indexing, or a field by its
/// index in the struct's declaration.
#[derive(Clone, Debug)]
enum Step {
    Index(Int, Span),
    Field(usize),
}

/// A place a `mut` argument or an assignment names: its root binding and the steps into it.
type Place = (BindingId, Vec<Step>);

/// The arguments of a call once evaluated: their values, where each was written, and the place
/// each `mut` argument names.
type Given = (Vec<Value>, Vec<Span>, Vec<Option<Place>>);

pub(crate) struct Machine<'m> {
    module: &'m CheckedModule,
    limits: Limits,
    steps: u64,
    /// The bytes the frames of the calls that have begun and not returned sum to, against the
    /// stack budget (`core-calculus.md`, item 25).
    stack: u128,
    /// Where the interpreter stands on the host's stack (`crate::host`).
    host: Result<Host, &'static str>,
    /// What is shown each integer an expression gives, when a run is observed.
    seen: Option<Observer<'m>>,
    /// What is shown each point of the run Tier 1 also analyses, when a run is traced.
    trace: Option<Tracer<'m>>,
    /// The kinds of each function's expressions, derived at its first call.
    kinds: Vec<Option<Arc<[Kind]>>>,
}

impl<'m> Machine<'m> {
    pub(crate) fn new(
        module: &'m CheckedModule,
        limits: Limits,
        seen: Option<Observer<'m>>,
    ) -> Machine<'m> {
        Machine {
            module,
            limits,
            steps: 0,
            stack: 0,
            host: Host::start(0),
            seen,
            trace: None,
            kinds: vec![None; module.functions.len()],
        }
    }

    /// The machine, showing `trace` each [`Event`] of its run.
    pub(crate) fn traced(mut self, trace: Tracer<'m>) -> Machine<'m> {
        self.trace = Some(trace);
        self
    }

    /// Shows the tracer, when the run is traced, the point `at` of `frame`'s function, with
    /// its bindings' values. Apart from the walk, so that a step's host frame holds none of it.
    #[inline(never)]
    fn point(&mut self, frame: &Frame<'m>, at: Point) {
        let Some(trace) = self.trace.as_mut() else {
            return;
        };
        let (function, slots) = (frame.index, frame.slots.as_slice());
        trace(match at {
            Point::Before(stmt) => Event::Before {
                function,
                stmt,
                slots,
            },
            Point::End(block) => Event::End {
                function,
                block,
                slots,
            },
            Point::Head(stmt) => Event::Head {
                function,
                stmt,
                slots,
            },
        });
    }

    /// Calls the function `name` from outside the program.
    pub(crate) fn enter(
        &mut self,
        name: &str,
        args: Vec<Value>,
    ) -> Result<(Value, Vec<Value>), Stop> {
        if let Err(why) = self.host {
            return Err(Stop::BadArguments(why.to_string()));
        }
        let index = self
            .module
            .index(name)
            .filter(|&i| matches!(self.module.functions.get(i), Some(Some(_))))
            .ok_or_else(|| Stop::NoFunction(name.to_string()))?;
        let f = self.function(index)?;
        if args.len() != f.syntax.params.len() {
            return Err(Stop::BadArguments(format!(
                "`{}` takes {} arguments, but {} were given",
                name,
                f.syntax.params.len(),
                args.len()
            )));
        }
        // An argument must have its parameter's shape: an integer, a `bool`, or an array of the
        // parameter's length whose elements have its element's shape. One that does not is no
        // call of this function, so it is refused before anything runs, on every path alike.
        let tree = &f.syntax.tree;
        for (p, v) in f.syntax.params.iter().zip(&args) {
            if !shaped(v, &Ty::of(tree, p.ty), &self.module.structs) {
                return Err(Stop::BadArguments(format!(
                    "`{}` does not have its parameter's shape",
                    v
                )));
            }
        }
        let spans = vec![f.syntax.name_span; args.len()];
        self.call(index, args, &spans, f.syntax.name_span, None)
    }

    fn function(&self, index: usize) -> Result<&'m CheckedFn, Stop> {
        match self.module.functions.get(index) {
            Some(Some(f)) => Ok(f),
            _ => Err(Stop::NoFunction(
                self.module.names.get(index).cloned().unwrap_or_default(),
            )),
        }
    }

    /// Whether function `index` has a body to run: a stop where it has none.
    #[inline(never)]
    fn has_body(&self, index: usize) -> Flow<()> {
        self.function(index)?;
        Ok(())
    }

    /// The kinds of the expressions of function `index`, `f`, derived once.
    fn kinds(&mut self, index: usize, f: &CheckedFn) -> Arc<[Kind]> {
        match &self.kinds[index] {
            Some(k) => Arc::clone(k),
            None => {
                let k: Arc<[Kind]> = kinds(f).into();
                self.kinds[index] = Some(Arc::clone(&k));
                k
            }
        }
    }

    fn step(&mut self) -> Flow<()> {
        self.steps += 1;
        if self.steps > self.limits.max_steps {
            return Err(stopped(Stop::OutOfSteps));
        }
        Ok(())
    }

    /// Runs function `index` on `args`, each narrowed into its parameter's type at the span in
    /// `spans`. Returns the result and the final value of every parameter, for the caller to
    /// copy back into the places it passed as `mut`. The call is counted against the stack
    /// budget first, its arguments evaluated and none narrowed yet, and traps at `at` where the
    /// stack in use and its frame together exceed it; its frame is in use until it returns
    /// (`core-calculus.md`, item 25).
    fn call(
        &mut self,
        index: usize,
        args: Vec<Value>,
        spans: &[Span],
        at: Span,
        caller: Option<(usize, Option<Vec<Int>>)>,
    ) -> Result<(Value, Vec<Value>), Stop> {
        let (mut frame, bytes) = self.counted(index, at)?;
        let at_host = host::here();
        let outer = self.host.as_mut().map_or(0, |h| h.called(at_host));
        self.stack += bytes;
        let outcome = self.run(&mut frame, args, spans, at, caller);
        self.stack -= bytes;
        if let Ok(h) = &mut self.host {
            h.returned(outer);
        }
        ended(outcome)
    }

    /// The frame of a call of function `index` at `at`, and its bytes, which the call counts
    /// against the budget: a trap where the stack in use and those bytes together exceed it.
    #[inline(never)]
    fn counted(&mut self, index: usize, at: Span) -> Result<(Frame<'m>, u128), Stop> {
        let f = self.function(index)?;
        let bytes = self
            .module
            .frames
            .get(index)
            .copied()
            .flatten()
            .map_or(0, |frame| frame.bytes);
        if self.stack + bytes > BUDGET {
            return Err(Stop::Trap(Trap {
                kind: TrapKind::StackOverflow,
                span: at,
            }));
        }
        let kinds = self.kinds(index, f);
        let frame = Frame {
            index,
            f,
            slots: vec![None; f.resolution.bindings.len()],
            measure: None,
            kinds,
        };
        Ok((frame, bytes))
    }

    fn run(
        &mut self,
        frame: &mut Frame<'m>,
        args: Vec<Value>,
        spans: &[Span],
        at: Span,
        caller: Option<(usize, Option<Vec<Int>>)>,
    ) -> Flow<(Value, Vec<Value>)> {
        self.touch();
        self.entered(frame, args, spans, at, caller)?;
        let (result, ret_span) = self.body(frame, at)?;
        self.left(frame, result, ret_span)
    }

    /// The parameters bound and narrowed, their refinements, the `requires` clauses and the
    /// `decreases` measure checked, on entry to a call at `at`.
    #[inline(never)]
    fn entered(
        &mut self,
        frame: &mut Frame<'m>,
        args: Vec<Value>,
        spans: &[Span],
        at: Span,
        caller: Option<(usize, Option<Vec<Int>>)>,
    ) -> Flow<()> {
        let f = frame.f;
        let tree = &f.syntax.tree;
        // The parameters are the function's first bindings, in order.
        for (i, (p, v)) in f.syntax.params.iter().zip(args).enumerate() {
            let v = self.narrow_plain(v, &Ty::of(tree, p.ty), spans[i])?;
            frame.slots[i] = Some(v);
        }
        // A refinement may name another parameter, so each is checked once all are bound.
        for (i, p) in f.syntax.params.iter().enumerate() {
            let v = frame.slots[i].clone().unwrap_or(Value::Unit);
            self.refined(frame, &v, Some((tree, p.ty)), spans[i])?;
        }
        for c in &f.syntax.contracts {
            match c.kind {
                ContractKind::Requires => {
                    if !self.cond(frame, c.expr)? {
                        return trap(TrapKind::Requires(tree.render_expr(c.expr)), at);
                    }
                }
                ContractKind::Decreases | ContractKind::Ensures => {}
            }
        }
        // The `decreases` clauses together form one measure, each part never negative, and at a
        // direct recursive call smaller than the caller's in lexicographic order.
        let clauses: Vec<ExprId> = f
            .syntax
            .contracts
            .iter()
            .filter(|c| c.kind == ContractKind::Decreases)
            .map(|c| c.expr)
            .collect();
        if let Some(&first) = clauses.first() {
            let measure = self.measure(frame, &clauses)?;
            let recursive = caller.as_ref().filter(|(i, _)| *i == frame.index);
            if let Some((_, Some(before))) = recursive {
                if measure >= *before {
                    return trap(TrapKind::Decreases(tree.render_expr(first)), at);
                }
            }
            frame.measure = Some(measure);
        }
        Ok(())
    }

    /// The value the body gives, and where: a `return`'s, narrowed where it stands, or the body's
    /// last expression's, narrowed into the function's result type there. An expression body
    /// leaves by `return` too, from a branch of an `if` or an arm of a `match` in it, which ends
    /// this call alone.
    fn body(&mut self, frame: &mut Frame<'m>, at: Span) -> Flow<(Value, Span)> {
        let f = frame.f;
        let (ended, span) = match f.syntax.body {
            Some(Body::Expr(e)) => (
                self.eval(frame, e, Kind::Literal),
                f.syntax.tree.expr(e).span,
            ),
            Some(Body::Block(b)) => (self.block(frame, b, Kind::Literal), end_of(f, b)),
            None => return Ok((Value::Unit, at)),
        };
        match ended {
            Ok(v) => Ok((self.result(frame, v, span)?, span)),
            Err(e) => returned(*e),
        }
    }

    /// `v`, the value a body ends with at `span`, narrowed into the function's result type.
    #[inline(never)]
    fn result(&mut self, frame: &Frame<'m>, v: Value, span: Span) -> Flow<Value> {
        let f = frame.f;
        let tree = &f.syntax.tree;
        let result_ty = f.syntax.result.map_or(Ty::Unit, |t| Ty::of(tree, t));
        let declared = f.syntax.result.map(|t| (tree, t));
        self.narrow(frame, v, declared, &result_ty, span)
    }

    /// The `ensures` clauses checked of `result`, given at `ret_span`, and the call's outcome:
    /// its result and the final value of every parameter.
    #[inline(never)]
    fn left(
        &mut self,
        frame: &mut Frame<'m>,
        result: Value,
        ret_span: Span,
    ) -> Flow<(Value, Vec<Value>)> {
        let f = frame.f;
        let tree = &f.syntax.tree;
        for c in &f.syntax.contracts {
            if c.kind != ContractKind::Ensures {
                continue;
            }
            // Each `ensures` clause has its own `result` binding, declared at the clause.
            let clause = tree.expr(c.expr).span;
            if let Some(id) =
                f.resolution.bindings.iter().position(|b| {
                    b.kind == mente_sema::names::BindingKind::Result && b.span == clause
                })
            {
                frame.slots[id] = Some(result.clone());
            }
            if !self.cond(frame, c.expr)? {
                return trap(TrapKind::Ensures(tree.render_expr(c.expr)), ret_span);
            }
        }
        let finals = (0..f.syntax.params.len())
            .map(|i| frame.slots[i].clone().unwrap_or(Value::Unit))
            .collect();
        Ok((result, finals))
    }

    /// Runs `step`, one step of the evaluation, here; or, where less than the margin is left of
    /// this segment's host stack, on a new segment, a thread the step borrows the machine from and
    /// this one waits for (`crate::host`). Every expression and every block is a step, so none
    /// begins with less than the margin left, whatever the program's shape.
    fn checked<T: Send>(&mut self, step: impl FnOnce(&mut Self) -> Flow<T> + Send) -> Flow<T> {
        match self.begin(host::here()) {
            Some(outer) => {
                let outcome = step(self);
                if let Ok(h) = &mut self.host {
                    h.leave(outer);
                }
                outcome
            }
            None => self.segment(step),
        }
    }

    /// Begins a step whose check stands at `at`: the caller's check, which the step restores when
    /// it ends, where the step runs on this segment; `None` where it continues on a new one, less
    /// than the margin being left, or the host's stack not being one the segments can read.
    #[inline(never)]
    fn begin(&mut self, at: usize) -> Option<usize> {
        match &mut self.host {
            Ok(h) if !h.low(at) => Some(h.enter(at)),
            Ok(_) | Err(_) => None,
        }
    }

    /// Runs `step` on a new segment of the host stack, started through `mente_threads::alone`
    /// with the stack of the one sizing rule, and waits for it.
    #[cold]
    #[inline(never)]
    fn segment<T: Send>(&mut self, step: impl FnOnce(&mut Self) -> Flow<T> + Send) -> Flow<T> {
        let carried = match &self.host {
            Ok(h) => h.carried(host::here()),
            Err(why) => return Err(stopped(Stop::BadArguments(why.to_string()))),
        };
        let started = mente_threads::alone(None, Some(host::SEGMENT), || {
            let outer = std::mem::replace(&mut self.host, Host::start(carried));
            let outcome = match self.host {
                Ok(_) => step(self),
                Err(why) => Err(stopped(Stop::BadArguments(why.to_string()))),
            };
            let inner = std::mem::replace(&mut self.host, outer);
            if let (Ok(h), Ok(i)) = (&mut self.host, &inner) {
                h.absorb(i);
            }
            outcome
        });
        match started {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => Err(stopped(Stop::BadArguments(
                "the interpreter stopped unexpectedly".to_string(),
            ))),
            Err(e) => Err(stopped(Stop::BadArguments(format!(
                "the interpreter could not start a segment of its stack: {}",
                e
            )))),
        }
    }

    /// Records, for the measure of a step, how far below its last check the host stack reaches
    /// here: each function a step runs without checking again calls it first, so the step the
    /// segments' margin is made of includes what runs below the step's last check (`crate::host`).
    #[inline(always)]
    fn touch(&mut self) {
        let at = host::here();
        if let Ok(h) = &mut self.host {
            h.touch(at);
        }
    }

    /// The statements and loop rounds this machine has run, which [`Limits::max_steps`] bounds.
    pub(crate) fn steps(&self) -> u64 {
        self.steps
    }

    /// The largest step of the host stack between two checks this machine has taken.
    #[cfg(test)]
    pub(crate) fn deepest_step(&self) -> usize {
        self.host.as_ref().map_or(0, Host::deepest)
    }

    /// The most host stack a call of this machine has taken, from where it began to where a call
    /// it makes begins.
    pub(crate) fn per_call(&self) -> usize {
        self.host.as_ref().map_or(0, Host::per_call)
    }

    /// The value of a block: its last statement's, when that is an expression, whose literals
    /// take `ctx` where nothing nearer gives them a type.
    fn block(&mut self, frame: &mut Frame<'m>, b: BlockId, ctx: Kind) -> Flow<Value> {
        self.checked(|m| m.block_here(frame, b, ctx))
    }

    /// [`Machine::block`], on this segment.
    fn block_here(&mut self, frame: &mut Frame<'m>, b: BlockId, ctx: Kind) -> Flow<Value> {
        match frame.f.syntax.tree.block(b).stmts.split_last() {
            None => {
                self.point(frame, Point::End(b));
                Ok(Value::Unit)
            }
            Some((&last, before)) => {
                for &s in before {
                    self.point(frame, Point::Before(s));
                    self.stmt(frame, s, Kind::Literal)?;
                }
                self.point(frame, Point::Before(last));
                let v = self.stmt(frame, last, ctx)?;
                self.point(frame, Point::End(b));
                Ok(v)
            }
        }
    }

    /// A statement, its value its expression's where it is one, and nothing otherwise. Each
    /// kind runs in a method of its own and gives its outcome back as it is, so that a step's
    /// host frame holds no other kind's locals (`Machine::value` says why).
    fn stmt(&mut self, frame: &mut Frame<'m>, s: StmtId, ctx: Kind) -> Flow<Value> {
        self.step()?;
        match frame.f.syntax.tree.stmt(s).kind {
            StmtKind::Expr(e) => self.eval(frame, e, ctx),
            StmtKind::Discard(_)
            | StmtKind::Binding { .. }
            | StmtKind::Assign { .. }
            | StmtKind::While { .. }
            | StmtKind::Guard { .. }
            | StmtKind::Return(_)
            | StmtKind::Break
            | StmtKind::Continue => self.other_stmt(frame, s),
        }
    }

    /// A statement that is not an expression.
    #[inline(never)]
    fn other_stmt(&mut self, frame: &mut Frame<'m>, s: StmtId) -> Flow<Value> {
        let tree = &frame.f.syntax.tree;
        let unit = |done: Flow<()>| done.map(|()| Value::Unit);
        match &tree.stmt(s).kind {
            StmtKind::Expr(e) => self.eval(frame, *e, Kind::Literal),
            StmtKind::Discard(e) => unit(self.eval(frame, *e, Kind::Literal).map(drop)),
            StmtKind::Binding { ty, init, .. } => unit(self.binding(frame, s, *ty, *init)),
            StmtKind::Assign { place, op, value } => {
                unit(self.assign(frame, *place, *op, *value, tree.stmt(s).span))
            }
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => unit(self.while_loop(frame, (s, *cond), invariants, decreases, *body)),
            StmtKind::Guard { cond, otherwise } => unit(self.guard(frame, *cond, *otherwise)),
            StmtKind::Return(value) => self.leave(frame, s, *value),
            StmtKind::Break => Err(Box::new(Exit::Break)),
            StmtKind::Continue => Err(Box::new(Exit::Continue)),
        }
    }

    /// `guard cond else { otherwise }`.
    #[inline(never)]
    fn guard(&mut self, frame: &mut Frame<'m>, cond: ExprId, otherwise: BlockId) -> Flow<()> {
        if !self.cond(frame, cond)? {
            self.block(frame, otherwise, Kind::Literal)?;
        }
        Ok(())
    }

    /// A binding statement: its initializer narrowed into the binding's type.
    #[inline(never)]
    fn binding(
        &mut self,
        frame: &mut Frame<'m>,
        s: StmtId,
        ty: Option<TypeId>,
        init: ExprId,
    ) -> Flow<()> {
        let f = frame.f;
        let tree = &f.syntax.tree;
        let Some(id) = f.resolution.stmts[s.0 as usize] else {
            return broken("a binding statement that declares no binding");
        };
        let plain = f.typing.bindings[id.0 as usize].clone();
        let v = self.eval(frame, init, Kind::Literal)?;
        let declared = ty.map(|t| (tree, t));
        let v = self.narrow(frame, v, declared, &plain, tree.expr(init).span)?;
        frame.slots[id.0 as usize] = Some(v);
        Ok(())
    }

    /// `return`, with its value narrowed into the function's result type.
    #[inline(never)]
    fn leave(&mut self, frame: &mut Frame<'m>, s: StmtId, value: Option<ExprId>) -> Flow<Value> {
        let f = frame.f;
        let tree = &f.syntax.tree;
        let Some(e) = value else {
            return Err(Box::new(Exit::Return(Value::Unit, tree.stmt(s).span)));
        };
        let result_ty = f.syntax.result.map_or(Ty::Unit, |t| Ty::of(tree, t));
        let span = tree.expr(e).span;
        let v = self.eval(frame, e, Kind::Literal)?;
        let declared = f.syntax.result.map(|t| (tree, t));
        let v = self.narrow(frame, v, declared, &result_ty, span)?;
        Err(Box::new(Exit::Return(v, span)))
    }

    #[inline(never)]
    fn while_loop(
        &mut self,
        frame: &mut Frame<'m>,
        (s, cond): (StmtId, ExprId),
        invariants: &[ExprId],
        decreases: &[ExprId],
        body: BlockId,
    ) -> Flow<()> {
        let tree = &frame.f.syntax.tree;
        let mut before: Option<Vec<Int>> = None;
        loop {
            self.step()?;
            for &i in invariants {
                if !self.cond(frame, i)? {
                    return trap(TrapKind::Invariant(tree.render_expr(i)), tree.expr(i).span);
                }
            }
            self.point(frame, Point::Head(s));
            if !self.cond(frame, cond)? {
                return Ok(());
            }
            if let Some(&first) = decreases.first() {
                let measure = self.measure(frame, decreases)?;
                if before.as_ref().is_some_and(|b| measure >= *b) {
                    return trap(
                        TrapKind::Decreases(tree.render_expr(first)),
                        tree.expr(first).span,
                    );
                }
                before = Some(measure);
            }
            if let Err(e) = self.block(frame, body, Kind::Literal) {
                match *e {
                    Exit::Continue => {}
                    Exit::Break => return Ok(()),
                    other @ (Exit::Return(..) | Exit::Stop(_)) => return Err(Box::new(other)),
                }
            }
        }
    }

    /// The values of a measure, each of which must not be negative.
    fn measure(&mut self, frame: &mut Frame<'m>, exprs: &[ExprId]) -> Flow<Vec<Int>> {
        let tree = &frame.f.syntax.tree;
        let mut values = Vec::new();
        for &e in exprs {
            let v = self.int(frame, e, Kind::Literal)?;
            if v.is_negative() {
                return trap(TrapKind::Decreases(tree.render_expr(e)), tree.expr(e).span);
            }
            values.push(v);
        }
        Ok(values)
    }

    #[inline(never)]
    fn assign(
        &mut self,
        frame: &mut Frame<'m>,
        place: ExprId,
        op: AssignOp,
        value: ExprId,
        whole: Span,
    ) -> Flow<()> {
        let path = self.place(frame, place)?;
        let declared = self.place_type(frame, &path)?;
        let new = match op {
            AssignOp::Set => self.eval(frame, value, Kind::Literal)?,
            AssignOp::Compound(op) => self.compound(frame, op, (place, &path), value, whole)?,
        };
        self.landed(frame, (place, &path), declared, new, value)
    }

    /// The value `x op e` that a compound assignment `x op= e` lands in the place `x`, at
    /// `path`: `x`'s value, then `e`'s.
    #[inline(never)]
    fn compound(
        &mut self,
        frame: &mut Frame<'m>,
        op: BinaryOp,
        (place, path): (ExprId, &Place),
        value: ExprId,
        whole: Span,
    ) -> Flow<Value> {
        let f = frame.f;
        let tree = &f.syntax.tree;
        let plain = &f.typing.exprs[place.0 as usize];
        let new = {
            {
                let current = self.read(frame, path)?;
                // `x op= e` is `x = x op e` (item 6): under arithmetic `e`'s operands are
                // operands of that expression with `x`, whose literals take the type of them
                // all; a bit operation works in `x`'s type, which its literal operand takes;
                // and a shift's amount takes none.
                let rt = frame.kinds[value.0 as usize];
                let (other, member) = match op {
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Rem => (Kind::of(plain).join(rt), true),
                    BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                        (Kind::of(plain), false)
                    }
                    BinaryOp::Shl
                    | BinaryOp::Shr
                    | BinaryOp::And
                    | BinaryOp::Or
                    | BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge => (Kind::Literal, false),
                };
                let rhs = self.eval_as(frame, value, other, member)?;
                let operands = [(current, int_ty(plain), place), (rhs, rt.ty(), value)];
                let new = self.arith(frame, op, operands, other, tree.expr(value).span)?;
                // No expression of the program gives `x op e`: an observer is shown it with
                // the span of the whole assignment, before it is narrowed into the place.
                if let Some(seen) = self.seen.as_mut() {
                    show(&new, whole, Shown::Given, &mut **seen);
                }
                new
            }
        };
        Ok(new)
    }

    /// `new`, the value of `value`, narrowed into the place `place` at `path` and its declared
    /// type `declared`, and written there.
    #[inline(never)]
    fn landed(
        &mut self,
        frame: &mut Frame<'m>,
        (place, path): (ExprId, &Place),
        declared: Declared<'m>,
        new: Value,
        value: ExprId,
    ) -> Flow<()> {
        let f = frame.f;
        let plain = &f.typing.exprs[place.0 as usize];
        let new = self.narrow(frame, new, declared, plain, f.syntax.tree.expr(value).span)?;
        self.write(frame, path, new)
    }

    /// The root binding of a place and its steps, its indexes evaluated left to right.
    fn place(&mut self, frame: &mut Frame<'m>, e: ExprId) -> Flow<Place> {
        self.touch();
        let f = frame.f;
        match &f.syntax.tree.expr(e).kind {
            ExprKind::Index { base, index } => {
                let (root, mut steps) = self.place(frame, *base)?;
                let i = self.int(frame, *index, Kind::Literal)?;
                steps.push(Step::Index(i, f.syntax.tree.expr(e).span));
                Ok((root, steps))
            }
            ExprKind::Field { base, name } => {
                let (root, mut steps) = self.place(frame, *base)?;
                let k = match &f.typing.exprs[base.0 as usize] {
                    Ty::Struct(s) => match self.module.structs.get(s).and_then(|d| d.field(name)) {
                        Some((k, _)) => k,
                        None => return broken("a field its struct does not declare"),
                    },
                    Ty::Int(_) | Ty::Bool | Ty::Array(..) | Ty::Enum(_) | Ty::Unit | Ty::Any => {
                        return broken("a field of a place that is not a struct")
                    }
                };
                steps.push(Step::Field(k));
                Ok((root, steps))
            }
            ExprKind::Name(_) => match f.resolution.names[e.0 as usize] {
                Some(root) => Ok((root, Vec::new())),
                None => broken("a place whose name was never resolved"),
            },
            // The checker passes as a place only a binding, an element, or a field of a struct
            // (`E0205`, `E0206`).
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Call { .. }
            | ExprKind::Construct { .. }
            | ExprKind::Variant { .. }
            | ExprKind::Match { .. }
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::If { .. }
            | ExprKind::Error => broken("a place that is not a binding, an element or a field"),
        }
    }

    /// The declared type of a place: its root binding's, when it states one, down its indexes;
    /// a field's, in its struct's declaration, whatever its root states.
    fn place_type(&self, frame: &Frame<'m>, (root, steps): &Place) -> Flow<Declared<'m>> {
        let f = frame.f;
        let (Some(binding), Some(ty)) = (
            f.resolution.bindings.get(root.0 as usize),
            f.typing.bindings.get(root.0 as usize),
        ) else {
            return broken("a place whose root binding does not exist");
        };
        let mut declared = binding.ty.map(|t| (&f.syntax.tree, t));
        let mut ty = ty.clone();
        for step in steps {
            match step {
                Step::Index(..) => {
                    let elem = match ty {
                        Ty::Array(_, elem) => *elem,
                        Ty::Int(_)
                        | Ty::Bool
                        | Ty::Struct(_)
                        | Ty::Enum(_)
                        | Ty::Unit
                        | Ty::Any => return broken("an element of a place that is not an array"),
                    };
                    // A binding declared without a type has none down its elements either.
                    declared = match declared {
                        None => None,
                        Some((tree, t)) => match &tree.ty(t).kind {
                            TypeKind::Array { elem, .. } => Some((tree, *elem)),
                            TypeKind::Int { .. }
                            | TypeKind::Bool
                            | TypeKind::Named(_)
                            | TypeKind::Enum(_)
                            | TypeKind::Error => {
                                return broken("an array place declared of another type")
                            }
                        },
                    };
                    ty = elem;
                }
                Step::Field(k) => {
                    let s = match ty {
                        Ty::Struct(s) => s,
                        Ty::Int(_)
                        | Ty::Bool
                        | Ty::Array(..)
                        | Ty::Enum(_)
                        | Ty::Unit
                        | Ty::Any => return broken("a field of a place that is not a struct"),
                    };
                    let Some(field) = self.module.structs.get(&s).and_then(|d| {
                        d.fields
                            .get(*k)
                            .map(|field| (d.tree(), field.decl, field.ty.clone()))
                    }) else {
                        return broken("a field its struct does not declare");
                    };
                    declared = Some((field.0, field.1));
                    ty = field.2;
                }
            }
        }
        Ok(declared)
    }

    fn read(&self, frame: &Frame<'m>, (root, steps): &Place) -> Flow<Value> {
        let Some(mut v) = frame.slots[root.0 as usize].as_ref() else {
            return broken("a place whose binding holds no value");
        };
        for step in steps {
            v = match step {
                Step::Index(i, span) => element(v, *i, *span)?,
                Step::Field(k) => match v {
                    Value::Struct(_, fields) => match fields.get(*k) {
                        Some((_, f)) => f,
                        None => return broken("a field its struct value does not hold"),
                    },
                    Value::Int(_)
                    | Value::Bool(_)
                    | Value::Array(_)
                    | Value::Enum(..)
                    | Value::Unit => return broken("a field read of a value that is not a struct"),
                },
            };
        }
        Ok(v.clone())
    }

    fn write(&self, frame: &mut Frame<'m>, (root, steps): &Place, new: Value) -> Flow<()> {
        let mut v = frame.slots[root.0 as usize].get_or_insert(Value::Unit);
        for step in steps {
            match step {
                Step::Index(i, span) => match v {
                    Value::Array(items) => {
                        let len = items.len();
                        match i.to_u128().and_then(|i| usize::try_from(i).ok()) {
                            Some(k) if k < len => v = &mut items[k],
                            _ => {
                                return trap(TrapKind::IndexOutOfBounds { index: *i, len }, *span);
                            }
                        }
                    }
                    Value::Int(_)
                    | Value::Bool(_)
                    | Value::Struct(..)
                    | Value::Enum(..)
                    | Value::Unit => return broken("a store into an element of a non-array"),
                },
                Step::Field(k) => match v {
                    Value::Struct(_, fields) => match fields.get_mut(*k) {
                        Some((_, f)) => v = f,
                        None => return broken("a store into a field its struct value lacks"),
                    },
                    Value::Int(_)
                    | Value::Bool(_)
                    | Value::Array(_)
                    | Value::Enum(..)
                    | Value::Unit => return broken("a store into a field of a non-struct"),
                },
            }
        }
        *v = new;
        Ok(())
    }

    fn cond(&mut self, frame: &mut Frame<'m>, e: ExprId) -> Flow<bool> {
        Ok(self.eval(frame, e, Kind::Literal)? == Value::Bool(true))
    }

    /// An order comparison of two integers, the left operand evaluated first.
    #[inline(never)]
    fn compare(
        &mut self,
        frame: &mut Frame<'m>,
        (lhs, rhs): (ExprId, ExprId),
        holds: fn(&Int, &Int) -> bool,
    ) -> Flow<Value> {
        self.touch();
        let (left, right) = self.compared(frame, (lhs, rhs));
        let l = self.int(frame, lhs, left)?;
        let r = self.int(frame, rhs, right)?;
        Ok(Value::Bool(holds(&l, &r)))
    }

    /// What each operand of a comparison gives the literals of the other: its type, an
    /// array's through its elements (`core-calculus.md`, item 5).
    fn compared(&self, frame: &Frame<'m>, (lhs, rhs): (ExprId, ExprId)) -> (Kind, Kind) {
        let ty = |e: ExprId| frame.kinds[e.0 as usize];
        (ty(rhs), ty(lhs))
    }

    fn int(&mut self, frame: &mut Frame<'m>, e: ExprId, ctx: Kind) -> Flow<Int> {
        self.int_as(frame, e, ctx, false)
    }

    /// [`Machine::int`], of an operand of an operator of one expression where `member`.
    fn int_as(&mut self, frame: &mut Frame<'m>, e: ExprId, ctx: Kind, member: bool) -> Flow<Int> {
        match self.eval_as(frame, e, ctx, member)? {
            Value::Int(i) => Ok(i),
            Value::Bool(_)
            | Value::Array(_)
            | Value::Struct(..)
            | Value::Enum(..)
            | Value::Unit => broken("an integer operand that is not an integer"),
        }
    }

    /// The value of `e`, whose integers an observer of the run is shown. `ctx` is the integer
    /// type an expression around `e` gives the literals among its operands, which an
    /// expression of literals alone takes; the place a value is narrowed into gives none
    /// (`core-calculus.md`, item 5).
    fn eval(&mut self, frame: &mut Frame<'m>, e: ExprId, ctx: Kind) -> Flow<Value> {
        self.eval_as(frame, e, ctx, false)
    }

    /// [`Machine::eval`] of `e`, an operand of an operator of one expression where `member`:
    /// `ctx` is then the type of every operand of that expression that has one, which its
    /// literals take whatever the order of the operands (`core-calculus.md`, item 5).
    fn eval_as(
        &mut self,
        frame: &mut Frame<'m>,
        e: ExprId,
        ctx: Kind,
        member: bool,
    ) -> Flow<Value> {
        self.checked(|m| m.eval_here(frame, e, ctx, member))
    }

    /// [`Machine::eval_as`], on this segment.
    fn eval_here(
        &mut self,
        frame: &mut Frame<'m>,
        e: ExprId,
        ctx: Kind,
        member: bool,
    ) -> Flow<Value> {
        let v = self.value(frame, e, ctx, member);
        if self.seen.is_some() || self.trace.is_some() {
            if let Ok(v) = &v {
                self.observe(v, frame, e);
            }
        }
        v
    }

    /// Shows an observer of the run each integer `v`, the value of `e`, holds, and a tracer `v`
    /// itself.
    #[inline(never)]
    fn observe(&mut self, v: &Value, frame: &Frame<'m>, e: ExprId) {
        if let Some(seen) = self.seen.as_mut() {
            show(
                v,
                frame.f.syntax.tree.expr(e).span,
                Shown::Given,
                &mut **seen,
            );
        }
        if let Some(trace) = self.trace.as_mut() {
            trace(Event::Given {
                function: frame.index,
                expr: e,
                value: v,
            });
        }
    }

    /// [`Machine::eval`]'s value. Each construct of more than a line evaluates in a method of its
    /// own, so that a step's host frame holds the locals of the construct it evaluates and no
    /// other's: a build without optimisation keeps a slot for every local of a function, and a
    /// step of a deep nesting would otherwise hold every construct's (`crate::host`).
    fn value(&mut self, frame: &mut Frame<'m>, e: ExprId, ctx: Kind, member: bool) -> Flow<Value> {
        let f = frame.f;
        let tree = &f.syntax.tree;
        let span = tree.expr(e).span;
        // What this expression gives the literals among its operands, the branches of an `if`
        // and the arms of a `match` included: within the operators of one expression, the type
        // of all its operands, which it was given; elsewhere its own type where it has one,
        // since they are each other's other operands, and else the type it was given itself.
        let joined = joins(tree, &frame.kinds, e);
        let here = if member && joined {
            ctx
        } else {
            match frame.kinds[e.0 as usize] {
                own @ Kind::Typed(_) => own,
                Kind::Literal | Kind::Other => ctx,
            }
        };
        match &tree.expr(e).kind {
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Name(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Error => self.leaf(frame, e),
            ExprKind::Unary { op, operand } => {
                self.unary(frame, *op, *operand, (here, joined), span)
            }
            ExprKind::Binary { op, lhs, rhs } => {
                // A shift's amount takes no context type; every other operator's right operand
                // does.
                let shift = matches!(op, BinaryOp::Shl | BinaryOp::Shr);
                self.binary(frame, (*op, shift), (*lhs, *rhs), (here, joined), span)
            }
            // An array literal indexed gives its element's value, so its elements take what
            // the indexing is given; the index takes nothing (`core-calculus.md`, item 5).
            ExprKind::Index { base, index } => self.index(frame, (*base, *index), here, span),
            ExprKind::Field { base, name } => self.field(frame, *base, name),
            ExprKind::Construct { name, fields } => self.construct(frame, name, fields),
            ExprKind::Variant {
                enum_name,
                name,
                payload,
                ..
            } => self.variant(frame, enum_name, name, payload),
            ExprKind::Match { scrutinee, arms } => {
                self.match_value(frame, e, *scrutinee, arms, here, span)
            }
            ExprKind::Call { args, .. } => self.call_value(frame, e, args, span),
            ExprKind::Array(items) => self.array(frame, e, items, here, span),
            ExprKind::Repeat { value, count } => self.repeat(frame, e, *value, *count, here, span),
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => self.if_value(frame, *cond, *then, otherwise.as_ref(), here),
        }
    }

    /// The value of an expression that evaluates no other: a literal, a name, `todo` and
    /// `unreachable`.
    #[inline(never)]
    fn leaf(&self, frame: &Frame<'m>, e: ExprId) -> Flow<Value> {
        let f = frame.f;
        let span = f.syntax.tree.expr(e).span;
        match &f.syntax.tree.expr(e).kind {
            ExprKind::Int { value, .. } => Ok(Value::Int(Int::from_u128(*value))),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Name(_) => match f.resolution.names[e.0 as usize]
                .and_then(|id| frame.slots[id.0 as usize].clone())
            {
                Some(v) => Ok(v),
                None => broken("a name without a value"),
            },
            ExprKind::Todo(reason) => trap(TrapKind::Todo(reason.clone()), span),
            ExprKind::Unreachable(reason) => trap(TrapKind::Unreachable(reason.clone()), span),
            ExprKind::Error => Ok(Value::Unit),
            ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Index { .. }
            | ExprKind::Field { .. }
            | ExprKind::Construct { .. }
            | ExprKind::Variant { .. }
            | ExprKind::Match { .. }
            | ExprKind::Call { .. }
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::If { .. } => broken("a construct that evaluates others, taken for a leaf"),
        }
    }

    /// An `if`: the branch its condition picks gives its value, which takes `here`.
    #[inline(never)]
    fn if_value(
        &mut self,
        frame: &mut Frame<'m>,
        cond: ExprId,
        then: BlockId,
        otherwise: Option<&Else>,
        here: Kind,
    ) -> Flow<Value> {
        if self.cond(frame, cond)? {
            self.block(frame, then, here)
        } else {
            match otherwise {
                Some(Else::Block(b)) => self.block(frame, *b, here),
                Some(Else::If(i)) => self.eval(frame, *i, here),
                None => Ok(Value::Unit),
            }
        }
    }

    /// An element read `base[index]`.
    #[inline(never)]
    fn index(
        &mut self,
        frame: &mut Frame<'m>,
        (base, index): (ExprId, ExprId),
        here: Kind,
        span: Span,
    ) -> Flow<Value> {
        let b = self.eval(frame, base, here)?;
        let i = self.int(frame, index, Kind::Literal)?;
        Ok(element(&b, i, span)?.clone())
    }

    /// A unary operator's value, its operand's literals taking `here` within one expression
    /// where `joined`.
    #[inline(never)]
    fn unary(
        &mut self,
        frame: &mut Frame<'m>,
        op: UnaryOp,
        operand: ExprId,
        (here, joined): (Kind, bool),
        span: Span,
    ) -> Flow<Value> {
        let kind = frame.kinds[operand.0 as usize];
        Ok(match op {
            UnaryOp::Not => Value::Bool(!self.cond(frame, operand)?),
            UnaryOp::Neg => {
                let x = self.int_as(frame, operand, here, joined)?.negate();
                // On an `i128` or `u128` operand, checked at the operator (`core-calculus.md`,
                // item 4).
                match kind.ty().filter(|t| t.bits == 128) {
                    Some(t) => Value::Int(fit(x, t, span)?),
                    None => Value::Int(x),
                }
            }
            UnaryOp::BitNot => {
                let t = match kind {
                    Kind::Typed(Some(t)) => t,
                    Kind::Literal => {
                        worked_in(here, "`~` of literals among integers no type holds")?
                    }
                    // `E0212`: no type is chosen for integers no standard type holds.
                    Kind::Typed(None) | Kind::Other => {
                        return broken("`~` of integers no standard type holds")
                    }
                };
                let x = self.int_as(frame, operand, here, joined)?;
                let x = fit(x, t, frame.f.syntax.tree.expr(operand).span)?;
                Value::Int(Int::from_bits(!x.to_bits(t), t))
            }
        })
    }

    /// A binary operator's value, its operands' literals taking `here` within one expression
    /// where `joined`.
    #[inline(never)]
    fn binary(
        &mut self,
        frame: &mut Frame<'m>,
        (op, shift): (BinaryOp, bool),
        (lhs, rhs): (ExprId, ExprId),
        (here, joined): (Kind, bool),
        span: Span,
    ) -> Flow<Value> {
        match op {
            BinaryOp::And => self.both(frame, (lhs, rhs)),
            BinaryOp::Or => self.either(frame, (lhs, rhs)),
            BinaryOp::Eq | BinaryOp::Ne => self.equal(frame, op == BinaryOp::Eq, (lhs, rhs)),
            BinaryOp::Lt => self.compare(frame, (lhs, rhs), |l, r| l < r),
            BinaryOp::Le => self.compare(frame, (lhs, rhs), |l, r| l <= r),
            BinaryOp::Gt => self.compare(frame, (lhs, rhs), |l, r| l > r),
            BinaryOp::Ge => self.compare(frame, (lhs, rhs), |l, r| l >= r),
            BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitAnd
            | BinaryOp::Shl
            | BinaryOp::Shr
            | BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Rem => {
                self.arithmetic(frame, (op, shift), (lhs, rhs), (here, joined), span)
            }
        }
    }

    /// `lhs and rhs`: the right operand evaluated only where the left one is `true`.
    #[inline(never)]
    fn both(&mut self, frame: &mut Frame<'m>, (lhs, rhs): (ExprId, ExprId)) -> Flow<Value> {
        Ok(Value::Bool(
            self.cond(frame, lhs)? && self.cond(frame, rhs)?,
        ))
    }

    /// `lhs or rhs`: the right operand evaluated only where the left one is `false`.
    #[inline(never)]
    fn either(&mut self, frame: &mut Frame<'m>, (lhs, rhs): (ExprId, ExprId)) -> Flow<Value> {
        Ok(Value::Bool(
            self.cond(frame, lhs)? || self.cond(frame, rhs)?,
        ))
    }

    /// `lhs == rhs` where `eq`, and `lhs != rhs` otherwise, the left operand evaluated first.
    #[inline(never)]
    fn equal(
        &mut self,
        frame: &mut Frame<'m>,
        eq: bool,
        (lhs, rhs): (ExprId, ExprId),
    ) -> Flow<Value> {
        let (left, right) = self.compared(frame, (lhs, rhs));
        let l = self.eval(frame, lhs, left)?;
        let r = self.eval(frame, rhs, right)?;
        Ok(Value::Bool((l == r) == eq))
    }

    /// An arithmetic, bit or shift operator's value, its operands evaluated left to right, the
    /// amount of a shift (`shift`) taking no context type, and every other operand's literals
    /// `here` within one expression where `joined`.
    #[inline(never)]
    fn arithmetic(
        &mut self,
        frame: &mut Frame<'m>,
        (op, shift): (BinaryOp, bool),
        (lhs, rhs): (ExprId, ExprId),
        (here, joined): (Kind, bool),
        span: Span,
    ) -> Flow<Value> {
        let l = self.eval_as(frame, lhs, here, joined)?;
        let r = if shift {
            self.eval(frame, rhs, Kind::Literal)?
        } else {
            self.eval_as(frame, rhs, here, joined)?
        };
        let lt = frame.kinds[lhs.0 as usize].ty();
        let rt = frame.kinds[rhs.0 as usize].ty();
        self.arith(frame, op, [(l, lt, lhs), (r, rt, rhs)], here, span)
    }

    /// A field read `base.name`. The base is a value first, as an index's is, so that a field
    /// read through an index reads the value the base had when it was evaluated, its indexes
    /// evaluated, and each checked, from left to right (`core-calculus.md`, items 10 and 12).
    #[inline(never)]
    fn field(&mut self, frame: &mut Frame<'m>, base: ExprId, name: &str) -> Flow<Value> {
        match self.eval(frame, base, Kind::Literal)? {
            Value::Array(items) => Ok(Value::Int(Int::from_u128(items.len() as u128))),
            Value::Struct(_, fields) => match fields.into_iter().find(|(n, _)| n == name) {
                Some((_, v)) => Ok(v),
                None => broken("a field its struct value does not hold"),
            },
            Value::Int(_) | Value::Bool(_) | Value::Enum(..) | Value::Unit => {
                broken("a field read of a value that is neither an array nor a struct")
            }
        }
    }

    /// A struct built: each value is narrowed into its field's type and refinement where it
    /// lands, in the order written, which is the declaration's (`core-calculus.md`, item 13).
    #[inline(never)]
    fn construct(
        &mut self,
        frame: &mut Frame<'m>,
        name: &str,
        fields: &[FieldInit],
    ) -> Flow<Value> {
        let tree = &frame.f.syntax.tree;
        let def = self.module.structs.get(name);
        let mut values = Vec::new();
        for fi in fields {
            let field = def.and_then(|d| d.field(&fi.name)).map(|(_, f)| f);
            let fty = field.map_or(Ty::Any, |f| f.ty.clone());
            let declared = def.zip(field).map(|(d, f)| (d.tree(), f.decl));
            let v = self.eval(frame, fi.value, Kind::Literal)?;
            let at = tree.expr(fi.value).span;
            let v = self.narrow(frame, v, declared, &fty, at)?;
            values.push((fi.name.clone(), v));
        }
        Ok(Value::Struct(name.to_string(), values))
    }

    /// A variant built: its payload is built as a struct is (`core-calculus.md`, item 14).
    #[inline(never)]
    fn variant(
        &mut self,
        frame: &mut Frame<'m>,
        enum_name: &str,
        name: &str,
        payload: &Payload,
    ) -> Flow<Value> {
        let tree = &frame.f.syntax.tree;
        let def = self.module.structs.get_enum(enum_name);
        let variant = def.and_then(|d| d.variant(name)).map(|(_, v)| v);
        let mut values = Vec::new();
        let fields: &[FieldInit] = match payload {
            Payload::Named(fields) => fields,
            Payload::None => &[],
            // A variant built by position is `E0226`.
            Payload::Positional(_) => return broken("a variant built by position"),
        };
        for fi in fields {
            let field = variant.and_then(|v| v.field(&fi.name)).map(|(_, f)| f);
            let fty = field.map_or(Ty::Any, |f| f.ty.clone());
            let declared = def.zip(field).map(|(d, f)| (d.tree(), f.decl));
            let v = self.eval(frame, fi.value, Kind::Literal)?;
            let at = tree.expr(fi.value).span;
            let v = self.narrow(frame, v, declared, &fty, at)?;
            values.push((fi.name.clone(), v));
        }
        Ok(Value::Enum(enum_name.to_string(), name.to_string(), values))
    }

    /// The `match` at `e`: the arm whose pattern matches gives its value, which takes `here`.
    #[inline(never)]
    fn match_value(
        &mut self,
        frame: &mut Frame<'m>,
        e: ExprId,
        scrutinee: ExprId,
        arms: &[Arm],
        here: Kind,
        span: Span,
    ) -> Flow<Value> {
        let f = frame.f;
        let tree = &f.syntax.tree;
        let v = self.eval(frame, scrutinee, Kind::Literal)?;
        // An integer scrutinee is narrowed into its type, which the arms cover
        // ([`match-scrutinee-narrowing`]); literals alone are an `i64`.
        let read = match &f.typing.exprs[scrutinee.0 as usize] {
            // Literals alone are an `i64`; integers no type holds are `E0212`, never read in a
            // type chosen for them.
            Ty::Int(_) => match frame.kinds[scrutinee.0 as usize] {
                Kind::Literal => Ty::Int(IntType::One(I64)),
                Kind::Typed(Some(t)) => Ty::Int(IntType::One(t)),
                Kind::Typed(None) | Kind::Other => {
                    return broken("a scrutinee of integers no standard type holds")
                }
            },
            t @ (Ty::Bool | Ty::Array(..) | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit | Ty::Any) => {
                t.clone()
            }
        };
        let v = self.narrow_plain(v, &read, tree.expr(scrutinee).span)?;
        for (k, arm) in arms.iter().enumerate() {
            if !self.matches(frame, e, k, &arm.pattern.kind, &v)? {
                continue;
            }
            return match arm.body {
                ArmBody::Expr(x) => self.eval(frame, x, here),
                ArmBody::Block(b) => self.block(frame, b, here),
            };
        }
        // A `match` the checker passes is exhaustive.
        trap(
            TrapKind::Unreachable("no arm of this `match` matches".into()),
            span,
        )
    }

    /// The call at `e`: its arguments evaluated, a `mut` one read from its place, then the call,
    /// then each `mut` place takes back what the callee left.
    #[inline(never)]
    fn call_value(
        &mut self,
        frame: &mut Frame<'m>,
        e: ExprId,
        args: &[Arg],
        span: Span,
    ) -> Flow<Value> {
        let Some(index) = frame.f.resolution.calls[e.0 as usize] else {
            return broken("a call that was never resolved");
        };
        // A function without a body stops the run before its arguments are evaluated.
        self.has_body(index)?;
        let given = self.arguments(frame, args)?;
        self.called(frame, index, given, span)
    }

    /// The arguments of a call, evaluated left to right: their values, where each was written,
    /// and the place each `mut` argument names, which the call's final value of it lands in.
    #[inline(never)]
    fn arguments(&mut self, frame: &mut Frame<'m>, args: &[Arg]) -> Flow<Given> {
        let tree = &frame.f.syntax.tree;
        let mut values = Vec::new();
        let mut spans = Vec::new();
        let mut places = Vec::new();
        for a in args {
            if a.is_mut {
                let (v, path) = self.mut_argument(frame, a.value)?;
                values.push(v);
                places.push(Some(path));
            } else {
                values.push(self.eval(frame, a.value, Kind::Literal)?);
                places.push(None);
            }
            spans.push(tree.expr(a.value).span);
        }
        Ok((values, spans, places))
    }

    /// A `mut` argument `e`: the value its place holds, and the place.
    #[inline(never)]
    fn mut_argument(&mut self, frame: &mut Frame<'m>, e: ExprId) -> Flow<(Value, Place)> {
        let path = self.place(frame, e)?;
        let v = self.read(frame, &path)?;
        Ok((v, path))
    }

    /// The call of function `index` at `span` on arguments `given`, whose final values land in
    /// the places the `mut` ones name.
    #[inline(never)]
    fn called(
        &mut self,
        frame: &mut Frame<'m>,
        index: usize,
        (values, spans, places): Given,
        span: Span,
    ) -> Flow<Value> {
        let caller = Some((frame.index, frame.measure.clone()));
        let (result, finals) = self.call(index, values, &spans, span, caller)?;
        self.handed_back(frame, places, finals, span)?;
        Ok(result)
    }

    /// The final values of a call's parameters, `finals`, landed in the places its `mut`
    /// arguments name, `places`, each entering its place's refinement.
    #[inline(never)]
    fn handed_back(
        &mut self,
        frame: &mut Frame<'m>,
        places: Vec<Option<Place>>,
        finals: Vec<Value>,
        span: Span,
    ) -> Flow<()> {
        for (place, v) in places.into_iter().zip(finals) {
            if let Some(path) = place {
                let declared = self.place_type(frame, &path)?;
                self.refined(frame, &v, declared, span)?;
                self.write(frame, &path, v)?;
            }
        }
        Ok(())
    }

    /// The array literal at `e`: each element is narrowed into the literal's element type, at
    /// the literal (`core-calculus.md`, item 4).
    #[inline(never)]
    fn array(
        &mut self,
        frame: &mut Frame<'m>,
        e: ExprId,
        items: &[ExprId],
        here: Kind,
        span: Span,
    ) -> Flow<Value> {
        let f = frame.f;
        let elem_ty = match &f.typing.exprs[e.0 as usize] {
            Ty::Array(_, elem) => elem.as_ref(),
            Ty::Int(_) | Ty::Bool | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit | Ty::Any => {
                return broken("an array literal whose type is not an array")
            }
        };
        let mut values = Vec::new();
        for &i in items {
            let v = self.eval(frame, i, here)?;
            values.push(self.narrow_plain(v, elem_ty, span)?);
        }
        Ok(Value::Array(values))
    }

    /// The repeat `[value; count]` at `e`.
    #[inline(never)]
    fn repeat(
        &mut self,
        frame: &mut Frame<'m>,
        e: ExprId,
        value: ExprId,
        count: u128,
        here: Kind,
        span: Span,
    ) -> Flow<Value> {
        if count > self.limits.max_array {
            return Err(stopped(Stop::TooLarge(count)));
        }
        let f = frame.f;
        let v = self.eval(frame, value, here)?;
        let v = match &f.typing.exprs[e.0 as usize] {
            Ty::Array(_, elem_ty) => self.narrow_plain(v, elem_ty, span)?,
            Ty::Int(_) | Ty::Bool | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit | Ty::Any => {
                return broken("an array literal whose type is not an array")
            }
        };
        Ok(Value::Array(vec![v; count as usize]))
    }

    /// Whether pattern `p`, of arm `k` of the `match` at `m`, matches `v`; when it does, its
    /// bindings take the payload's fields, which the arm only reads.
    fn matches(
        &mut self,
        frame: &mut Frame<'m>,
        m: ExprId,
        k: usize,
        p: &PatternKind,
        v: &Value,
    ) -> Flow<bool> {
        self.touch();
        let mismatch = || broken("a pattern of another kind than the value it matches");
        match p {
            PatternKind::Wildcard => Ok(true),
            PatternKind::Bool(b) => match v {
                Value::Bool(x) => Ok(b == x),
                Value::Int(_)
                | Value::Array(_)
                | Value::Struct(..)
                | Value::Enum(..)
                | Value::Unit => mismatch(),
            },
            PatternKind::Int {
                negative, value, ..
            } => match v {
                Value::Int(x) => {
                    let lit = Int::from_u128(*value);
                    Ok(*x == if *negative { lit.negate() } else { lit })
                }
                Value::Bool(_)
                | Value::Array(_)
                | Value::Struct(..)
                | Value::Enum(..)
                | Value::Unit => mismatch(),
            },
            PatternKind::Variant { name, bindings, .. } => {
                let (variant, fields) = match v {
                    Value::Enum(_, variant, fields) => (variant, fields),
                    Value::Int(_)
                    | Value::Bool(_)
                    | Value::Array(_)
                    | Value::Struct(..)
                    | Value::Unit => return mismatch(),
                };
                if name != variant {
                    return Ok(false);
                }
                for (i, b) in bindings.iter().flatten().enumerate() {
                    let Some(id) = frame.f.resolution.patterns.get(&(m, k, i)) else {
                        continue;
                    };
                    let field = match &b.field {
                        Some((f, _)) => fields.iter().find(|(n, _)| n == f),
                        None => fields.get(i),
                    };
                    frame.slots[id.0 as usize] = field.map(|(_, x)| x.clone());
                }
                Ok(true)
            }
        }
    }

    /// An arithmetic, bit or shift operation on two evaluated operands with their static types.
    fn arith(
        &mut self,
        frame: &mut Frame<'m>,
        op: BinaryOp,
        [(l, lt, le), (r, rt, re)]: [(Value, Option<IntTy>, ExprId); 2],
        ctx: Kind,
        span: Span,
    ) -> Flow<Value> {
        self.touch();
        let tree = &frame.f.syntax.tree;
        let (Value::Int(a), Value::Int(b)) = (l, r) else {
            return broken("an integer operand that is not an integer");
        };
        // A bit operation works on the operands' bits in one type: either operand's, or the
        // place's.
        let bits = |on: fn(u128, u128) -> u128| -> Flow<Value> {
            let t = match lt.or(rt) {
                Some(t) => t,
                None => worked_in(
                    ctx,
                    "a bit operation of literals among integers no type holds",
                )?,
            };
            let x = fit(a, t, tree.expr(le).span)?.to_bits(t);
            let y = fit(b, t, tree.expr(re).span)?.to_bits(t);
            Ok(Value::Int(Int::from_bits(on(x, y), t)))
        };
        let result = match op {
            BinaryOp::Shl | BinaryOp::Shr => {
                let t = match lt {
                    Some(t) => t,
                    None => worked_in(ctx, "a shift of literals among integers no type holds")?,
                };
                let k = match b.to_u128() {
                    Some(k) if k < u128::from(t.bits) => k as u32,
                    _ => {
                        return trap(
                            TrapKind::ShiftAmount {
                                amount: b,
                                width: t.bits,
                            },
                            span,
                        )
                    }
                };
                let shifted = if op == BinaryOp::Shl {
                    a.checked_shl(k)
                } else {
                    Some(a.shr_floor(k))
                };
                return match shifted {
                    Some(v) => fit(v, t, span).map(Value::Int),
                    None => trap(
                        TrapKind::Narrowing {
                            value: None,
                            into: t.name,
                        },
                        span,
                    ),
                };
            }
            BinaryOp::BitAnd => return bits(|x, y| x & y),
            BinaryOp::BitOr => return bits(|x, y| x | y),
            BinaryOp::BitXor => return bits(|x, y| x ^ y),
            BinaryOp::Div | BinaryOp::Rem if b == Int::ZERO => {
                return trap(TrapKind::DivisionByZero, span);
            }
            BinaryOp::Add => a.checked_add(b),
            BinaryOp::Sub => a.checked_sub(b),
            BinaryOp::Mul => a.checked_mul(b),
            BinaryOp::Div => a.checked_div(b),
            BinaryOp::Rem => a.checked_rem(b),
            // `eval` gives these their own arms, which never come here.
            BinaryOp::Or
            | BinaryOp::And
            | BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => {
                return broken("a logical or comparison operator taken as arithmetic")
            }
        };
        // An operation on 128-bit operands may leave 128 bits, so it is narrowed into their
        // type (`core-calculus.md`, item 4).
        let wide = [lt, rt].into_iter().flatten().find(|t| t.bits == 128);
        match (result, wide) {
            (Some(v), Some(t)) => fit(v, t, span).map(Value::Int),
            (Some(v), None) => Ok(Value::Int(v)),
            (None, Some(t)) => trap(
                TrapKind::Narrowing {
                    value: None,
                    into: t.name,
                },
                span,
            ),
            (None, None) => trap(TrapKind::IntermediateOverflow, span),
        }
    }

    /// Narrows a value into a place of static type `plain`, and into the refinement of its
    /// declared type `declared` when it has one.
    fn narrow(
        &mut self,
        frame: &Frame<'m>,
        v: Value,
        declared: Declared<'m>,
        plain: &Ty,
        span: Span,
    ) -> Flow<Value> {
        self.touch();
        let v = self.narrow_plain(v, plain, span)?;
        self.refined(frame, &v, declared, span)?;
        Ok(v)
    }

    /// Narrows a value into the integer ranges of a static type, elementwise for an array,
    /// showing an observer of the run each integer it narrows. Each level of a nested value is a
    /// step of its own, which may continue on a new segment of the host's stack
    /// (`Machine::checked`), since a value nests as deep as its type does.
    fn narrow_plain(&mut self, v: Value, plain: &Ty, span: Span) -> Flow<Value> {
        self.checked(|m| m.narrow_plain_here(v, plain, span))
    }

    /// [`Machine::narrow_plain`], on this segment.
    fn narrow_plain_here(&mut self, v: Value, plain: &Ty, span: Span) -> Flow<Value> {
        let mismatch = || broken("a value of another shape than its static type");
        match plain {
            // A literal's value is narrowed as any value is: the place gives it no type.
            Ty::Int(_) => match v {
                Value::Int(x) => self.narrow_int(x, plain, span),
                Value::Bool(_)
                | Value::Array(_)
                | Value::Struct(..)
                | Value::Enum(..)
                | Value::Unit => mismatch(),
            },
            Ty::Array(_, elem) => match v {
                Value::Array(items) => self.narrow_elements(items, elem, span),
                Value::Int(_)
                | Value::Bool(_)
                | Value::Struct(..)
                | Value::Enum(..)
                | Value::Unit => mismatch(),
            },
            // A struct's fields, each into its type, in the order of the declaration: a value
            // built in the program already lies there, one from outside it may not.
            Ty::Struct(s) => match v {
                Value::Struct(name, fields) => self.narrow_struct(s, name, fields, span),
                Value::Int(_)
                | Value::Bool(_)
                | Value::Array(_)
                | Value::Enum(..)
                | Value::Unit => mismatch(),
            },
            // A variant's payload, each field into its type, likewise.
            Ty::Enum(e) => match v {
                Value::Enum(name, variant, fields) => {
                    self.narrow_variant(e, (name, variant), fields, span)
                }
                Value::Int(_)
                | Value::Bool(_)
                | Value::Array(_)
                | Value::Struct(..)
                | Value::Unit => mismatch(),
            },
            Ty::Bool => match v {
                Value::Bool(_) => Ok(v),
                Value::Int(_)
                | Value::Array(_)
                | Value::Struct(..)
                | Value::Enum(..)
                | Value::Unit => mismatch(),
            },
            Ty::Unit => match v {
                Value::Unit => Ok(v),
                Value::Int(_)
                | Value::Bool(_)
                | Value::Array(_)
                | Value::Struct(..)
                | Value::Enum(..) => mismatch(),
            },
            // `todo`, `unreachable` and a value whose every branch leaves have any type, and
            // never give a value to narrow.
            Ty::Any => broken("a value of no type"),
        }
    }

    /// An integer narrowed into the integer type `plain`, shown to an observer of the run.
    #[inline(never)]
    fn narrow_int(&mut self, x: Int, plain: &Ty, span: Span) -> Flow<Value> {
        match int_ty(plain) {
            Some(t) => {
                if let Some(seen) = self.seen.as_mut() {
                    seen(span, x, Shown::Narrowed);
                }
                fit(x, t, span).map(Value::Int)
            }
            None => Ok(Value::Int(x)),
        }
    }

    /// An array's elements, each narrowed into its element type `elem`.
    #[inline(never)]
    fn narrow_elements(&mut self, items: Vec<Value>, elem: &Ty, span: Span) -> Flow<Value> {
        let mut out = Vec::new();
        for v in items {
            out.push(self.narrow_plain(v, elem, span)?);
        }
        Ok(Value::Array(out))
    }

    /// A value of struct `s`, its fields each narrowed into its type, in the order of the
    /// declaration.
    #[inline(never)]
    fn narrow_struct(
        &mut self,
        s: &str,
        name: String,
        fields: Vec<(String, Value)>,
        span: Span,
    ) -> Flow<Value> {
        let Some(def) = self.module.structs.get(s) else {
            return broken("a struct the module does not declare");
        };
        let mut out = Vec::new();
        for ((n, v), d) in fields.into_iter().zip(&def.fields) {
            out.push((n, self.narrow_plain(v, &d.ty, span)?));
        }
        Ok(Value::Struct(name, out))
    }

    /// A variant of enum `e`, its payload's fields each narrowed into its type, likewise.
    #[inline(never)]
    fn narrow_variant(
        &mut self,
        e: &str,
        (name, variant): (String, String),
        fields: Vec<(String, Value)>,
        span: Span,
    ) -> Flow<Value> {
        let Some((_, def)) = self
            .module
            .structs
            .get_enum(e)
            .and_then(|d| d.variant(&variant))
        else {
            return broken("a variant its enum does not declare");
        };
        let mut out = Vec::new();
        for ((n, v), d) in fields.into_iter().zip(&def.fields) {
            out.push((n, self.narrow_plain(v, &d.ty, span)?));
        }
        Ok(Value::Enum(name, variant, out))
    }

    /// Checks a value against the refinements of a declared type, elementwise for an array and
    /// fieldwise for a struct, whose fields have the refinements its declaration gives them. Each
    /// level of a nested value is a step of its own, which may continue on a new segment of the
    /// host's stack (`Machine::checked`), since a value nests as deep as its type does.
    fn refined(
        &mut self,
        frame: &Frame<'m>,
        v: &Value,
        declared: Declared<'_>,
        span: Span,
    ) -> Flow<()> {
        self.checked(|m| m.refined_here(frame, v, declared, span))
    }

    /// [`Machine::refined`], on this segment.
    fn refined_here(
        &mut self,
        frame: &Frame<'m>,
        v: &Value,
        declared: Declared<'_>,
        span: Span,
    ) -> Flow<()> {
        let Some((tree, id)) = declared else {
            return Ok(());
        };
        let mismatch = || broken("a value of another shape than its declared type");
        match &tree.ty(id).kind {
            TypeKind::Int { refinement, .. } => match v {
                Value::Int(x) => match refinement {
                    Some(r) => self.in_range(frame, *x, r, span),
                    None => Ok(()),
                },
                Value::Bool(_)
                | Value::Array(_)
                | Value::Struct(..)
                | Value::Enum(..)
                | Value::Unit => mismatch(),
            },
            TypeKind::Bool => match v {
                Value::Bool(_) => Ok(()),
                Value::Int(_)
                | Value::Array(_)
                | Value::Struct(..)
                | Value::Enum(..)
                | Value::Unit => mismatch(),
            },
            TypeKind::Array { elem, .. } => match v {
                Value::Array(items) => {
                    for item in items {
                        self.refined(frame, item, Some((tree, *elem)), span)?;
                    }
                    Ok(())
                }
                Value::Int(_)
                | Value::Bool(_)
                | Value::Struct(..)
                | Value::Enum(..)
                | Value::Unit => mismatch(),
            },
            TypeKind::Named(s) => match v {
                Value::Struct(_, fields) => {
                    let Some(def) = self.module.structs.get(s) else {
                        return broken("a struct the module does not declare");
                    };
                    for ((_, v), d) in fields.iter().zip(&def.fields) {
                        self.refined(frame, v, Some((def.tree(), d.decl)), span)?;
                    }
                    Ok(())
                }
                Value::Int(_)
                | Value::Bool(_)
                | Value::Array(_)
                | Value::Enum(..)
                | Value::Unit => mismatch(),
            },
            TypeKind::Enum(e) => match v {
                Value::Enum(_, variant, fields) => {
                    let Some(def) = self.module.structs.get_enum(e) else {
                        return broken("an enum the module does not declare");
                    };
                    let Some((_, v)) = def.variant(variant) else {
                        return broken("a variant its enum does not declare");
                    };
                    for ((_, x), d) in fields.iter().zip(&v.fields) {
                        self.refined(frame, x, Some((def.tree(), d.decl)), span)?;
                    }
                    Ok(())
                }
                Value::Int(_)
                | Value::Bool(_)
                | Value::Array(_)
                | Value::Struct(..)
                | Value::Unit => mismatch(),
            },
            // A checked module has no type a diagnostic replaced.
            TypeKind::Error => broken("a value of a type a diagnostic replaced"),
        }
    }

    fn in_range(&self, frame: &Frame<'m>, x: Int, r: &Refinement, span: Span) -> Flow<()> {
        let lo = self.bound(frame, &r.lo)?;
        let hi = self.bound(frame, &r.hi)?;
        let below = if r.inclusive { x <= hi } else { x < hi };
        if lo <= x && below {
            return Ok(());
        }
        let range = format!("{}..{}{}", lo, if r.inclusive { "=" } else { "" }, hi);
        trap(TrapKind::Refinement { value: x, range }, span)
    }

    /// The value of a refinement bound in this frame: a name's an integer binding's, a length's
    /// an array binding's, as name resolution and the checker require.
    fn bound(&self, frame: &Frame<'m>, b: &Bound) -> Flow<Int> {
        let slot = |name: &str| {
            frame
                .f
                .resolution
                .bindings
                .iter()
                .position(|binding| binding.name == name)
                .and_then(|i| frame.slots[i].as_ref())
        };
        Ok(match b {
            Bound::Int { negative, value } => {
                let v = Int::from_u128(*value);
                if *negative {
                    v.negate()
                } else {
                    v
                }
            }
            Bound::Name(n) => match slot(n) {
                Some(Value::Int(i)) => *i,
                Some(
                    Value::Bool(_)
                    | Value::Array(_)
                    | Value::Struct(..)
                    | Value::Enum(..)
                    | Value::Unit,
                )
                | None => return broken("a refinement bound that names no integer"),
            },
            Bound::Len(n) => match slot(n) {
                Some(Value::Array(items)) => Int::from_u128(items.len() as u128),
                Some(
                    Value::Int(_)
                    | Value::Bool(_)
                    | Value::Struct(..)
                    | Value::Enum(..)
                    | Value::Unit,
                )
                | None => return broken("a refinement bound that names no array"),
            },
        })
    }
}

/// Shows `seen` each integer of `v`, the value given at `span` for the reason `why`: itself,
/// or those of an array's elements, through the arrays nested in it. A struct's and a
/// variant's fields were shown where they were built.
fn show(v: &Value, span: Span, why: Shown, seen: &mut (dyn FnMut(Span, Int, Shown) + Send)) {
    // A loop over the elements, first to last at every depth, so that a value nested as deep as
    // its type takes no host stack for its depth (`crate::host`).
    let mut pending = vec![v];
    while let Some(v) = pending.pop() {
        match v {
            Value::Int(x) => seen(span, *x, why),
            Value::Array(items) => pending.extend(items.iter().rev()),
            Value::Bool(_) | Value::Struct(..) | Value::Enum(..) | Value::Unit => {}
        }
    }
}

/// `x`, when it fits `t`; a narrowing trap otherwise.
fn fit(x: Int, t: IntTy, span: Span) -> Flow<Int> {
    if x.fits(t) {
        Ok(x)
    } else {
        trap(
            TrapKind::Narrowing {
                value: Some(x),
                into: t.name,
            },
            span,
        )
    }
}

/// The element `i` of an array, or an index trap.
fn element(v: &Value, i: Int, span: Span) -> Flow<&Value> {
    let items = match v {
        Value::Array(items) => items,
        Value::Int(_) | Value::Bool(_) | Value::Struct(..) | Value::Enum(..) | Value::Unit => {
            return broken("an index into a value that is not an array")
        }
    };
    match i.to_u128().and_then(|i| usize::try_from(i).ok()) {
        Some(k) if k < items.len() => Ok(&items[k]),
        _ => trap(
            TrapKind::IndexOutOfBounds {
                index: i,
                len: items.len(),
            },
            span,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recurses `levels` times, each frame holding a KiB, and probes at the bottom as a function
    /// a step runs without checking again does.
    #[inline(never)]
    fn below(m: &mut Machine<'_>, levels: u32) {
        let pad = [0u8; 1024];
        std::hint::black_box(&pad);
        if levels == 0 {
            m.touch();
        } else {
            below(m, levels - 1);
        }
        std::hint::black_box(&pad);
    }

    /// A machine's probe records how far below its last check the host stack reaches where it
    /// stands, and how far below its start the current call does, so that a step's measure takes
    /// in what runs below the step's last check (`crate::host`): nine frames of a KiB each lie at
    /// least 8 KiB below where the machine began.
    #[test]
    fn test_a_machines_probe_records_how_deep_a_step_reaches() {
        let module =
            match mente_sema::check_program_without_proofs("module m\npub fn f() -> u8 = 0\n") {
                Ok(m) => m,
                Err(d) => panic!("{:?}", d),
            };
        let mut m = Machine::new(&module, Limits::default(), None);
        assert_eq!((m.deepest_step(), m.per_call()), (0, 0));
        below(&mut m, 8);
        assert!(m.deepest_step() >= 4096, "{}", m.deepest_step());
        assert!(m.per_call() >= 4096, "{}", m.per_call());
    }
}
