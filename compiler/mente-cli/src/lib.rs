//! The parts of `mente-cli` that its integration tests read as `mn` does, so that each is defined
//! once: the forms the project's records are written in, the record of escaped bugs, and the
//! report of the pressure on the repository's files.

pub mod escapes;
pub mod evidence;
pub mod json;
pub mod pressure;
pub mod records;
