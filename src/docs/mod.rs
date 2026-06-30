//! Embedded documentation: the `mfb man` built-in help (`man`) and the `mfb
//! spec` language specification (`spec`). Both walk a directory tree at build
//! time, embed every page via `include_str!`, and render to width-aware
//! terminal text through the shared Markdown renderer in [`render`].

pub(crate) mod man;
pub(crate) mod render;
pub(crate) mod spec;
