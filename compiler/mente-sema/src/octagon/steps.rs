//! The meter of Tier 1's work, alone in its module so that its count is private to these lines:
//! no operation of the octagon, and no other code, can build a meter from the field, and
//! [`Steps::of_a_check`] is the one start of one outside the tests. A test holds that this module
//! builds a meter once and that only a function's check starts one
//! (`test_a_meter_starts_only_where_a_check_does`).

/// Tier 1's work on its states, counted in the matrix entries their operations visit: a
/// closure's scans, relaxations and strengthenings, and every entry of a new octagon, a join or
/// a copy; a box, each bound it sets, fills or copies. It is the same on every machine, so a change or a program that
/// makes Tier 1 work more shows in it (`compiler/mente-cli/tests/golden/work.txt`). Every
/// operation that does such work takes it by `mut`, and an octagon has no `Clone`, only
/// [`Octagon::copy`](super::Octagon::copy), and no equality outside the tests, only
/// [`Octagon::same_bounds`](super::Octagon::same_bounds), so none goes uncounted. Reading bounds,
/// forgetting a variable and stamping one cost time linear in the variables and are not counted.
/// [`Octagon::constraints`](super::Octagon::constraints), quadratic, is not counted either: only
/// the SMT cross-check reads it, when `Ask::queries` asks, which no compile does, and the count is
/// of a compile's work.
///
/// A meter is neither `Clone` nor `Copy`, nor `Default` outside the tests, which the compiler
/// holds below, and its count is private to this module, so a meter is had only by starting one,
/// through [`Steps::of_a_check`], or, in the tests alone, `Default`.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(Default))]
pub struct Steps(u64);

impl Steps {
    /// A meter at zero, the only start of one outside the tests. Only a function's check starts
    /// one (`tier1::checked`), which hands it to every pass and every operation by `mut`, so that
    /// no count lands on a meter of its own and is lost: a test holds that it is called there
    /// alone (`test_a_meter_starts_only_where_a_check_does`), which no visibility can say, since
    /// `pub(in path)` names only modules that hold this one.
    pub(crate) fn of_a_check() -> Steps {
        Steps(0)
    }

    /// The entries visited so far.
    pub fn count(&self) -> u64 {
        self.0
    }

    /// Counts `entries` more, for the octagon's operations alone.
    pub(super) fn add(&mut self, entries: u64) {
        self.0 = self.0.saturating_add(entries);
    }
}

/// A constant every type has once, through the first impl below, and a type with a trait one of
/// the others names has again, so that naming it for such a type is ambiguous and does not
/// compile.
trait Lacks<Marker> {
    const HOLDS: () = ();
}

/// Every type has the constant once, through this.
impl<T> Lacks<()> for T {}

/// A second time for a type that can be cloned, which a copy of a meter, its count lost, needs.
impl<T: Clone> Lacks<u8> for T {}

/// A second time for a type with a default, which a meter of its own, its count lost, needs;
/// outside the tests, which start their meters by `Default`.
#[cfg(not(test))]
impl<T: Default> Lacks<u16> for T {}

const _: () = <Steps as Lacks<_>>::HOLDS;
