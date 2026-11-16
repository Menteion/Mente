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
    let levels = Levels::of_module(m);
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
        .map(|t| levels.of(m, t));
    structs.chain(enums).chain(fields).max().unwrap_or(0)
}

/// The height of `tree`: the levels of its deepest node, which the parser holds to
/// `NESTING_LIMIT`, so that the search for it starts there.
fn height(tree: &Tree) -> u32 {
    let (mut low, mut high) = (0, NESTING_LIMIT);
    // `too_deep(limit)` is `None` exactly when no node is higher than `limit`.
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

/// The levels a value of each declared struct and enum nests through its fields: one for the
/// value, and its deepest field's. They are worked out in rounds over the declarations, each from
/// the last round's, until a round changes none: no declaration contains itself (`E0225`), so a
/// round settles a declaration whose fields' declarations the round before settled, and the
/// work grows with the declarations and the chain of them, never with the paths through them.
struct Levels {
    structs: Vec<u32>,
    enums: Vec<u32>,
}

impl Levels {
    fn of_module(m: &CheckedModule) -> Levels {
        let mut levels = Levels {
            structs: vec![0; m.structs.defs.len()],
            enums: vec![0; m.structs.enums.len()],
        };
        loop {
            let structs: Vec<u32> = m
                .structs
                .defs
                .iter()
                .map(|d| 1 + levels.deepest(m, d.fields.iter().map(|f| &f.ty)))
                .collect();
            let enums: Vec<u32> = m
                .structs
                .enums
                .iter()
                .map(|e| {
                    1 + levels.deepest(
                        m,
                        e.variants
                            .iter()
                            .flat_map(|v| v.fields.iter().map(|f| &f.ty)),
                    )
                })
                .collect();
            if structs == levels.structs && enums == levels.enums {
                return levels;
            }
            levels = Levels { structs, enums };
        }
    }

    /// The most levels a value of any of `tys` nests, none for none.
    fn deepest<'t>(&self, m: &CheckedModule, tys: impl Iterator<Item = &'t Ty>) -> u32 {
        tys.map(|t| self.of(m, t)).max().unwrap_or(0)
    }

    /// The levels a value of type `t` nests through its elements and fields: one for the value,
    /// and its element's, or the levels of its declaration.
    fn of(&self, m: &CheckedModule, t: &Ty) -> u32 {
        match t {
            Ty::Array(_, elem) => 1 + self.of(m, elem),
            Ty::Struct(s) => m.structs.position(s).map_or(1, |i| self.structs[i]),
            Ty::Enum(e) => m.structs.enum_position(e).map_or(1, |i| self.enums[i]),
            Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Any => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(text: &str) -> CheckedModule {
        match mente_sema::check_program_without_proofs(text) {
            Ok(m) => m,
            Err(d) => panic!("{:?}\n{}", d, text),
        }
    }

    /// `count` structs, each holding the next `fields` times, the last holding a `u8`, declared
    /// the first first, so that each round of [`Levels`] settles one more of them; and a function
    /// that nests next to nothing.
    fn chain(count: usize, fields: usize) -> String {
        let mut text = String::from("module m\npub fn f(x: u8) -> u8 = x\n");
        for i in 0..count {
            let ty = if i + 1 < count {
                format!("S{}", i + 1)
            } else {
                "u8".to_string()
            };
            let held: Vec<String> = (0..fields).map(|k| format!("a{}: {}", k, ty)).collect();
            text.push_str(&format!("struct S{} {{\n  {}\n}}\n", i, held.join("\n  ")));
        }
        text
    }

    /// A value nests a level for each array, struct and enum it passes through, and its
    /// innermost value one more; a module nests as deep as its deepest value or tree, and so does
    /// a call of each of its functions. A field of `S1` in a chain of 12 structs nests 12 levels;
    /// an enum's variants' fields count as a struct's do.
    #[test]
    fn test_a_value_nests_a_level_for_each_type_it_passes_through() {
        let m = module(&chain(12, 1));
        assert_eq!(nesting(&m), 12);
        assert_eq!(function_nesting(&m, 0), 12);
        let arrays = module(
            "module m\npub fn f(x: u8) -> u8 = x\nstruct A {\n  x: [2][3]u8\n}\n\
             enum E {\n  V(a: A)\n  W\n}\nstruct B {\n  e: E\n  f: bool\n}\n",
        );
        // `[2][3]u8` nests 3 levels, `A` 4, `E` 5.
        assert_eq!(Levels::of_module(&arrays).structs, [4, 6]);
        assert_eq!(Levels::of_module(&arrays).enums, [5]);
        assert_eq!(nesting(&arrays), 5);
    }

    /// Structs that each hold the next twice nest as deep as a chain of them, found in the time a
    /// chain takes: the levels are worked out once a declaration, where following each field
    /// would take 2^30 paths through 31 of them, the most a value of 4 GiB, kept in one piece, holds.
    #[test]
    fn test_types_held_twice_are_counted_once() {
        let m = module(&chain(31, 2));
        assert_eq!(nesting(&m), 31);
        assert_eq!(
            host_bound_of_function(&m, 0),
            host_bound_at(m.frames[0].map_or(0, |f| f.bytes), 31)
        );
    }
}
