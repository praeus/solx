//! The checks that stand between a caller and a change or a call:
//!
//! * [`scope`] — whether a call is within the scopes of whoever makes it.
//! * [`content`] — what an `Actor` may change, for every surface's action
//!   and file writes.

pub(crate) mod content;
pub mod scope;
