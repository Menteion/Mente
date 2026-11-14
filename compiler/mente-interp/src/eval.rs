//! The interpreter proper (`spec/core-calculus.md`, items 4, 6, 7, 9 and 10).
//!
//! A [`Frame`] holds one call's bindings in slots indexed like the function's resolution, which
//! names each binding once (no shadowing), so a block needs no scope of its own. Control leaves
//! a statement by an [`Exit`]: a `return`, a `break`, a `continue`, or a stop.

use mente_sema::names::BindingId;
use mente_sema::structs::Structs;
use mente_sema::types::{Int as IntType, IntTy, Ty, I64};
use mente_sema::{CheckedFn, CheckedModule};
use mente_syntax::tree::{
    ArmBody, AssignOp, BinaryOp, BlockId, Body, Bound, ContractKind, Else, ExprId, ExprKind,
    PatternKind, Payload, Refinement, StmtId, StmtKind, Tree, TypeId, TypeKind, UnaryOp,
};
use mente_syntax::Span;

use crate::{Int, Limits, Observer, Stop, Trap, TrapKind, Value};

/// How control leaves a statement other than by its end.
enum Exit {
    Return(Value, Span),
    Break,
    Continue,
    Stop(Stop),
}

impl From<Stop> for Exit {
    fn from(s: Stop) -> Exit {
        Exit::Stop(s)
    }
}

type Flow<T> = Result<T, Exit>;

fn trap<T>(kind: TrapKind, span: Span) -> Flow<T> {
    Err(Exit::Stop(Stop::Trap(Trap { kind, span })))
}

/// A state the checks rule out, reported rather than run on with a made-up value.
fn broken<T>(what: &str) -> Flow<T> {
    Err(Exit::Stop(Stop::Broken(what.to_string())))
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

pub(crate) struct Machine<'m> {
    module: &'m CheckedModule,
    limits: Limits,
    steps: u64,
    depth: usize,
    /// What is shown each integer an expression gives, when a run is observed.
    seen: Option<Observer<'m>>,
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
            depth: 0,
            seen,
        }
    }

    /// Calls the function `name` from outside the program.
    pub(crate) fn enter(
        &mut self,
        name: &str,
        args: Vec<Value>,
    ) -> Result<(Value, Vec<Value>), Stop> {
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

    fn step(&mut self) -> Flow<()> {
        self.steps += 1;
        if self.steps > self.limits.max_steps {
            return Err(Exit::Stop(Stop::OutOfSteps));
        }
        Ok(())
    }

    /// Runs function `index` on `args`, each narrowed into its parameter's type at the span in
    /// `spans`. Returns the result and the final value of every parameter, for the caller to
    /// copy back into the places it passed as `mut`.
    fn call(
        &mut self,
        index: usize,
        args: Vec<Value>,
        spans: &[Span],
        at: Span,
        caller: Option<(usize, Option<Vec<Int>>)>,
    ) -> Result<(Value, Vec<Value>), Stop> {
        let f = self.function(index)?;
        if self.depth >= self.limits.max_depth {
            return Err(Stop::Trap(Trap {
                kind: TrapKind::StackOverflow,
                span: at,
            }));
        }
        let mut frame = Frame {
            index,
            f,
            slots: vec![None; f.resolution.bindings.len()],
            measure: None,
        };
        self.depth += 1;
        let outcome = self.run(&mut frame, args, spans, at, caller);
        self.depth -= 1;
        match outcome {
            Ok(v) => Ok(v),
            Err(Exit::Stop(s)) => Err(s),
            // A checked body never lets `break` or `continue` leave it.
            Err(_) => Ok((Value::Unit, Vec::new())),
        }
    }

    fn run(
        &mut self,
        frame: &mut Frame<'m>,
        args: Vec<Value>,
        spans: &[Span],
        at: Span,
        caller: Option<(usize, Option<Vec<Int>>)>,
    ) -> Flow<(Value, Vec<Value>)> {
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
        let result_ty = f.syntax.result.map_or(Ty::Unit, |t| Ty::of(tree, t));
        let declared = f.syntax.result.map(|t| (tree, t));
        let (result, ret_span) = match f.syntax.body {
            // An expression body leaves by `return` too, from a branch of an `if` or an arm of
            // a `match` in it, which ends this call alone.
            Some(Body::Expr(e)) => match self.eval(frame, e, int_ty(&result_ty)) {
                Ok(v) => {
                    let span = tree.expr(e).span;
                    (self.narrow(frame, v, declared, &result_ty, span)?, span)
                }
                Err(Exit::Return(v, span)) => (v, span),
                Err(e) => return Err(e),
            },
            Some(Body::Block(b)) => match self.block(frame, b) {
                Ok(v) => {
                    let span = tree
                        .block(b)
                        .stmts
                        .last()
                        .map_or(tree.block(b).span, |&s| tree.stmt(s).span);
                    (self.narrow(frame, v, declared, &result_ty, span)?, span)
                }
                Err(Exit::Return(v, span)) => (v, span),
                Err(e) => return Err(e),
            },
            None => (Value::Unit, at),
        };
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

    /// The value of a block: its last statement's, when that is an expression.
    fn block(&mut self, frame: &mut Frame<'m>, b: BlockId) -> Flow<Value> {
        let tree = &frame.f.syntax.tree;
        let mut last = Value::Unit;
        for &s in &tree.block(b).stmts {
            last = self.stmt(frame, s)?;
        }
        Ok(last)
    }

    fn stmt(&mut self, frame: &mut Frame<'m>, s: StmtId) -> Flow<Value> {
        self.step()?;
        let f = frame.f;
        let tree = &f.syntax.tree;
        match &tree.stmt(s).kind {
            StmtKind::Expr(e) => return self.eval(frame, *e, None),
            StmtKind::Discard(e) => {
                self.eval(frame, *e, None)?;
            }
            StmtKind::Binding { ty, init, .. } => {
                let Some(id) = f.resolution.stmts[s.0 as usize] else {
                    return broken("a binding statement that declares no binding");
                };
                let plain = f.typing.bindings[id.0 as usize].clone();
                let v = self.eval(frame, *init, int_ty(&plain))?;
                let declared = ty.map(|t| (tree, t));
                let v = self.narrow(frame, v, declared, &plain, tree.expr(*init).span)?;
                frame.slots[id.0 as usize] = Some(v);
            }
            StmtKind::Assign { place, op, value } => {
                self.assign(frame, *place, *op, *value, tree.stmt(s).span)?
            }
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => self.while_loop(frame, *cond, invariants, decreases, *body)?,
            StmtKind::Guard { cond, otherwise } => {
                if !self.cond(frame, *cond)? {
                    self.block(frame, *otherwise)?;
                }
            }
            StmtKind::Return(value) => {
                let Some(e) = value else {
                    return Err(Exit::Return(Value::Unit, tree.stmt(s).span));
                };
                let result_ty = f.syntax.result.map_or(Ty::Unit, |t| Ty::of(tree, t));
                let span = tree.expr(*e).span;
                let v = self.eval(frame, *e, int_ty(&result_ty))?;
                let declared = f.syntax.result.map(|t| (tree, t));
                let v = self.narrow(frame, v, declared, &result_ty, span)?;
                return Err(Exit::Return(v, span));
            }
            StmtKind::Break => return Err(Exit::Break),
            StmtKind::Continue => return Err(Exit::Continue),
        }
        Ok(Value::Unit)
    }

    fn while_loop(
        &mut self,
        frame: &mut Frame<'m>,
        cond: ExprId,
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
            match self.block(frame, body) {
                Ok(_) | Err(Exit::Continue) => {}
                Err(Exit::Break) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }

    /// The values of a measure, each of which must not be negative.
    fn measure(&mut self, frame: &mut Frame<'m>, exprs: &[ExprId]) -> Flow<Vec<Int>> {
        let tree = &frame.f.syntax.tree;
        let mut values = Vec::new();
        for &e in exprs {
            let v = self.int(frame, e, None)?;
            if v.is_negative() {
                return trap(TrapKind::Decreases(tree.render_expr(e)), tree.expr(e).span);
            }
            values.push(v);
        }
        Ok(values)
    }

    fn assign(
        &mut self,
        frame: &mut Frame<'m>,
        place: ExprId,
        op: AssignOp,
        value: ExprId,
        whole: Span,
    ) -> Flow<()> {
        let f = frame.f;
        let tree = &f.syntax.tree;
        let path = self.place(frame, place)?;
        let plain = f.typing.exprs[place.0 as usize].clone();
        let declared = self.place_type(frame, &path)?;
        let new = match op {
            AssignOp::Set => self.eval(frame, value, int_ty(&plain))?,
            AssignOp::Compound(op) => {
                let current = self.read(frame, &path)?;
                let rhs = self.eval(frame, value, int_ty(&plain))?;
                let rt = f.typing.exprs[value.0 as usize].clone();
                let operands = [(current, &plain, place), (rhs, &rt, value)];
                let new = self.arith(frame, op, operands, int_ty(&plain), tree.expr(value).span)?;
                // No expression of the program gives `x op e`: an observer is shown it with
                // the span of the whole assignment, before it is narrowed into the place.
                if let Some(seen) = self.seen.as_mut() {
                    show(&new, whole, &mut **seen);
                }
                new
            }
        };
        let new = self.narrow(frame, new, declared, &plain, tree.expr(value).span)?;
        self.write(frame, &path, new)
    }

    /// The root binding of a place and its steps, its indexes evaluated left to right.
    fn place(&mut self, frame: &mut Frame<'m>, e: ExprId) -> Flow<Place> {
        let f = frame.f;
        match &f.syntax.tree.expr(e).kind {
            ExprKind::Index { base, index } => {
                let (root, mut steps) = self.place(frame, *base)?;
                let i = self.int(frame, *index, None)?;
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
        Ok(self.eval(frame, e, None)? == Value::Bool(true))
    }

    /// An order comparison of two integers, the left operand evaluated first.
    fn compare(
        &mut self,
        frame: &mut Frame<'m>,
        (lhs, rhs): (ExprId, ExprId),
        holds: fn(&Int, &Int) -> bool,
    ) -> Flow<Value> {
        let l = self.int(frame, lhs, None)?;
        let r = self.int(frame, rhs, None)?;
        Ok(Value::Bool(holds(&l, &r)))
    }

    fn int(&mut self, frame: &mut Frame<'m>, e: ExprId, ctx: Option<IntTy>) -> Flow<Int> {
        match self.eval(frame, e, ctx)? {
            Value::Int(i) => Ok(i),
            Value::Bool(_)
            | Value::Array(_)
            | Value::Struct(..)
            | Value::Enum(..)
            | Value::Unit => broken("an integer operand that is not an integer"),
        }
    }

    /// The value of `e`, whose integers an observer of the run is shown. `ctx` is the integer
    /// type of the place the value is narrowed into, which literals alone take
    /// (`core-calculus.md`, item 5).
    fn eval(&mut self, frame: &mut Frame<'m>, e: ExprId, ctx: Option<IntTy>) -> Flow<Value> {
        let v = self.value(frame, e, ctx)?;
        if let Some(seen) = self.seen.as_mut() {
            show(&v, frame.f.syntax.tree.expr(e).span, &mut **seen);
        }
        Ok(v)
    }

    /// [`Machine::eval`]'s value.
    fn value(&mut self, frame: &mut Frame<'m>, e: ExprId, ctx: Option<IntTy>) -> Flow<Value> {
        let f = frame.f;
        let tree = &f.syntax.tree;
        let span = tree.expr(e).span;
        Ok(match &tree.expr(e).kind {
            ExprKind::Int { value, .. } => Value::Int(Int::from_u128(*value)),
            ExprKind::Bool(b) => Value::Bool(*b),
            ExprKind::Name(_) => {
                match f.resolution.names[e.0 as usize]
                    .and_then(|id| frame.slots[id.0 as usize].clone())
                {
                    Some(v) => v,
                    None => return broken("a name without a value"),
                }
            }
            ExprKind::Todo(reason) => return trap(TrapKind::Todo(reason.clone()), span),
            ExprKind::Unreachable(reason) => {
                return trap(TrapKind::Unreachable(reason.clone()), span)
            }
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Not => Value::Bool(!self.cond(frame, *operand)?),
                UnaryOp::Neg => {
                    let x = self.int(frame, *operand, ctx)?.negate();
                    // On an `i128` or `u128` operand, checked at the operator (`core-calculus.md`,
                    // item 4).
                    match int_ty(&f.typing.exprs[operand.0 as usize]).filter(|t| t.bits == 128) {
                        Some(t) => Value::Int(fit(x, t, span)?),
                        None => Value::Int(x),
                    }
                }
                UnaryOp::BitNot => {
                    let ty = &f.typing.exprs[operand.0 as usize];
                    let t = int_ty(ty).or(ctx).unwrap_or(I64);
                    let x = self.int(frame, *operand, ctx)?;
                    let x = fit(x, t, tree.expr(*operand).span)?;
                    Value::Int(Int::from_bits(!x.to_bits(t), t))
                }
            },
            ExprKind::Binary { op, lhs, rhs } => match op {
                BinaryOp::And => Value::Bool(self.cond(frame, *lhs)? && self.cond(frame, *rhs)?),
                BinaryOp::Or => Value::Bool(self.cond(frame, *lhs)? || self.cond(frame, *rhs)?),
                BinaryOp::Eq | BinaryOp::Ne => {
                    let l = self.eval(frame, *lhs, None)?;
                    let r = self.eval(frame, *rhs, None)?;
                    Value::Bool((l == r) == (*op == BinaryOp::Eq))
                }
                BinaryOp::Lt => self.compare(frame, (*lhs, *rhs), |l, r| l < r)?,
                BinaryOp::Le => self.compare(frame, (*lhs, *rhs), |l, r| l <= r)?,
                BinaryOp::Gt => self.compare(frame, (*lhs, *rhs), |l, r| l > r)?,
                BinaryOp::Ge => self.compare(frame, (*lhs, *rhs), |l, r| l >= r)?,
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
                    let shift = matches!(op, BinaryOp::Shl | BinaryOp::Shr);
                    let l = self.eval(frame, *lhs, ctx)?;
                    let r = self.eval(frame, *rhs, if shift { None } else { ctx })?;
                    let lt = &f.typing.exprs[lhs.0 as usize];
                    let rt = &f.typing.exprs[rhs.0 as usize];
                    self.arith(frame, *op, [(l, lt, *lhs), (r, rt, *rhs)], ctx, span)?
                }
            },
            ExprKind::Index { base, index } => {
                let b = self.eval(frame, *base, None)?;
                let i = self.int(frame, *index, None)?;
                element(&b, i, span)?.clone()
            }
            // The base is a value first, as an index's is, so that a field read through an
            // index reads the value the base had when it was evaluated, its indexes evaluated,
            // and each checked, from left to right (`core-calculus.md`, items 10 and 12).
            ExprKind::Field { base, name } => match self.eval(frame, *base, None)? {
                Value::Array(items) => Value::Int(Int::from_u128(items.len() as u128)),
                Value::Struct(_, fields) => match fields.into_iter().find(|(n, _)| n == name) {
                    Some((_, v)) => v,
                    None => return broken("a field its struct value does not hold"),
                },
                Value::Int(_) | Value::Bool(_) | Value::Enum(..) | Value::Unit => {
                    return broken("a field read of a value that is neither an array nor a struct")
                }
            },
            // Each value is narrowed into its field's type and refinement where it lands, in the
            // order written, which is the declaration's (`core-calculus.md`, item 13).
            ExprKind::Construct { name, fields } => {
                let def = self.module.structs.get(name);
                let mut values = Vec::new();
                for fi in fields {
                    let field = def.and_then(|d| d.field(&fi.name)).map(|(_, f)| f);
                    let fty = field.map_or(Ty::Any, |f| f.ty.clone());
                    let declared = def.zip(field).map(|(d, f)| (d.tree(), f.decl));
                    let v = self.eval(frame, fi.value, int_ty(&fty))?;
                    let at = tree.expr(fi.value).span;
                    let v = self.narrow(frame, v, declared, &fty, at)?;
                    values.push((fi.name.clone(), v));
                }
                Value::Struct(name.clone(), values)
            }
            // A variant's payload is built as a struct is (`core-calculus.md`, item 14).
            ExprKind::Variant {
                enum_name,
                name,
                payload,
                ..
            } => {
                let def = self.module.structs.get_enum(enum_name);
                let variant = def.and_then(|d| d.variant(name)).map(|(_, v)| v);
                let mut values = Vec::new();
                let fields: &[mente_syntax::tree::FieldInit] = match payload {
                    Payload::Named(fields) => fields,
                    Payload::None => &[],
                    // A variant built by position is `E0226`.
                    Payload::Positional(_) => return broken("a variant built by position"),
                };
                for fi in fields {
                    let field = variant.and_then(|v| v.field(&fi.name)).map(|(_, f)| f);
                    let fty = field.map_or(Ty::Any, |f| f.ty.clone());
                    let declared = def.zip(field).map(|(d, f)| (d.tree(), f.decl));
                    let v = self.eval(frame, fi.value, int_ty(&fty))?;
                    let at = tree.expr(fi.value).span;
                    let v = self.narrow(frame, v, declared, &fty, at)?;
                    values.push((fi.name.clone(), v));
                }
                Value::Enum(enum_name.clone(), name.clone(), values)
            }
            ExprKind::Match { scrutinee, arms } => {
                let v = self.eval(frame, *scrutinee, None)?;
                // An integer scrutinee is narrowed into its type, which the arms cover
                // ([`match-scrutinee-narrowing`]); literals alone are an `i64`.
                let read = match &f.typing.exprs[scrutinee.0 as usize] {
                    Ty::Int(IntType::Literal) => Ty::Int(IntType::One(I64)),
                    t @ (Ty::Int(_)
                    | Ty::Bool
                    | Ty::Array(..)
                    | Ty::Struct(_)
                    | Ty::Enum(_)
                    | Ty::Unit
                    | Ty::Any) => t.clone(),
                };
                let v = self.narrow_plain(v, &read, tree.expr(*scrutinee).span)?;
                for (k, arm) in arms.iter().enumerate() {
                    if !self.matches(frame, e, k, &arm.pattern.kind, &v)? {
                        continue;
                    }
                    return match arm.body {
                        ArmBody::Expr(x) => self.eval(frame, x, ctx),
                        ArmBody::Block(b) => self.block(frame, b),
                    };
                }
                // A `match` the checker passes is exhaustive.
                return trap(
                    TrapKind::Unreachable("no arm of this `match` matches".into()),
                    span,
                );
            }
            ExprKind::Call { args, .. } => {
                let Some(index) = f.resolution.calls[e.0 as usize] else {
                    return broken("a call that was never resolved");
                };
                let callee = self.function(index)?;
                let mut values = Vec::new();
                let mut spans = Vec::new();
                let mut places = Vec::new();
                for (a, p) in args.iter().zip(&callee.syntax.params) {
                    if a.is_mut {
                        let path = self.place(frame, a.value)?;
                        values.push(self.read(frame, &path)?);
                        places.push(Some(path));
                    } else {
                        let pt = Ty::of(&callee.syntax.tree, p.ty);
                        values.push(self.eval(frame, a.value, int_ty(&pt))?);
                        places.push(None);
                    }
                    spans.push(tree.expr(a.value).span);
                }
                let caller = Some((frame.index, frame.measure.clone()));
                let (result, finals) = self.call(index, values, &spans, span, caller)?;
                for (place, v) in places.into_iter().zip(finals) {
                    if let Some(path) = place {
                        // The value a `mut` parameter hands back enters the place's refinement.
                        let declared = self.place_type(frame, &path)?;
                        self.refined(frame, &v, declared, span)?;
                        self.write(frame, &path, v)?;
                    }
                }
                result
            }
            // Each element is narrowed into the literal's element type, at the literal
            // (`core-calculus.md`, item 4).
            ExprKind::Array(items) => {
                let elem_ty = match &f.typing.exprs[e.0 as usize] {
                    Ty::Array(_, elem) => elem.as_ref(),
                    Ty::Int(_) | Ty::Bool | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit | Ty::Any => {
                        return broken("an array literal whose type is not an array")
                    }
                };
                let elem = int_ty(elem_ty);
                let mut values = Vec::new();
                for &i in items {
                    let v = self.eval(frame, i, elem.or(ctx))?;
                    values.push(self.narrow_plain(v, elem_ty, span)?);
                }
                Value::Array(values)
            }
            ExprKind::Repeat { value, count } => {
                if *count > self.limits.max_array {
                    return Err(Exit::Stop(Stop::TooLarge(*count)));
                }
                let v = self.eval(frame, *value, ctx)?;
                let v = match &f.typing.exprs[e.0 as usize] {
                    Ty::Array(_, elem_ty) => self.narrow_plain(v, elem_ty, span)?,
                    Ty::Int(_) | Ty::Bool | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit | Ty::Any => {
                        return broken("an array literal whose type is not an array")
                    }
                };
                Value::Array(vec![v; *count as usize])
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                if self.cond(frame, *cond)? {
                    self.block(frame, *then)?
                } else {
                    match otherwise {
                        Some(Else::Block(b)) => self.block(frame, *b)?,
                        Some(Else::If(i)) => self.eval(frame, *i, ctx)?,
                        None => Value::Unit,
                    }
                }
            }
            ExprKind::Error => Value::Unit,
        })
    }

    /// Whether pattern `p`, of arm `k` of the `match` at `m`, matches `v`; when it does, its
    /// bindings take the payload's fields, which the arm only reads.
    fn matches(
        &self,
        frame: &mut Frame<'m>,
        m: ExprId,
        k: usize,
        p: &PatternKind,
        v: &Value,
    ) -> Flow<bool> {
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
        [(l, lt, le), (r, rt, re)]: [(Value, &Ty, ExprId); 2],
        ctx: Option<IntTy>,
        span: Span,
    ) -> Flow<Value> {
        let tree = &frame.f.syntax.tree;
        let (Value::Int(a), Value::Int(b)) = (l, r) else {
            return broken("an integer operand that is not an integer");
        };
        // A bit operation works on the operands' bits in one type: either operand's, or the
        // place's.
        let bits = |on: fn(u128, u128) -> u128| -> Flow<Value> {
            let t = int_ty(lt).or(int_ty(rt)).or(ctx).unwrap_or(I64);
            let x = fit(a, t, tree.expr(le).span)?.to_bits(t);
            let y = fit(b, t, tree.expr(re).span)?.to_bits(t);
            Ok(Value::Int(Int::from_bits(on(x, y), t)))
        };
        let result = match op {
            BinaryOp::Shl | BinaryOp::Shr => {
                let t = int_ty(lt).or(ctx).unwrap_or(I64);
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
        let wide = [int_ty(lt), int_ty(rt)]
            .into_iter()
            .flatten()
            .find(|t| t.bits == 128);
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
        &self,
        frame: &Frame<'m>,
        v: Value,
        declared: Declared<'m>,
        plain: &Ty,
        span: Span,
    ) -> Flow<Value> {
        let v = self.narrow_plain(v, plain, span)?;
        self.refined(frame, &v, declared, span)?;
        Ok(v)
    }

    /// Narrows a value into the integer ranges of a static type, elementwise for an array.
    fn narrow_plain(&self, v: Value, plain: &Ty, span: Span) -> Flow<Value> {
        let mismatch = || broken("a value of another shape than its static type");
        match plain {
            // A literal of no suffix is narrowed where it lands, which gives it its type.
            Ty::Int(_) => match v {
                Value::Int(x) => match int_ty(plain) {
                    Some(t) => fit(x, t, span).map(Value::Int),
                    None => Ok(Value::Int(x)),
                },
                Value::Bool(_)
                | Value::Array(_)
                | Value::Struct(..)
                | Value::Enum(..)
                | Value::Unit => mismatch(),
            },
            Ty::Array(_, elem) => match v {
                Value::Array(items) => items
                    .into_iter()
                    .map(|v| self.narrow_plain(v, elem, span))
                    .collect::<Flow<Vec<Value>>>()
                    .map(Value::Array),
                Value::Int(_)
                | Value::Bool(_)
                | Value::Struct(..)
                | Value::Enum(..)
                | Value::Unit => mismatch(),
            },
            // A struct's fields, each into its type, in the order of the declaration: a value
            // built in the program already lies there, one from outside it may not.
            Ty::Struct(s) => match v {
                Value::Struct(name, fields) => {
                    let Some(def) = self.module.structs.get(s) else {
                        return broken("a struct the module does not declare");
                    };
                    let mut out = Vec::new();
                    for ((n, v), d) in fields.into_iter().zip(&def.fields) {
                        out.push((n, self.narrow_plain(v, &d.ty, span)?));
                    }
                    Ok(Value::Struct(name, out))
                }
                Value::Int(_)
                | Value::Bool(_)
                | Value::Array(_)
                | Value::Enum(..)
                | Value::Unit => mismatch(),
            },
            // A variant's payload, each field into its type, likewise.
            Ty::Enum(e) => match v {
                Value::Enum(name, variant, fields) => {
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

    /// Checks a value against the refinements of a declared type, elementwise for an array and
    /// fieldwise for a struct, whose fields have the refinements its declaration gives them.
    fn refined(
        &self,
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

/// Shows `seen` each integer of `v`, the value of the expression at `span`: itself, or those
/// of an array's elements, through the arrays nested in it. A struct's and a variant's fields
/// were shown where they were built.
fn show(v: &Value, span: Span, seen: &mut (dyn FnMut(Span, Int) + Send)) {
    match v {
        Value::Int(x) => seen(span, *x),
        Value::Array(items) => items.iter().for_each(|i| show(i, span, seen)),
        Value::Bool(_) | Value::Struct(..) | Value::Enum(..) | Value::Unit => {}
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
