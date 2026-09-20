//! Defensive ceilings for untrusted input, shared by the `samp` and `patt`
//! parsers. Both blocks carry the same kinds of field, so a limit that differs
//! between them is a bug rather than a policy.

pub(crate) const MAX_DIMENSION: u32 = 16384;
pub(crate) const MAX_NAME_CODE_UNITS: usize = 1024 * 1024;
