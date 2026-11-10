//! The interpreter proper (`spec/core-calculus.md`, items 4, 6, 7, 9 and 10).
//!
//! A [`Frame`] holds one call's bindings in slots indexed like the function's resolution, which
//! names each binding once (no shadowing), so a block needs no scope of its own. Control leaves
//! a statement by an [`Exit`]: a `return`, a `break`, a `continue`, or a stop.

use mente_sema::names::BindingId;
use mente_sema::types::{Int as IntType, IntTy, Ty, I64};
use mente_sema::{CheckedFn, CheckedModule};
use mente_syntax::tree::{
    AssignOp, BinaryOp, BlockId, Body, Bound, ContractKind, Else, ExprId, ExprKind, Refinement,
    StmtId, StmtKind, TypeId, TypeKind, UnaryOp,
};
use mente_syntax::Span;

use crate::{Int, Limits, Stop, Trap, TrapKind, Value};

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

/// The integer type a static type names: its one type, or the type that holds a mix.
fn int_ty(ty: &Ty) -> Option<IntTy> {
    match ty {
        Ty::Int(IntType::One(t)) | Ty::Int(IntType::Mixed(Some(t))) => Some(*t),
        _ => None,
    }
}

/// Whether `v` has the shape of a value of type `ty`: an integer, a `bool`, or an array of its
/// length whose elements have its element's shape. Its range is not checked here.
fn shaped(v: &Value, ty: &Ty) -> bool {
    match (v, ty) {
        (Value::Int(_), Ty::Int(_)) | (Value::Bool(_), Ty::Bool) => true,
        (Value::Array(items), Ty::Array(n, elem)) => {
            items.len() as u128 == *n && items.iter().all(|i| shaped(i, elem))
        }
        _ => false,
    }
}

/// One call: its function and the values of its bindings.
struct Frame<'m> {
    index: usize,
    f: &'m CheckedFn,
    slots: Vec<Option<Value>>,
    /// The function's `decreases` measure at this call, for the recursive calls it makes.
    measure: Option<Vec<Int>>,
}

/// A place a `mut` argument or an assignment names: its root binding and the indexes into it,
/// each with the span of its indexing.
type Place = (BindingId, Vec<(Int, Span)>);

pub(crate) struct Machine<'m> {
    module: &'m CheckedModule,
    limits: Limits,
    steps: u64,
    depth: usize,
}

impl<'m> Machine<'m> {
    pub(crate) fn new(module: &'m CheckedModule, limits: Limits) -> Machine<'m> {
        Machine {
            module,
            limits,
            steps: 0,
            depth: 0,
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
            if !shaped(v, &Ty::of(tree, p.ty)) {
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
            self.refined(frame, &v, Some(p.ty), spans[i])?;
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
        let (result, ret_span) = match f.syntax.body {
            Some(Body::Expr(e)) => {
                let v = self.eval(frame, e, int_ty(&result_ty))?;
                let span = tree.expr(e).span;
                (
                    self.narrow(frame, v, f.syntax.result, &result_ty, span)?,
                    span,
                )
            }
            Some(Body::Block(b)) => match self.block(frame, b) {
                Ok(v) => {
                    let span = tree
                        .block(b)
                        .stmts
                        .last()
                        .map_or(tree.block(b).span, |&s| tree.stmt(s).span);
                    (
                        self.narrow(frame, v, f.syntax.result, &result_ty, span)?,
                        span,
                    )
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
                    return Ok(Value::Unit);
                };
                let plain = f.typing.bindings[id.0 as usize].clone();
                let v = self.eval(frame, *init, int_ty(&plain))?;
                let v = self.narrow(frame, v, *ty, &plain, tree.expr(*init).span)?;
                frame.slots[id.0 as usize] = Some(v);
            }
            StmtKind::Assign { place, op, value } => self.assign(frame, *place, *op, *value)?,
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
                let v = self.narrow(frame, v, f.syntax.result, &result_ty, span)?;
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
    ) -> Flow<()> {
        let f = frame.f;
        let tree = &f.syntax.tree;
        let path = self.place(frame, place)?;
        let plain = f.typing.exprs[place.0 as usize].clone();
        let declared = self.place_type(frame, &path);
        let new = match op {
            AssignOp::Set => self.eval(frame, value, int_ty(&plain))?,
            AssignOp::Compound(op) => {
                let current = self.read(frame, &path)?;
                let rhs = self.eval(frame, value, int_ty(&plain))?;
                let rt = f.typing.exprs[value.0 as usize].clone();
                let operands = [(current, &plain, place), (rhs, &rt, value)];
                self.arith(frame, op, operands, int_ty(&plain), tree.expr(value).span)?
            }
        };
        let new = self.narrow(frame, new, declared, &plain, tree.expr(value).span)?;
        self.write(frame, &path, new)
    }

    /// The root binding of a place and its indexes, evaluated left to right.
    fn place(&mut self, frame: &mut Frame<'m>, e: ExprId) -> Flow<Place> {
        let f = frame.f;
        match &f.syntax.tree.expr(e).kind {
            ExprKind::Index { base, index } => {
                let (root, mut indexes) = self.place(frame, *base)?;
                let i = self.int(frame, *index, None)?;
                indexes.push((i, f.syntax.tree.expr(e).span));
                Ok((root, indexes))
            }
            _ => Ok((
                f.resolution.names[e.0 as usize].unwrap_or(BindingId(0)),
                Vec::new(),
            )),
        }
    }

    /// The declared type of a place, when its root binding states one.
    fn place_type(&self, frame: &Frame<'m>, (root, indexes): &Place) -> Option<TypeId> {
        let tree = &frame.f.syntax.tree;
        let mut ty = frame.f.resolution.bindings.get(root.0 as usize)?.ty?;
        for _ in indexes {
            match &tree.ty(ty).kind {
                TypeKind::Array { elem, .. } => ty = *elem,
                _ => return None,
            }
        }
        Some(ty)
    }

    fn read(&self, frame: &Frame<'m>, (root, indexes): &Place) -> Flow<Value> {
        let mut v = frame.slots[root.0 as usize]
            .as_ref()
            .unwrap_or(&Value::Unit);
        for (i, span) in indexes {
            v = element(v, *i, *span)?;
        }
        Ok(v.clone())
    }

    fn write(&self, frame: &mut Frame<'m>, (root, indexes): &Place, new: Value) -> Flow<()> {
        let mut v = frame.slots[root.0 as usize].get_or_insert(Value::Unit);
        for (i, span) in indexes {
            let Value::Array(items) = v else {
                return Ok(());
            };
            let len = items.len();
            match i.to_u128().and_then(|i| usize::try_from(i).ok()) {
                Some(k) if k < len => v = &mut items[k],
                _ => {
                    return trap(TrapKind::IndexOutOfBounds { index: *i, len }, *span);
                }
            }
        }
        *v = new;
        Ok(())
    }

    fn cond(&mut self, frame: &mut Frame<'m>, e: ExprId) -> Flow<bool> {
        Ok(self.eval(frame, e, None)? == Value::Bool(true))
    }

    fn int(&mut self, frame: &mut Frame<'m>, e: ExprId, ctx: Option<IntTy>) -> Flow<Int> {
        match self.eval(frame, e, ctx)? {
            Value::Int(i) => Ok(i),
            _ => Ok(Int::ZERO),
        }
    }

    /// The value of `e`. `ctx` is the integer type of the place the value is narrowed into,
    /// which literals alone take (`core-calculus.md`, item 5).
    fn eval(&mut self, frame: &mut Frame<'m>, e: ExprId, ctx: Option<IntTy>) -> Flow<Value> {
        let f = frame.f;
        let tree = &f.syntax.tree;
        let span = tree.expr(e).span;
        Ok(match &tree.expr(e).kind {
            ExprKind::Int { value, .. } => Value::Int(Int::from_u128(*value)),
            ExprKind::Bool(b) => Value::Bool(*b),
            ExprKind::Name(_) => f.resolution.names[e.0 as usize]
                .and_then(|id| frame.slots[id.0 as usize].clone())
                .unwrap_or(Value::Unit),
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
                BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                    let l = self.int(frame, *lhs, None)?;
                    let r = self.int(frame, *rhs, None)?;
                    Value::Bool(match op {
                        BinaryOp::Lt => l < r,
                        BinaryOp::Le => l <= r,
                        BinaryOp::Gt => l > r,
                        _ => l >= r,
                    })
                }
                _ => {
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
            ExprKind::Field { base, .. } => match self.eval(frame, *base, None)? {
                Value::Array(items) => Value::Int(Int::from_u128(items.len() as u128)),
                _ => Value::Unit,
            },
            ExprKind::Call { args, .. } => {
                let Some(index) = f.resolution.calls[e.0 as usize] else {
                    return Ok(Value::Unit);
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
                        let declared = self.place_type(frame, &path);
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
                    Ty::Array(_, elem) => Some(elem.as_ref()),
                    _ => None,
                };
                let elem = elem_ty.and_then(int_ty);
                let mut values = Vec::new();
                for &i in items {
                    let v = self.eval(frame, i, elem.or(ctx))?;
                    values.push(match elem_ty {
                        Some(t) => self.narrow_plain(v, t, span)?,
                        None => v,
                    });
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
                    _ => v,
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
            return Ok(Value::Unit);
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
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                let t = int_ty(lt).or(int_ty(rt)).or(ctx).unwrap_or(I64);
                let x = fit(a, t, tree.expr(le).span)?.to_bits(t);
                let y = fit(b, t, tree.expr(re).span)?.to_bits(t);
                let bits = match op {
                    BinaryOp::BitAnd => x & y,
                    BinaryOp::BitOr => x | y,
                    _ => x ^ y,
                };
                return Ok(Value::Int(Int::from_bits(bits, t)));
            }
            BinaryOp::Div | BinaryOp::Rem if b == Int::ZERO => {
                return trap(TrapKind::DivisionByZero, span);
            }
            BinaryOp::Add => a.checked_add(b),
            BinaryOp::Sub => a.checked_sub(b),
            BinaryOp::Mul => a.checked_mul(b),
            BinaryOp::Div => a.checked_div(b),
            _ => a.checked_rem(b),
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
        declared: Option<TypeId>,
        plain: &Ty,
        span: Span,
    ) -> Flow<Value> {
        let v = self.narrow_plain(v, plain, span)?;
        self.refined(frame, &v, declared, span)?;
        Ok(v)
    }

    /// Narrows a value into the integer ranges of a static type, elementwise for an array.
    fn narrow_plain(&self, v: Value, plain: &Ty, span: Span) -> Flow<Value> {
        match (v, plain) {
            (Value::Int(x), ty) => match int_ty(ty) {
                Some(t) => fit(x, t, span).map(Value::Int),
                None => Ok(Value::Int(x)),
            },
            (Value::Array(items), Ty::Array(_, elem)) => items
                .into_iter()
                .map(|v| self.narrow_plain(v, elem, span))
                .collect::<Flow<Vec<Value>>>()
                .map(Value::Array),
            (v, _) => Ok(v),
        }
    }

    /// Checks a value against the refinements of a declared type, elementwise for an array.
    fn refined(
        &self,
        frame: &Frame<'m>,
        v: &Value,
        declared: Option<TypeId>,
        span: Span,
    ) -> Flow<()> {
        let Some(id) = declared else {
            return Ok(());
        };
        let tree = &frame.f.syntax.tree;
        match (v, &tree.ty(id).kind) {
            (
                Value::Int(x),
                TypeKind::Int {
                    refinement: Some(r),
                    ..
                },
            ) => self.in_range(frame, *x, r, span),
            (Value::Array(items), TypeKind::Array { elem, .. }) => {
                for item in items {
                    self.refined(frame, item, Some(*elem), span)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn in_range(&self, frame: &Frame<'m>, x: Int, r: &Refinement, span: Span) -> Flow<()> {
        let lo = self.bound(frame, &r.lo);
        let hi = self.bound(frame, &r.hi);
        let below = if r.inclusive { x <= hi } else { x < hi };
        if lo <= x && below {
            return Ok(());
        }
        let range = format!("{}..{}{}", lo, if r.inclusive { "=" } else { "" }, hi);
        trap(TrapKind::Refinement { value: x, range }, span)
    }

    /// The value of a refinement bound in this frame.
    fn bound(&self, frame: &Frame<'m>, b: &Bound) -> Int {
        let slot = |name: &str| {
            frame
                .f
                .resolution
                .bindings
                .iter()
                .position(|binding| binding.name == name)
                .and_then(|i| frame.slots[i].as_ref())
        };
        match b {
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
                _ => Int::ZERO,
            },
            Bound::Len(n) => match slot(n) {
                Some(Value::Array(items)) => Int::from_u128(items.len() as u128),
                _ => Int::ZERO,
            },
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
    let Value::Array(items) = v else {
        return Ok(v);
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
