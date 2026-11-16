//! The host stack the reference interpreter runs on, which the stack budget of item 25 bounds
//! (`core-calculus.md`, item 25; [`stack-budget`]).
//!
//! The interpreter's evaluation recurses on the host's stack, a step for each expression and each
//! block, so what one interpreted call takes there depends on the program's shape: a call nested
//! in eight `if`s takes several times what a call nested in one does, while its frame holds
//! nothing more. No figure per call bounds it, and none is assumed. So the interpreter runs in
//! segments: at each step it reads how much of its segment's stack is left, and where less than
//! [`MARGIN`] is, the step continues on a new segment, a thread started through
//! `mente_threads::alone`, which the step waits for. A run is then ended by the budget of item 25,
//! whatever its shape, and the host's stack never ends a run. The margin is one figure for
//! every build, made in one place of the largest step measured and of what the platform keeps of
//! a thread's stack.
//!
//! What the segments take of the host's memory is bounded by how deep the program nests: each
//! call open takes at most [`CALLS`]' figure and [`LEVELS`]' for each level of nesting, which
//! every shape a call can stand in is held to, and [`host_bound`] is what a run of a given nesting
//! reserves at most, in any build. The harnesses that run generated and mutated programs derive
//! the nesting they keep to from it.
//!
//! This module is the one sizing rule: every thread the interpreter runs on gets [`SEGMENT`], and
//! no size is written where a thread starts.
//!
//! [`stack-budget`]: ../../docs/design-review.md#stack-budget

use std::hint::black_box;

/// The most host stack one step of the interpreter takes, from a check to the deepest point of
/// what runs before the next, in the build that takes the most, with headroom: twice the largest
/// step the test `test_no_step_outgrows_the_host_step` measures, on each shape nested as deep as
/// the parser allows, rounded up. On 2026-11-16, on x86-64 Windows, that was 20,656 bytes in a
/// build without optimisation, which keeps a slot for every local of a function, and 4,184 in one
/// with it, both the step that enters a call. Every build uses the one figure, so that a step
/// measured in one build holds in the other.
pub(crate) const STEP: usize = 48 << 10;

/// The guard page the platform keeps below a thread's stack, which no frame may reach: at most
/// 64 KiB, the largest page AArch64 Linux runs with. Kept on every platform, so that the margin is
/// one figure everywhere.
const GUARD_PAGE: usize = 64 << 10;

/// What Windows keeps of every Rust thread's stack for its handler of an overflow: Rust's
/// standard library asks for 20 KiB (`SetThreadStackGuarantee`), which no frame may reach. Kept
/// on every platform, as the guard page is.
const STACK_GUARANTEE: usize = 20 << 10;

/// What starting a thread takes of the stack of the thread that starts it, beyond the guard page
/// and the guarantee: the platform's own code. On 2026-11-16, on x86-64 Windows, a thread started
/// with 28 to 32 KiB left in a build with optimisation, and with 36 to 40 KiB left in one without,
/// the guarantee included; with headroom, 64 KiB.
const THREAD_START: usize = 64 << 10;

/// What a segment keeps free: where less is left, the next step starts a segment. The step begun
/// where the check found the margin may take [`STEP`], and a step more is headroom; the segment
/// the next step starts then takes [`THREAD_START`], above the platform's [`STACK_GUARANTEE`] and
/// [`GUARD_PAGE`]. A thread's stack is address space the platform reserves and commits only as
/// it is used, so a generous margin costs next to nothing.
/// `test_a_segment_starts_from_the_least_a_switch_begins_with` holds it.
pub(crate) const MARGIN: usize = 2 * STEP + THREAD_START + STACK_GUARANTEE + GUARD_PAGE;

/// The stack of each thread the interpreter runs on: room for many steps, so that a deep run
/// starts few segments.
pub(crate) const SEGMENT: usize = 1024 * STEP;

/// The most host stack one call takes besides its levels of nesting, from where it begins to the
/// deepest point it reaches before a call it makes begins: in a build without optimisation, and
/// in one with it. The shapes a call can stand in hold the build that runs them to its figure
/// ([`host_figures`]).
pub(crate) const CALLS: [usize; 2] = [4 << 10, 1 << 10];

/// The most host stack one level of nesting adds to a call, as the parser counts levels
/// (`NESTING_LIMIT`): in a build without optimisation, and in one with it, held as [`CALLS`] is.
pub(crate) const LEVELS: [usize; 2] = [5376, 2048];

/// The figures of the build that runs: what a call takes, and what each level adds, which the
/// shapes a call can stand in are held to (`mente-cli`'s `mir_run::host_shapes`).
pub const fn host_figures() -> (usize, usize) {
    let build = if cfg!(debug_assertions) { 0 } else { 1 };
    (CALLS[build], LEVELS[build])
}

/// The larger of two figures.
const fn larger(a: usize, b: usize) -> usize {
    if a > b {
        a
    } else {
        b
    }
}

/// The most host stack one open call of a function nesting `depth` levels takes, in any build: a
/// call's figure and a level's for each level, of the larger build's.
const fn call_cost(depth: u32) -> u128 {
    larger(CALLS[0], CALLS[1]) as u128 + larger(LEVELS[0], LEVELS[1]) as u128 * depth as u128
}

/// What the segments that hold `used` bytes of host stack reserve: each but the last is filled to
/// within its margin before the next begins, and each reserves [`SEGMENT`] and the guard page the
/// platform keeps below it.
const fn reserved(used: u128) -> u128 {
    (used / (SEGMENT - MARGIN) as u128 + 1) * (SEGMENT + GUARD_PAGE) as u128
}

/// The most host memory a run of the reference interpreter reserves for its stacks, in any build,
/// where every function it can call has a frame of at least `frame` bytes and nests at most
/// `depth` levels (`crate::nesting`). The frames of the calls open at once fill at most the
/// budget, so there are at most as many as it holds of `frame`, each taking at most a call's
/// figure and a level's for each level; one call nesting as deep as the parser allows more covers
/// what the run takes before its first call. A program's own bound is the largest of its
/// functions' (`crate::host_bound_of`), a function whose calls take more host stack for each
/// byte of frame bounding more.
pub const fn host_bound_at(frame: u128, depth: u32) -> u128 {
    let frame = if frame < mente_sema::frames::CALL {
        mente_sema::frames::CALL
    } else {
        frame
    };
    let calls = mente_sema::frames::BUDGET.div_ceil(frame);
    reserved(calls * call_cost(depth) + call_cost(mente_syntax::body::NESTING_LIMIT))
}

/// The most host memory a run of any program reserves for its stacks, in any build, where nothing
/// the program declares nests deeper than `depth` levels: every frame is at least the call's 64
/// bytes.
pub const fn host_bound(depth: u32) -> u128 {
    host_bound_at(mente_sema::frames::CALL, depth)
}

/// Where the interpreter stands on the host's stack: the address its segment began at, the
/// address of the last check, the largest step so far, from a check to the deepest point of what
/// follows it before the next; and the address the current call began at, and the most host
/// stack a call has taken so far, from where it began to the deepest point it reached before a
/// call it makes began.
#[derive(Clone, Debug)]
pub(crate) struct Host {
    base: usize,
    last: usize,
    deepest: usize,
    call: usize,
    per_call: usize,
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
    /// The host stack of a segment that begins here, on a thread of [`SEGMENT`] bytes, inside a
    /// call that has taken `carried` bytes of the segments before it; an error where the stack
    /// does not grow down.
    pub(crate) fn start(carried: usize) -> Result<Host, &'static str> {
        let base = here();
        if !deeper_is_lower(base) {
            return Err(GROWS_UP);
        }
        Ok(Host {
            base,
            last: base,
            deepest: 0,
            call: base.saturating_add(carried),
            per_call: 0,
        })
    }

    /// What the current call has taken of this segment and those before it, at `at`: the bytes a
    /// segment started there carries.
    pub(crate) fn carried(&self, at: usize) -> usize {
        self.call.saturating_sub(at)
    }

    /// Whether less than [`MARGIN`] of the segment's stack is left at `at`.
    pub(crate) fn low(&self, at: usize) -> bool {
        self.base.saturating_sub(at) + MARGIN > SEGMENT
    }

    /// A check at `at`: records the step from the last check, and returns that check's address,
    /// for the step to restore when it ends.
    pub(crate) fn enter(&mut self, at: usize) -> usize {
        self.touch(at);
        std::mem::replace(&mut self.last, at)
    }

    /// The end of the step that [`Host::enter`] began, `outer` its caller's check.
    pub(crate) fn leave(&mut self, outer: usize) {
        self.last = outer;
    }

    /// A probe at `at`, in a function a step runs below its last check: records how far below that
    /// check the step reached, and how far below its start the current call did.
    pub(crate) fn touch(&mut self, at: usize) {
        self.deepest = self.deepest.max(self.last.saturating_sub(at));
        self.per_call = self.per_call.max(self.call.saturating_sub(at));
    }

    /// The start of a call at `at`, made by the current call, which has reached at least that far:
    /// returns where the current call began, for [`Host::returned`] to restore.
    pub(crate) fn called(&mut self, at: usize) -> usize {
        self.touch(at);
        std::mem::replace(&mut self.call, at)
    }

    /// The end of the call [`Host::called`] began, `outer` where its caller began.
    pub(crate) fn returned(&mut self, outer: usize) {
        self.call = outer;
    }

    /// The largest step between two checks so far.
    #[cfg(test)]
    pub(crate) fn deepest(&self) -> usize {
        self.deepest
    }

    /// The most host stack a call has taken so far, from where it began to the deepest point it
    /// reached before a call it makes began.
    pub(crate) fn per_call(&self) -> usize {
        self.per_call
    }

    /// This host's records joined with a segment's.
    pub(crate) fn absorb(&mut self, segment: &Host) {
        self.deepest = self.deepest.max(segment.deepest);
        self.per_call = self.per_call.max(segment.per_call);
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
        assert!(Host::start(0).is_ok());
    }

    /// Recurses, a frame of a few hundred bytes at a time, until no more than `left` is left of a
    /// segment of `size` begun at `base`, as `Host` measures it, and starts a segment there as
    /// `Machine::segment` does, returning what the new segment returns.
    #[inline(never)]
    fn dive(base: usize, size: usize, left: usize) -> u32 {
        let pad = [0u8; 256];
        black_box(&pad);
        if size - base.saturating_sub(here()) > left {
            dive(base, size, left) + black_box(0)
        } else {
            match mente_threads::alone(None, Some(size), || 7u32) {
                Ok(Ok(v)) => v,
                _ => 0,
            }
        }
    }

    /// A switch begins with at least the margin less a step left: the check before found the
    /// margin, and one step ran since. Starting a segment from there runs, so that reserves too
    /// thin for the start of a thread fail here first: a release build's margin of 32 KiB once let
    /// a recursion run its host stack out on Windows, where a thread's start needs about 30 KiB
    /// left.
    #[test]
    fn test_a_segment_starts_from_the_least_a_switch_begins_with() {
        let ran = mente_threads::alone(None, Some(SEGMENT), || {
            let base = here();
            dive(base, SEGMENT, MARGIN - STEP)
        });
        assert!(matches!(ran, Ok(Ok(7))));
    }

    /// A segment is low once less than the margin is left of it, and not before.
    #[test]
    fn test_a_segment_is_low_once_less_than_the_margin_is_left() {
        let h = Host {
            base: SEGMENT + 1000,
            last: SEGMENT + 1000,
            deepest: 0,
            call: SEGMENT + 1000,
            per_call: 0,
        };
        assert!(!h.low(1000 + MARGIN));
        assert!(h.low(1000 + MARGIN - 1));
    }
}
