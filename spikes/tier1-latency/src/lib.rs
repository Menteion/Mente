//! M0b spike: Tier 1 on the generated module of the Tier-1 latency protocol (`ROADMAP.md`,
//! "Tier-1 re-check after an edit"). A spike lives outside the compiler crates, is tested, and is
//! never cited as evidence for a claim (`docs/design-brief.md`, "Spikes before the milestone that
//! needs them"); what it finds goes into the register. See README.md.

pub mod ast;
pub mod generate;
pub mod lexer;
pub mod octagon;
pub mod parser;
pub mod session;
pub mod tier1;

/// A diagnostic: a byte range of the module source and a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diag {
    pub start: usize,
    pub end: usize,
    pub message: String,
}

impl Diag {
    pub fn new(start: usize, end: usize, message: &str) -> Diag {
        Diag {
            start,
            end,
            message: message.to_string(),
        }
    }
}
