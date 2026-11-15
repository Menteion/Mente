//! The host stack the reference interpreter runs on, which the stack budget of item 25 bounds
//! (`core-calculus.md`, item 25; [`stack-budget`]).
//!
//! The interpreter's evaluation recurses on the host's stack, a step for each expression and each
//! block, so what one interpreted call takes there depends on the program's shape: a call nested
//! in eight `if`s takes several times what a call nested in one does, while its frame holds
//! nothing more. No figure per call bounds it, and none is assumed. So the interpreter runs in
//! segments: at each step it reads how much of its segment's stack is left, and where less than
//! [`MARGIN`] is, the step continues on a new segment, a thread started through
//! `mente_threads::alone`, which the step waits for. A run is then bounded by the budget of item
//! 25, whatever its shape, and the host's stack never ends a run.
//!
//! This module is the one sizing rule: every thread the interpreter runs on gets [`SEGMENT`], and
//! no size is written where a thread starts.
//!
//! [`stack-budget`]: ../../docs/design-review.md#stack-budget

use std::hint::black_box;

/// The most host stack one step of the interpreter uses between two checks, with headroom: twice
/// the largest step the test `test_no_step_outgrows_the_host_step` measures, on each shape nested
/// as deep as the parser allows, rounded up. On 2026-11-16, on x86-64 Windows, that was 19,008
/// bytes without optimisation and 3,776 with it, both the step that enters a call; a build without
/// optimisation keeps a slot for every local of a function, so its steps are several times larger.
pub(crate) const STEP: usize = if cfg!(debug_assertions) {
    40 << 10
} else {
    8 << 10
};

/// What a segment keeps free: a step begun where the check found [`MARGIN`] left still fits, and so
/// do the frames of the thread's own start above where the segment's interpreter began.
pub(crate) const MARGIN: usize = 4 * STEP;

/// The stack of each thread the interpreter runs on: room for many steps, so that a deep run
/// starts few segments.
pub(crate) const SEGMENT: usize = 1024 * STEP;

/// Where the interpreter stands on the host's stack: the address its segment began at, the
/// address of the last check, and the largest step between two checks so far.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Host {
    base: usize,
    last: usize,
    deepest: usize,
}

/// The address of a local of the caller's frame, as a number: the depth the stack has reached.
#[inline(always)]
pub(crate) fn here() -> usize {
    let mark = 0u8;
    black_box(&mark) as *const u8 as usize
}

/// Whether a call made from a frame whose local lies at `above` places its own locals lower: the
/// stack grows down, as it does on x86-64 and AArch64, on Linux and on Windows alike.
#[inline(never)]
fn deeper_is_lower(above: usize) -> bool {
    let mark = 0u8;
    (black_box(&mark) as *const u8 as usize) < above
}

/// Why a segment cannot start: the host's stack grows up, which the segments' reading of what is
/// left does not handle.
pub(crate) const GROWS_UP: &str =
    "the host's stack grows up, and the interpreter measures what is left of it as a stack that grows down";

impl Host {
    /// The host stack of a segment that begins here, on a thread of [`SEGMENT`] bytes; an error
    /// where the stack does not grow down.
    pub(crate) fn start() -> Result<Host, &'static str> {
        let base = here();
        if !deeper_is_lower(base) {
            return Err(GROWS_UP);
        }
        Ok(Host {
            base,
            last: base,
            deepest: 0,
        })
    }

    /// Whether less than [`MARGIN`] of the segment's stack is left at `at`.
    pub(crate) fn low(&self, at: usize) -> bool {
        self.base.saturating_sub(at) + MARGIN > SEGMENT
    }

    /// A check at `at`: records the step from the last check, and returns that check's address,
    /// for the step to restore when it ends.
    pub(crate) fn enter(&mut self, at: usize) -> usize {
        self.deepest = self.deepest.max(self.last.saturating_sub(at));
        std::mem::replace(&mut self.last, at)
    }

    /// The end of the step that [`Host::enter`] began, `outer` its caller's check.
    pub(crate) fn leave(&mut self, outer: usize) {
        self.last = outer;
    }

    /// The largest step between two checks so far.
    #[cfg(test)]
    pub(crate) fn deepest(&self) -> usize {
        self.deepest
    }

    /// This host's record of steps joined with a segment's.
    pub(crate) fn absorb(&mut self, segment: &Host) {
        self.deepest = self.deepest.max(segment.deepest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The segments read what is left of the stack as a stack that grows down; on the hosts the
    /// interpreter runs on, x86-64 and AArch64 on Linux and Windows, it does.
    #[test]
    fn test_the_host_stack_grows_down() {
        assert!(deeper_is_lower(here()));
        assert!(Host::start().is_ok());
    }

    /// A segment is low once less than the margin is left of it, and not before.
    #[test]
    fn test_a_segment_is_low_once_less_than_the_margin_is_left() {
        let h = Host {
            base: SEGMENT + 1000,
            last: SEGMENT + 1000,
            deepest: 0,
        };
        assert!(!h.low(1000 + MARGIN));
        assert!(h.low(1000 + MARGIN - 1));
    }
}
