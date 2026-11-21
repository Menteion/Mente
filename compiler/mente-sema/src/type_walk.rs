//! The one walk over the structure of a module's declared types.
//!
//! A struct's or an enum's fields hold other declarations, through arrays, so a module's
//! declarations form a graph. A walk that follows every field from a declaration visits a
//! declaration once for each path that leads to it: a DAG of structs, each holding two of the one
//! before it, forty deep, has 2^40 such paths. A walk that recurses a level of the host's stack
//! for each declaration overflows a thread's stack on a chain of a few thousand. The review of
//! pull request 184 found both in the lowering, the fuzzer, the reference interpreter, the
//! generated programs' arguments and Tier 1, and one of them filled a laptop's memory
//! (2026-11-22).
//!
//! [`TypeWalk`] visits each declaration and each field once, with a stack of its own. It orders
//! the declarations, each after those its fields hold, and finds the cycles among them, which the
//! checker refuses (`E0225`). [`TypeWalk::fold`] works out a fact of every declaration once, in
//! that order, from the facts of its fields' types, and charges each declaration and each node of
//! each field's type to a budget: the declarations and the nodes of their fields' types, the most
//! one pass over the module takes. Every phase that walks the structure of types folds through
//! it, so that its work is the module's size, never the number of paths through it, and
//! [`Measure`], what a value of a type holds and how deep it nests, is defined here once for
//! every phase that reads it.

use crate::structs::{Decl, FieldDef, Structs};
use crate::types::Ty;
use mente_syntax::tree::Tree;

/// Where one declaration holds another: a struct's field, or a field of an enum's variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Via {
    pub variant: Option<usize>,
    pub field: usize,
}

/// The declarations of a module as a graph, in dependency order, with the cycles among them.
pub struct TypeWalk<'s> {
    structs: &'s Structs,
    /// Every declaration, by its slot: the structs first, then the enums.
    decls: Vec<Decl>,
    /// The slots in dependency order: each after the declarations it holds, but for one that a
    /// cycle leads back to.
    order: Vec<usize>,
    /// Each cycle the walk closed, as the declarations on it, each with where it holds the next,
    /// from the one the walk entered first.
    cycles: Vec<Vec<(Decl, Via)>>,
    /// The most steps a fold takes: a step for each declaration and for each node of each of its
    /// fields' types.
    budget: usize,
    /// The steps the ordering took: one for each declaration and each place one holds another.
    steps: usize,
}

/// A step's state in the ordering's depth-first search.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    New,
    Open,
    Done,
}

impl<'s> TypeWalk<'s> {
    /// The graph of `structs`, ordered in one depth-first search over the declarations, in the
    /// order of their declarations and fields, with a stack of its own. A field that leads back
    /// to a declaration still open on the search's path closes a cycle, recorded once, so that two
    /// cycles through one declaration are two.
    pub fn new(structs: &'s Structs) -> TypeWalk<'s> {
        let s = structs.defs.len();
        let mut decls: Vec<Decl> = (0..s).map(Decl::Struct).collect();
        decls.extend((0..structs.enums.len()).map(Decl::Enum));
        let slot_of = |name: &str| {
            structs
                .position(name)
                .or_else(|| structs.enum_position(name).map(|e| s + e))
        };
        let mut budget = decls.len();
        let mut held: Vec<Vec<(Via, usize)>> = Vec::with_capacity(decls.len());
        for &d in &decls {
            let mut edges = Vec::new();
            for (variant, fields) in fields_by_variant(structs, d) {
                for (field, f) in fields.iter().enumerate() {
                    // A node of the field's type for each array around it and for what it holds.
                    let mut t = &f.ty;
                    loop {
                        budget += 1;
                        match t {
                            Ty::Array(_, elem) => t = elem,
                            Ty::Struct(name) | Ty::Enum(name) => {
                                if let Some(j) = slot_of(name) {
                                    edges.push((Via { variant, field }, j));
                                }
                                break;
                            }
                            Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Any => break,
                        }
                    }
                }
            }
            held.push(edges);
        }
        let mut state = vec![Visit::New; decls.len()];
        let mut order = Vec::with_capacity(decls.len());
        let mut cycles = Vec::new();
        let mut steps = 0;
        for root in 0..decls.len() {
            if state[root] != Visit::New {
                continue;
            }
            state[root] = Visit::Open;
            steps += 1;
            // The search's path: each declaration with the index of the next place it holds
            // another, so that the one it is following now is the index before.
            let mut path: Vec<(usize, usize)> = vec![(root, 0)];
            while let Some(&(i, k)) = path.last() {
                let Some(&(_, j)) = held[i].get(k) else {
                    state[i] = Visit::Done;
                    order.push(i);
                    path.pop();
                    continue;
                };
                steps += 1;
                let top = path.len() - 1;
                path[top].1 = k + 1;
                match state[j] {
                    Visit::Open => {
                        let start = path.iter().position(|(n, _)| *n == j).unwrap_or(0);
                        let cycle = path[start..]
                            .iter()
                            .map(|&(n, next)| (decls[n], held[n][next - 1].0))
                            .collect();
                        cycles.push(cycle);
                    }
                    Visit::New => {
                        state[j] = Visit::Open;
                        steps += 1;
                        path.push((j, 0));
                    }
                    Visit::Done => {}
                }
            }
        }
        TypeWalk {
            structs,
            decls,
            order,
            cycles,
            budget,
            steps,
        }
    }

    /// The declarations in dependency order: each after those its fields hold, but for one that
    /// a cycle leads back to.
    pub fn order(&self) -> Vec<Decl> {
        self.order.iter().map(|&k| self.decls[k]).collect()
    }

    /// Each cycle among the declarations, which the checker refuses (`E0225`): the declarations
    /// on it, each with where it holds the next, from the one the search entered first.
    pub fn cycles(&self) -> &[Vec<(Decl, Via)>] {
        &self.cycles
    }

    /// The steps the ordering took: one for each declaration and each place one holds another.
    pub fn steps(&self) -> usize {
        self.steps
    }

    /// The most steps a fold takes: one for each declaration and for each node of each of its
    /// fields' types.
    pub fn budget(&self) -> usize {
        self.budget
    }

    /// A fact of every declaration, worked out once, in dependency order, from the facts of its
    /// fields' types, as `fold` defines them. Each declaration and each node of each field's type
    /// is a step charged to the budget; the facts the budget leaves no steps for, which a fold
    /// over the module never meets, are none.
    pub fn fold<F: Fold>(&self, fold: &mut F) -> Facts<F::Fact> {
        self.fold_within(fold, self.budget)
    }

    /// [`Self::fold`], within `budget` steps: the declaration whose step the budget does not
    /// hold, and every one after it, has no fact.
    fn fold_within<F: Fold>(&self, fold: &mut F, budget: usize) -> Facts<F::Fact> {
        let mut facts = Facts {
            facts: vec![None; self.decls.len()],
            structs: self.structs.defs.len(),
            steps: 0,
        };
        let mut spent = 0;
        for &k in &self.order {
            let d = self.decls[k];
            let tree = match d {
                Decl::Struct(i) => self.structs.defs[i].tree(),
                Decl::Enum(i) => self.structs.enums[i].tree(),
            };
            let mut variants = Vec::new();
            for (_, fields) in fields_by_variant(self.structs, d) {
                let mut taken = Vec::with_capacity(fields.len());
                for def in fields {
                    let fact = self.charged(&facts, fold, &def.ty, budget, &mut spent);
                    taken.push(Field { def, tree, fact });
                }
                variants.push(taken);
            }
            if spent >= budget {
                break;
            }
            spent += 1;
            facts.facts[k] = fold.decl(d, &variants);
        }
        facts.steps = spent;
        facts
    }

    /// The fact of type `t`, from the facts of the declarations it names: an array's from its
    /// element's, and a declaration's as `facts` holds it. Each node of `t` is a step.
    pub fn fact_of<F: Fold>(
        &self,
        facts: &Facts<F::Fact>,
        fold: &mut F,
        t: &Ty,
    ) -> Option<F::Fact> {
        self.charged(facts, fold, t, usize::MAX, &mut 0)
    }

    /// [`Self::fact_of`], adding each node of `t` to `spent`; none once `spent` reaches
    /// `budget`.
    fn charged<F: Fold>(
        &self,
        facts: &Facts<F::Fact>,
        fold: &mut F,
        t: &Ty,
        budget: usize,
        spent: &mut usize,
    ) -> Option<F::Fact> {
        // The lengths of the arrays around the innermost type, outermost first.
        let mut lengths = Vec::new();
        let mut at = t;
        loop {
            if *spent >= budget {
                return None;
            }
            *spent += 1;
            match at {
                Ty::Array(n, elem) => {
                    lengths.push(*n);
                    at = elem;
                }
                Ty::Int(_) | Ty::Bool | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit | Ty::Any => break,
            }
        }
        let s = self.structs.defs.len();
        let mut fact = match at {
            Ty::Struct(name) | Ty::Enum(name) => self
                .structs
                .position(name)
                .or_else(|| self.structs.enum_position(name).map(|e| s + e))
                .and_then(|k| facts.facts[k].clone()),
            Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Any | Ty::Array(..) => fold.leaf(at),
        };
        for &n in lengths.iter().rev() {
            fact = fold.array(n, fact?);
        }
        fact
    }

    /// What a value of each declaration holds, and how deep it nests ([`Measure`]).
    pub fn measures(&self) -> Facts<Measure> {
        self.fold(&mut Measuring)
    }

    /// What a value of type `t` holds, from `measures`, the measures of the declarations.
    pub fn measure_of(&self, measures: &Facts<Measure>, t: &Ty) -> Option<Measure> {
        self.fact_of(measures, &mut Measuring, t)
    }
}

/// The fields of declaration `d`, each group with its variant: a struct's one group, and each
/// variant's of an enum.
fn fields_by_variant(structs: &Structs, d: Decl) -> Vec<(Option<usize>, &[FieldDef])> {
    match d {
        Decl::Struct(i) => vec![(None, structs.defs[i].fields.as_slice())],
        Decl::Enum(i) => structs.enums[i]
            .variants
            .iter()
            .enumerate()
            .map(|(v, var)| (Some(v), var.fields.as_slice()))
            .collect(),
    }
}

/// How a fact of a type is worked out from its parts, for [`TypeWalk::fold`].
pub trait Fold {
    type Fact: Clone;
    /// The fact of a type that names no declaration and holds no element: an integer, a `bool`,
    /// `()`, or a type the checker could not give.
    fn leaf(&mut self, t: &Ty) -> Option<Self::Fact>;
    /// The fact of an array of `len` elements, from its element's.
    fn array(&mut self, len: u128, elem: Self::Fact) -> Option<Self::Fact>;
    /// The fact of declaration `d`, from its fields', grouped by variant: a struct's fields are
    /// one group. A field whose type has no fact, as one that a cycle leads back through, has
    /// none.
    fn decl(&mut self, d: Decl, variants: &[Vec<Field<'_, Self::Fact>>]) -> Option<Self::Fact>;
}

/// A field as a fold sees it: its declaration, the tree it is declared in, and its type's fact.
pub struct Field<'a, T> {
    pub def: &'a FieldDef,
    pub tree: &'a Tree,
    pub fact: Option<T>,
}

/// A fact of each declaration of a module, from [`TypeWalk::fold`].
#[derive(Clone, Debug)]
pub struct Facts<T> {
    facts: Vec<Option<T>>,
    structs: usize,
    /// The steps the fold took, which a test holds to the module's declarations and the nodes
    /// of their fields' types.
    pub steps: usize,
}

impl<T> Facts<T> {
    /// The fact of declaration `d`, none for one that has none.
    pub fn of(&self, d: Decl) -> Option<&T> {
        let k = match d {
            Decl::Struct(i) => i,
            Decl::Enum(i) => self.structs + i,
        };
        self.facts.get(k)?.as_ref()
    }
}

/// What a value of a type holds, and how deep it nests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Measure {
    /// The values it is built of, itself, each element and each field counted one each, an
    /// enum's those of its variant with the most, its tag counted: `[4294967295][0]u8` holds
    /// 4,294,967,296, though none is a scalar. Saturating.
    pub values: u128,
    /// The nodes of its type as a tree that holds each array's element once, however many
    /// elements it has, and the fields of every variant of an enum: what a description of its
    /// values takes. Saturating.
    pub nodes: u128,
    /// The levels it nests: one for itself, and its deepest element's or field's.
    pub levels: u32,
}

impl Measure {
    /// A scalar's.
    pub const ONE: Measure = Measure {
        values: 1,
        nodes: 1,
        levels: 1,
    };

    /// This measure with one more field, of measure `field`.
    fn holding(self, field: Measure) -> Measure {
        Measure {
            values: self.values.saturating_add(field.values),
            nodes: self.nodes.saturating_add(field.nodes),
            levels: self.levels.max(field.levels.saturating_add(1)),
        }
    }
}

/// The fold of [`Measure`].
struct Measuring;

impl Fold for Measuring {
    type Fact = Measure;

    fn leaf(&mut self, _: &Ty) -> Option<Measure> {
        Some(Measure::ONE)
    }

    fn array(&mut self, len: u128, elem: Measure) -> Option<Measure> {
        Some(Measure {
            values: len.saturating_mul(elem.values).saturating_add(1),
            nodes: elem.nodes.saturating_add(1),
            levels: elem.levels.saturating_add(1),
        })
    }

    fn decl(&mut self, d: Decl, variants: &[Vec<Field<'_, Measure>>]) -> Option<Measure> {
        let mut measure = Measure::ONE;
        for fields in variants {
            // A variant's values, its tag counted; every variant's fields are the enum's nodes
            // and levels.
            let mut values = 1u128;
            for f in fields {
                let field = f.fact?;
                values = values.saturating_add(field.values);
                measure = Measure {
                    values: measure.values,
                    ..measure.holding(field)
                };
            }
            measure.values = match d {
                Decl::Struct(_) => values,
                Decl::Enum(_) => measure.values.max(values),
            };
        }
        Some(measure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(src: &str) -> Structs {
        let module = mente_syntax::parse_module(src).unwrap();
        let parsed = mente_syntax::parse_structs(src, &module);
        let enums = mente_syntax::parse_enums(src, &module);
        crate::structs::declare(&module, &parsed, &enums).0
    }

    /// A DAG of structs, each holding two of the one before it, `levels` deep, declared the
    /// outermost first, and an enum holding the outermost in an array.
    fn dag(levels: usize) -> String {
        let mut src = format!("module m\nenum E {{\n  V(s: [2]S{})\n  W\n}}\n", levels - 1);
        for k in (1..levels).rev() {
            src.push_str(&format!(
                "struct S{k} {{\n  l: S{p}\n  r: S{p}\n}}\n",
                p = k - 1
            ));
        }
        src.push_str("struct S0 {\n  a: u8\n}\n");
        src
    }

    /// The walk orders a module's declarations, each after those its fields hold, wherever they
    /// are declared, in a step for each declaration and each place one holds another, and a fold
    /// takes a step for each declaration and each node of each field's type, however many paths
    /// lead to a declaration: a DAG eight deep takes two steps a level, where a walk of every
    /// path took 2^8 (2026-11-22). The fold's budget is those steps, and it spends all of them.
    #[test]
    fn test_the_walk_takes_a_step_for_each_declaration_and_field() {
        let s = declared(&dag(8));
        let walk = TypeWalk::new(&s);
        // Nine declarations; `E`'s array of `S7` holds one, and seven structs hold two each.
        assert_eq!(walk.steps(), 9 + 1 + 2 * 7);
        // The fold's nodes: `[2]S7` two, `a: u8` one, and fourteen fields of a struct.
        assert_eq!(walk.budget(), 9 + 2 + 1 + 14);
        let order = walk.order();
        let at = |d: Decl| order.iter().position(|o| *o == d).unwrap();
        for k in 1..8 {
            let inner = Decl::Struct(s.position(&format!("S{}", k - 1)).unwrap());
            let outer = Decl::Struct(s.position(&format!("S{k}")).unwrap());
            assert!(at(inner) < at(outer), "S{k}");
        }
        assert_eq!(at(Decl::Enum(0)), 8);
        let measures = walk.measures();
        assert_eq!(measures.steps, walk.budget());
        assert!(walk.cycles().is_empty());
        // A step short, the last declaration has no fact, and the one before it has its own.
        let short = walk.fold_within(&mut Measuring, walk.budget() - 1);
        assert_eq!(short.steps, walk.budget() - 1);
        assert_eq!(short.of(order[8]), None);
        assert_eq!(short.of(order[7]), measures.of(order[7]));
        let e = Decl::Enum(0);
        // `S0` holds 2 values, each level one and twice the one below; `E` its variant `V`'s
        // tag and `[2]S7`.
        let s7 = 3 * (1u128 << 7) - 1;
        let m = measures
            .of(Decl::Struct(s.position("S7").unwrap()))
            .unwrap();
        assert_eq!((m.values, m.nodes, m.levels), (s7, s7, 9));
        let m = measures.of(e).unwrap();
        assert_eq!((m.values, m.nodes, m.levels), (1 + 2 * s7 + 1, s7 + 2, 11));
    }

    /// A chain of declarations, each holding the one before it, is ordered and folded on a
    /// stack of the walk's own, however long: one that took a level of the host's stack for each
    /// declaration overflowed a thread's 2 MiB at a few thousand (2026-11-22).
    #[test]
    fn test_a_long_chain_is_walked_on_a_stack_of_its_own() {
        let levels = 20_000;
        let mut src = String::from("module m\nstruct C0 {\n  a: u8\n}\n");
        for k in 1..levels {
            src.push_str(&format!("struct C{k} {{\n  a: C{}\n}}\n", k - 1));
        }
        let s = declared(&src);
        let walk = TypeWalk::new(&s);
        assert_eq!(walk.steps(), 2 * levels - 1);
        let measures = walk.measures();
        let last = Decl::Struct(s.position(&format!("C{}", levels - 1)).unwrap());
        let m = measures.of(last).unwrap();
        assert_eq!(
            (m.values, m.levels),
            (levels as u128 + 1, levels as u32 + 1)
        );
    }

    /// Each cycle the search closes is recorded once, from the declaration it entered first,
    /// each with the field that holds the next; a declaration a cycle leads back to has no
    /// fact, nor has one that holds it.
    #[test]
    fn test_cycles_are_found_once_and_have_no_facts() {
        let s = declared(
            "module m\nstruct A {\n  n: u8\n  b: B\n}\nstruct B {\n  a: [2]A\n}\nstruct C {\n  b: B\n}\nstruct D {\n  x: u8\n}\n",
        );
        let walk = TypeWalk::new(&s);
        let a = Decl::Struct(s.position("A").unwrap());
        let b = Decl::Struct(s.position("B").unwrap());
        assert_eq!(
            walk.cycles(),
            [vec![
                (
                    a,
                    Via {
                        variant: None,
                        field: 1
                    }
                ),
                (
                    b,
                    Via {
                        variant: None,
                        field: 0
                    }
                )
            ]]
        );
        let measures = walk.measures();
        for name in ["A", "B", "C"] {
            assert!(
                measures
                    .of(Decl::Struct(s.position(name).unwrap()))
                    .is_none(),
                "{name}"
            );
        }
        assert_eq!(
            measures.of(Decl::Struct(s.position("D").unwrap())),
            Some(&Measure {
                values: 2,
                nodes: 2,
                levels: 2
            })
        );
    }

    /// A type's measure is worked out from its declarations': an array holds its length times
    /// its element's values and one, and one node more than its element; an enum holds its
    /// variant with the most values, its tag counted, and the nodes of every variant's fields.
    #[test]
    fn test_a_types_measure_counts_arrays_and_enums() {
        let s = declared(
            "module m\nstruct A {\n  xs: [3]u8\n}\nenum E {\n  V(a: A, b: u8)\n  W\n}\nenum F {\n  P(x: u8, y: u8)\n  Q(z: [3]u8)\n}\n",
        );
        let walk = TypeWalk::new(&s);
        let measures = walk.measures();
        let a = measures.of(Decl::Struct(0)).copied().unwrap();
        assert_eq!(
            a,
            Measure {
                values: 5,
                nodes: 3,
                levels: 3
            }
        );
        let e = measures.of(Decl::Enum(0)).copied().unwrap();
        assert_eq!(
            e,
            Measure {
                values: 7,
                nodes: 5,
                levels: 4
            }
        );
        // The variant with the most values, `Q`'s four and its tag, beside `P`'s two and its
        // tag; every variant's nodes.
        let f = measures.of(Decl::Enum(1)).copied().unwrap();
        assert_eq!(
            f,
            Measure {
                values: 5,
                nodes: 5,
                levels: 3
            }
        );
        let of = |t: Ty| walk.measure_of(&measures, &t).unwrap();
        assert_eq!(
            of(Ty::Array(2, Box::new(Ty::Enum("E".into())))),
            Measure {
                values: 15,
                nodes: 6,
                levels: 5
            }
        );
        assert_eq!(
            of(Ty::Array(0, Box::new(Ty::Struct("A".into())))),
            Measure {
                values: 1,
                nodes: 4,
                levels: 4
            }
        );
    }
}
