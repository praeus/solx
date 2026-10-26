//! Test-only code: fixtures for the crate's unit tests, and the principal
//! probe handler dependents enable through the `test-probe` feature.

pub mod probe;
#[cfg(test)]
pub(crate) mod support;
