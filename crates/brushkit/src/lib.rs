//! One dependency for the brushkit workspace.
//!
//! [`abr`] is `brushkit-abr` (read Photoshop `.abr` packs) and [`preview`] is
//! `brushkit-preview` (tip bitmaps for `.abr`, `.brush` and `.brushset`). The
//! two crates keep their own namespaces here because both export bitmap types.
//!
//! Features forward to the underlying crates: `text` (default) enables the
//! contact-sheet API in `preview`, `serde` enables serialization in `abr`.

pub use brushkit_abr as abr;
pub use brushkit_preview as preview;
