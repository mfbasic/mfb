//! Embedded documentation: the `mfb spec` language specification (`spec`). It
//! walks a directory tree at build time, embeds every page via `include_str!`,
//! and renders to width-aware terminal text through the shared Markdown renderer
//! in [`render`]. (The legacy markdown-based `mfb man` tree was retired to
//! `planning/old_man`; `mfb man2` renders built-in help from the descriptor
//! registry instead.)

pub(crate) mod render;
pub(crate) mod spec;
