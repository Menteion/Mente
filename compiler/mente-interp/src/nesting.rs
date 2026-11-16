//! How deep a module nests, which the bound on the reference interpreter's host memory is a
//! formula of ([`host_bound`]).
//!
//! [`host_bound`]: crate::host_bound

use crate::host::host_bound_at;
use mente_sema::types::Ty;
use mente_sema::CheckedModule;
use mente_syntax::body::NESTING_LIMIT;
use mente_syntax::tree::Tree;

/// How deep a module nests, as the parser counts it (`NESTING_LIMIT`): the most levels of
/// expressions, blocks and types in any of its functions and type declarations, and the most
/// levels a value of a declared type nests through its fields.
pub fn nesting(m: &CheckedModule) -> u32 {
    let functions = m.functions.iter().flatten().map(|f| height(&f.syntax.tree));
    functions.chain([types(m)]).max().unwrap_or(0)
}

/// How deep a call of function `index` of `m` nests: the levels of the function, and those of
/// the values it may narrow, as deep as the module's types nest.
pub fn function_nesting(m: &CheckedModule, index: usize) -> u32 {
    let own = m
        .functions
        .get(index)
        .and_then(|f| f.as_ref())
        .map_or(0, |f| height(&f.syntax.tree));
    own.max(types(m))
}

/// The bound on the host memory a run of `m` reserves for its stacks, were its calls all of
/// function `index` (`host_bound_at`): none for a function without a frame, which no run calls.
pub fn host_bound_of_function(m: &CheckedModule, index: usize) -> u128 {
    m.frames
        .get(index)
        .copied()
        .flatten()
        .map_or(0, |f| host_bound_at(f.bytes, function_nesting(m, index)))
}

/// The bound on the host memory a run of `m` reserves for its stacks, in any build: the largest of
/// its functions', since the frames of the calls open at once fill at most the budget, and the
/// function whose calls take the most host stack for each byte of frame bounds them all.
pub fn host_bound_of(m: &CheckedModule) -> u128 {
    (0..m.functions.len())
        .map(|i| host_bound_of_function(m, i))
        .max()
        .unwrap_or(0)
}

/// How deep the module's type declarations nest, and a value of a declared type through its
/// fields.
fn types(m: &CheckedModule) -> u32 {
    let structs = m.structs.defs.iter().map(|s| height(s.tree()));
    let enums = m.structs.enums.iter().map(|e| height(e.tree()));
    let fields = m
        .structs
        .defs
        .iter()
        .flat_map(|s| s.fields.iter().map(|f| &f.ty))
        .chain(m.structs.enums.iter().flat_map(|e| {
            e.variants
                .iter()
                .flat_map(|v| v.fields.iter().map(|f| &f.ty))
        }))
        .map(|t| depth(m, t, 0));
    structs.chain(enums).chain(fields).max().unwrap_or(0)
}

/// The height of `tree`: the levels of its deepest node, which the parser holds to
/// `NESTING_LIMIT`.
fn height(tree: &Tree) -> u32 {
    let (mut low, mut high) = (0, NESTING_LIMIT);
    // `too_deep(limit)` is `None` exactly when no node is higher than `limit`.
    if tree.too_deep(high).is_some() {
        return NESTING_LIMIT + 1;
    }
    while low < high {
        let mid = low + (high - low) / 2;
        if tree.too_deep(mid).is_some() {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low
}

/// The levels a value of type `t` nests through its elements and fields; `NESTING_LIMIT + 1`
/// past the limit, where a declaration would name itself (`seen` declarations deep).
fn depth(m: &CheckedModule, t: &Ty, seen: u32) -> u32 {
    if seen > NESTING_LIMIT {
        return NESTING_LIMIT + 1;
    }
    let fields = |tys: Vec<&Ty>| tys.into_iter().map(|t| depth(m, t, seen + 1)).max();
    1 + match t {
        Ty::Array(_, elem) => depth(m, elem, seen),
        Ty::Struct(s) => m
            .structs
            .get(s)
            .and_then(|d| fields(d.fields.iter().map(|f| &f.ty).collect()))
            .unwrap_or(0),
        Ty::Enum(e) => m
            .structs
            .get_enum(e)
            .and_then(|d| {
                fields(
                    d.variants
                        .iter()
                        .flat_map(|v| v.fields.iter().map(|f| &f.ty))
                        .collect(),
                )
            })
            .unwrap_or(0),
        Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Any => 0,
    }
}
