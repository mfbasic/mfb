//! The `canvas::Font` resource record — the twin of `canvas::Image`.
//!
//! A `Font` is a plain RES resource on the canonical header — `tag@0`, `handle@8`,
//! `closed@16`, `STATE@24` — with its own fields in the tail at 32+. Ownership is MFB
//! scope, exactly like an open file: `destroyFont` (or scope-drop of the owner) sets
//! the closed flag, and using a closed one is the universal `ErrResourceClosed`.
//!
//! `handle@8` is the backend's id, and it is the *only* thing a scene ever carries
//! (through a `FontRef`). That is what keeps an installed scene independent of every
//! resource's lifetime — a `Text` item holds an integer, so releasing a font a scene
//! still names cannot dangle anything; that text draws as empty instead.
//!
//! **What a `Font` owns that an `Image` does not: the file's bytes, undecoded.** A
//! TrueType file *is* the glyph database — `loca` indexes `glyf` by glyph id — so
//! decoding up front would mean deciding in advance which glyphs a program will draw.
//! The record keeps the file; the per-glyph raster cache is where repeated work is
//! actually saved.

// --- codegen tier imports (migration) ---
use crate::codegen::error::constants::*;

/// Pointer to the font file's bytes: a `List OF Byte` block owned by the arena.
///
/// The tail starts at 32, immediately after the canonical header. Nothing generic
/// frees offset 32 — `fs::File` happens to keep its buffer pointer there and reclaims
/// it, but that is `File`'s own drop path. The one documented rule is not to put a
/// per-record *close function pointer* here.
pub(crate) const FONT_BYTES: usize = 32;

// The tail must fit the canonical 96-byte envelope, and must not overlap the header.
const _: () = assert!(FONT_BYTES == RESOURCE_OFFSET_STATE + 8);
const _: () = assert!(FONT_BYTES + 8 <= RESOURCE_RECORD_SIZE_BYTES);
