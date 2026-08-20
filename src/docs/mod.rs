//! Embedded documentation: the `mfb spec` language specification (`spec`) and the
//! `mfb man <topic>` prose guides (`man`). Both walk a directory tree at build
//! time, embed every page via `include_str!`, and render to width-aware terminal
//! text through the shared Markdown renderer in [`render`].
//!
//! The built-in `mfb man <package>` / `mfb man <package> <function>` help pages
//! are rendered from the descriptor registry (`crate::codegen::registry`) by
//! `crate::cli::man`, not from an embedded tree; the [`man`] module here only
//! supplies the language guide topics that command falls back to.

pub(crate) mod man;
pub(crate) mod render;
pub(crate) mod spec;
