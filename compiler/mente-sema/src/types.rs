//! The types of M1 (`spec/core-calculus.md`, items 2 to 5), and M2's structs and enums (items 13
//! and 14).
//!
//! An integer type is a range, and any integer value flows into any integer type, checked where
//! it is narrowed; so the type checker compares integer types only where the spec makes one
//! type matter: an unannotated binding, the operands of `&`, `|` and `^`, and a `mut` argument.
//! Refinements are Tier 1's to prove and are not part of a [`Ty`].

use mente_syntax::tree::{Tree, TypeId, TypeKind};

/// A standard integer type. `usize` is 64 bits wide and unsigned, and a type of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntTy {
    pub name: &'static str,
    pub signed: bool,
    pub bits: u32,
}

/// The standard integer types, smallest range first, as a suggestion picks them.
pub const INT_TYPES: [IntTy; 11] = [
    int("u8", false, 8),
    int("i8", true, 8),
    int("u16", false, 16),
    int("i16", true, 16),
    int("u32", false, 32),
    int("i32", true, 32),
    int("u64", false, 64),
    int("usize", false, 64),
    int("i64", true, 64),
    int("u128", false, 128),
    int("i128", true, 128),
];

const fn int(name: &'static str, signed: bool, bits: u32) -> IntTy {
    IntTy { name, signed, bits }
}

/// The type an expression of literals alone takes.
pub const I64: IntTy = int("i64", true, 64);
/// The type of `a.len`.
pub const USIZE: IntTy = int("usize", false, 64);

impl IntTy {
    pub fn named(name: &str) -> Option<IntTy> {
        INT_TYPES.iter().copied().find(|t| t.name == name)
    }

    /// The largest value of the type.
    pub fn max(self) -> u128 {
        match (self.signed, self.bits) {
            (false, 128) => u128::MAX,
            (false, bits) => (1 << bits) - 1,
            (true, bits) => (1 << (bits - 1)) - 1,
        }
    }

    /// The magnitude of the smallest value of the type: 0, or 2^(N-1) for `iN`.
    pub fn min_magnitude(self) -> u128 {
        if self.signed {
            1 << (self.bits - 1)
        } else {
            0
        }
    }

    /// The range of the type, as a diagnostic writes it: "0 to 255", "-128 to 127".
    pub fn range_text(self) -> String {
        if self.signed {
            format!("-{} to {}", self.min_magnitude(), self.max())
        } else {
            format!("0 to {}", self.max())
        }
    }

    /// Whether every value of `other` is a value of `self`.
    pub fn holds(self, other: IntTy) -> bool {
        match (self.signed, other.signed) {
            (true, false) => self.bits > other.bits,
            (false, true) => false,
            _ => self.bits >= other.bits,
        }
    }
}

/// The smallest standard type that holds both, preferring either one itself.
pub fn smallest_holding(a: IntTy, b: IntTy) -> Option<IntTy> {
    if a.holds(b) {
        return Some(a);
    }
    if b.holds(a) {
        return Some(b);
    }
    INT_TYPES.iter().copied().find(|t| t.holds(a) && t.holds(b))
}

/// The integer type of an expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Int {
    /// Literals alone, without a suffix: the type of what they meet, or `i64`.
    Literal,
    /// Operands of one type.
    One(IntTy),
    /// Operands of different types, with the smallest standard type that holds them all, if
    /// there is one.
    Mixed(Option<IntTy>),
}

impl Int {
    /// The type of an operation on operands of these types (item 5).
    pub fn join(self, other: Int) -> Int {
        match (self, other) {
            (Int::Literal, x) | (x, Int::Literal) => x,
            (Int::One(a), Int::One(b)) if a == b => Int::One(a),
            (a, b) => Int::Mixed(
                a.holder()
                    .zip(b.holder())
                    .and_then(|(a, b)| smallest_holding(a, b)),
            ),
        }
    }

    /// The smallest standard type that holds this one's values, if there is one.
    fn holder(self) -> Option<IntTy> {
        match self {
            Int::Literal => Some(I64),
            Int::One(t) => Some(t),
            Int::Mixed(t) => t,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    Int(Int),
    Bool,
    Array(u128, Box<Ty>),
    /// A struct of the module, by its name: two structs are one type only when they are one
    /// declaration, and a module declares a name once.
    Struct(String),
    /// An enum of the module, by its name, likewise.
    Enum(String),
    /// What a statement produces, and a call of a function without a result type: no value.
    Unit,
    /// `todo`, `unreachable`, and what a diagnostic replaced: any type, so that one mistake is
    /// reported once.
    Any,
}

impl Ty {
    pub fn int(t: IntTy) -> Ty {
        Ty::Int(Int::One(t))
    }

    /// The type a type expression names, its refinement aside.
    pub fn of(tree: &Tree, t: TypeId) -> Ty {
        match &tree.ty(t).kind {
            TypeKind::Int { name, .. } => IntTy::named(name).map_or(Ty::Any, Ty::int),
            TypeKind::Bool => Ty::Bool,
            TypeKind::Array { len, elem } => Ty::Array(*len, Box::new(Ty::of(tree, *elem))),
            TypeKind::Named(name) => Ty::Struct(name.clone()),
            TypeKind::Enum(name) => Ty::Enum(name.clone()),
            TypeKind::Error => Ty::Any,
        }
    }

    /// The type both of two values have, for the branches of an `if` and the elements of an
    /// array literal; `None` when they have none.
    pub fn join(&self, other: &Ty) -> Option<Ty> {
        match (self, other) {
            (Ty::Any, t) | (t, Ty::Any) => Some(t.clone()),
            (Ty::Int(a), Ty::Int(b)) => Some(Ty::Int(a.join(*b))),
            (Ty::Array(n, a), Ty::Array(m, b)) if n == m => {
                a.join(b).map(|e| Ty::Array(*n, Box::new(e)))
            }
            (a, b) if a == b => Some(a.clone()),
            _ => None,
        }
    }

    /// Whether a value of type `value` may flow into a place of this type, narrowed where it
    /// enters (item 4).
    pub fn accepts(&self, value: &Ty) -> bool {
        match (self, value) {
            (Ty::Any, _) | (_, Ty::Any) => true,
            (Ty::Int(_), Ty::Int(_)) => true,
            (Ty::Array(n, a), Ty::Array(m, b)) => n == m && a.accepts(b),
            (a, b) => a == b,
        }
    }

    /// Whether a place of type `place` may be passed as `mut` to a parameter of this type:
    /// the call writes the parameter back into the place, so their types are the same, and
    /// the write-back never fails (`core-calculus.md`, item 7).
    pub fn same_as(&self, place: &Ty) -> bool {
        match (self, place) {
            (Ty::Any, _) | (_, Ty::Any) => true,
            (Ty::Array(n, a), Ty::Array(m, b)) => n == m && a.same_as(b),
            (a, b) => a == b,
        }
    }

    /// Whether no value of this type exists: no type, which a value that leaves has, or an
    /// array of at least one element of such a type (`core-calculus.md`, item 6). An array
    /// type of no element has one value, `[]`, whatever its element type.
    pub fn has_no_value(&self) -> bool {
        match self {
            Ty::Any => true,
            Ty::Array(n, elem) => *n > 0 && elem.has_no_value(),
            Ty::Int(_) | Ty::Bool | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit => false,
        }
    }

    /// The type an unannotated binding takes: literals alone become `i64`.
    pub fn settled(&self) -> Ty {
        match self {
            Ty::Int(Int::Literal) => Ty::int(I64),
            Ty::Array(n, e) => Ty::Array(*n, Box::new(e.settled())),
            t @ (Ty::Int(Int::One(_) | Int::Mixed(_))
            | Ty::Bool
            | Ty::Struct(_)
            | Ty::Enum(_)
            | Ty::Unit
            | Ty::Any) => t.clone(),
        }
    }

    /// Whether this type mixes integer types, with the smallest standard type that holds
    /// them all, if there is one.
    pub fn mixed(&self) -> Option<Option<IntTy>> {
        match self {
            Ty::Int(Int::Mixed(t)) => Some(*t),
            Ty::Array(_, e) => e.mixed(),
            Ty::Int(Int::Literal | Int::One(_))
            | Ty::Bool
            | Ty::Struct(_)
            | Ty::Enum(_)
            | Ty::Unit
            | Ty::Any => None,
        }
    }

    /// The type with `holder` in place of its mixed integers, as a suggestion spells it.
    pub fn with_int(&self, holder: IntTy) -> Ty {
        match self {
            Ty::Int(_) => Ty::int(holder),
            Ty::Array(n, e) => Ty::Array(*n, Box::new(e.with_int(holder))),
            t @ (Ty::Bool | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit | Ty::Any) => t.clone(),
        }
    }

    /// How a diagnostic names the type: `` `u8` ``, `` `[4]u8` ``, "an integer", "no value".
    pub fn describe(&self) -> String {
        match self {
            Ty::Int(Int::One(_)) | Ty::Bool | Ty::Array(..) | Ty::Struct(_) | Ty::Enum(_) => {
                format!("`{}`", self.spelling())
            }
            Ty::Int(_) => "an integer".to_string(),
            Ty::Unit => "no value".to_string(),
            Ty::Any => "any type".to_string(),
        }
    }

    /// The type as Mente writes it.
    pub fn spelling(&self) -> String {
        match self {
            Ty::Int(Int::One(t)) => t.name.to_string(),
            Ty::Int(_) => "integer".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Array(n, e) => format!("[{}]{}", n, e.spelling()),
            Ty::Struct(name) | Ty::Enum(name) => name.clone(),
            Ty::Unit => "()".to_string(),
            Ty::Any => "_".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(name: &str) -> IntTy {
        IntTy::named(name).unwrap_or(I64)
    }

    #[test]
    fn test_integer_types_hold_the_ranges_of_their_names() {
        assert!(t("u16").holds(t("u8")));
        assert!(t("u8").holds(t("u8")));
        assert!(!t("u8").holds(t("u16")));
        assert!(t("i16").holds(t("u8")));
        assert!(!t("i8").holds(t("u8")));
        assert!(!t("u64").holds(t("i8")));
        assert!(t("usize").holds(t("u64")) && t("u64").holds(t("usize")));
        assert_eq!(IntTy::named("u7"), None);
    }

    #[test]
    fn test_integer_types_know_their_bounds() {
        assert_eq!(t("u8").max(), 255);
        assert_eq!(t("u16").max(), 65_535);
        assert_eq!(t("i8").max(), 127);
        assert_eq!(t("u128").max(), u128::MAX);
        assert_eq!(t("i128").max(), (1 << 127) - 1);
        assert_eq!(t("u64").min_magnitude(), 0);
        assert_eq!(t("i8").min_magnitude(), 128);
        assert_eq!(t("i128").min_magnitude(), 1 << 127);
        assert_eq!(t("u8").range_text(), "0 to 255");
        assert_eq!(t("i16").range_text(), "-32768 to 32767");
    }

    #[test]
    fn test_mixed_operands_suggest_the_smallest_type_that_holds_both() {
        let holder = |a: &str, b: &str| smallest_holding(t(a), t(b)).map(|t| t.name);
        assert_eq!(holder("u8", "u32"), Some("u32"));
        assert_eq!(holder("u32", "u8"), Some("u32"));
        assert_eq!(holder("i32", "u32"), Some("i64"));
        assert_eq!(holder("u8", "i8"), Some("i16"));
        assert_eq!(holder("u64", "i64"), Some("i128"));
        assert_eq!(holder("usize", "u32"), Some("usize"));
        assert_eq!(holder("u128", "i8"), None);
    }

    #[test]
    fn test_an_expression_takes_the_one_type_of_its_operands() {
        let one = |n: &str| Int::One(t(n));
        assert_eq!(Int::Literal.join(one("u8")), one("u8"));
        assert_eq!(one("u8").join(Int::Literal), one("u8"));
        assert_eq!(Int::Literal.join(Int::Literal), Int::Literal);
        assert_eq!(one("u8").join(one("u8")), one("u8"));
        assert_eq!(one("u8").join(one("u32")), Int::Mixed(Some(t("u32"))));
        assert_eq!(
            one("u8").join(one("u32")).join(one("i8")),
            Int::Mixed(Some(t("i64")))
        );
        assert_eq!(
            Int::Mixed(Some(t("u16"))).join(Int::Literal),
            Int::Mixed(Some(t("u16")))
        );
        assert_eq!(one("u128").join(one("i8")), Int::Mixed(None));
        assert_eq!(Int::Mixed(None).join(one("u8")), Int::Mixed(None));
    }

    #[test]
    fn test_values_flow_into_places_of_their_kind() {
        let u8_ = Ty::int(t("u8"));
        let arr = |n: u128, e: Ty| Ty::Array(n, Box::new(e));
        assert!(u8_.accepts(&Ty::int(t("i128"))));
        assert!(!u8_.accepts(&Ty::Bool) && !Ty::Bool.accepts(&u8_));
        assert!(!u8_.accepts(&Ty::Unit));
        assert!(Ty::Unit.accepts(&Ty::Unit));
        assert!(u8_.accepts(&Ty::Any) && Ty::Any.accepts(&Ty::Bool));
        assert!(arr(4, u8_.clone()).accepts(&arr(4, Ty::Int(Int::Literal))));
        assert!(!arr(4, u8_.clone()).accepts(&arr(3, u8_.clone())));
        assert!(!arr(4, u8_.clone()).accepts(&arr(4, Ty::Bool)));
    }

    #[test]
    fn test_a_mut_argument_has_its_parameters_type_exactly() {
        let u8_ = Ty::int(t("u8"));
        let arr = |n: u128, e: Ty| Ty::Array(n, Box::new(e));
        assert!(u8_.same_as(&u8_));
        assert!(!u8_.same_as(&Ty::int(t("u16"))));
        assert!(!Ty::int(t("u64")).same_as(&Ty::int(t("usize"))));
        assert!(Ty::Bool.same_as(&Ty::Any) && Ty::Any.same_as(&Ty::Bool));
        assert!(!Ty::Bool.same_as(&u8_));
        assert!(arr(2, u8_.clone()).same_as(&arr(2, u8_.clone())));
        assert!(arr(2, u8_.clone()).same_as(&arr(2, Ty::Any)));
        assert!(!arr(2, u8_.clone()).same_as(&arr(3, u8_.clone())));
        assert!(!arr(2, u8_.clone()).same_as(&arr(2, Ty::int(t("u16")))));
    }

    #[test]
    fn test_join_settle_and_spell_types() {
        let u8_ = Ty::int(t("u8"));
        let lit = Ty::Int(Int::Literal);
        let arr = |n: u128, e: Ty| Ty::Array(n, Box::new(e));
        assert_eq!(Ty::Any.join(&Ty::Bool), Some(Ty::Bool));
        assert_eq!(Ty::Bool.join(&Ty::Any), Some(Ty::Bool));
        assert_eq!(u8_.join(&lit), Some(u8_.clone()));
        assert_eq!(Ty::Bool.join(&Ty::Bool), Some(Ty::Bool));
        assert_eq!(Ty::Bool.join(&u8_), None);
        assert_eq!(
            arr(2, lit.clone()).join(&arr(2, u8_.clone())),
            Some(arr(2, u8_.clone()))
        );
        assert_eq!(arr(2, lit.clone()).join(&arr(3, u8_.clone())), None);
        assert_eq!(arr(2, Ty::Bool).join(&arr(2, u8_.clone())), None);
        assert_eq!(arr(2, lit.clone()).settled(), arr(2, Ty::int(I64)));
        assert_eq!(u8_.settled(), u8_);
        let mixed = Ty::Int(Int::Mixed(Some(t("u32"))));
        assert_eq!(arr(3, mixed.clone()).mixed(), Some(Some(t("u32"))));
        assert_eq!(arr(3, u8_.clone()).mixed(), None);
        assert_eq!(
            arr(3, mixed.clone()).with_int(t("u32")).spelling(),
            "[3]u32"
        );
        assert_eq!(Ty::Bool.with_int(t("u32")), Ty::Bool);
        assert_eq!(arr(2, arr(3, Ty::Bool)).spelling(), "[2][3]bool");
        assert_eq!(u8_.describe(), "`u8`");
        assert_eq!(lit.describe(), "an integer");
        assert_eq!(mixed.spelling(), "integer");
        assert_eq!(Ty::Unit.describe(), "no value");
        assert_eq!(Ty::Unit.spelling(), "()");
        assert_eq!(Ty::Any.describe(), "any type");
        assert_eq!(Ty::Any.spelling(), "_");
    }
}
