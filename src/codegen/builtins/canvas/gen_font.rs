//! The `canvas::Font` resource record — the twin of `canvas::Image`.
//!
//! A `Font` is a plain RES resource on the canonical header — `tag@0`, `handle@8`,
//! `closed@16`, `STATE@24`, `owner@32` — with its own fields in the tail at 40+. Ownership is MFB
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
//! actually saved. Beside it the record keeps the font's **face index** (`FONT_FACE`):
//! the small part of the file every string needs — metrics, `cmap`, advances — parsed
//! once at load, so measuring and shaping never copy the file.

// --- codegen tier imports (migration) ---
use crate::codegen::error::constants::*;

/// Pointer to the font file's bytes: a `List OF Byte` block owned by the arena.
///
/// The tail starts at `RESOURCE_TAIL_START`, immediately after the canonical header.
/// Nothing generic frees that offset — `fs::File` happens to keep its buffer pointer
/// there and reclaims it, but that is `File`'s own drop path. The one documented rule is not to put a
/// per-record *close function pointer* here.
pub(crate) const FONT_BYTES: usize = RESOURCE_TAIL_START;

/// Pointer to the font's **face index** (plan-162-A): the `List OF Integer` block
/// `__canvas_faceIndex` built from the bytes at load, holding the metrics, the `cmap`
/// runs and the advances, so text reads them by handle instead of copying the file.
///
/// It lives exactly as long as `FONT_BYTES` does — no close frees either
/// (`.ai/canvas-threading.md` §7, "close never frees"), because the graphics thread
/// reads both in place, from a frame that may still be in flight when the font closes.
pub(crate) const FONT_FACE: usize = RESOURCE_TAIL_START + 8;

// The tail must fit the canonical envelope, and must not overlap the header.
const _: () = assert!(FONT_BYTES == RESOURCE_TAIL_START);
const _: () = assert!(FONT_BYTES + 8 <= RESOURCE_RECORD_SIZE_BYTES);
const _: () = assert!(FONT_FACE == FONT_BYTES + 8);
const _: () = assert!(FONT_FACE + 8 <= RESOURCE_RECORD_SIZE_BYTES);
