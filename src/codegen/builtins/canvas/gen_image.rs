//! The `canvas::Image` resource record, and the pieces its members share.
//!
//! An `Image` is a plain RES resource on the canonical header — `tag@0`,
//! `handle@8`, `closed@16`, `STATE@24` — with the image's own fields in the tail at
//! 32+. Ownership is MFB scope, exactly like an open file: `destroyImage` (or
//! scope-drop of the owner) sets the closed flag, and using a closed one is the
//! universal `ErrResourceClosed`. There is no refcount and no generation table.
//!
//! `handle@8` is the backend's id for the image — the record's own address. A scene's
//! `Picture` holds the resource itself (plan-116-I), and the renderer reads it through
//! `canvas::imageHandle`/`imageShadow`, which answer `0` once it is closed. Nothing
//! frees an `Image`'s record or its pixel block — not a close, not the scope-drop — so
//! a frame that already read the block keeps valid pixels whatever the worker does,
//! and there is no GPU-side object to release (bug-484: every backend copies a frame's
//! texels from the block while the frame is built).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;

/// Pixel width. The tail starts at 32, immediately after the canonical header.
///
/// Nothing generic frees offset 32 — `fs::File` happens to keep its buffer pointer
/// there and reclaims it, but that is `File`'s own drop path, and `process` stores a
/// plain fd at the same offset. The one documented rule is not to put a per-record
/// *close function pointer* here.
pub(crate) const IMAGE_WIDTH: usize = 32;
/// Pixel height.
pub(crate) const IMAGE_HEIGHT: usize = 40;
/// Pointer to the CPU-side pixel shadow: a `List OF Byte` block of exactly
/// `width * height * 4` RGBA8 bytes, owned by the arena.
///
/// The shadow is not a cache of the backend's copy — it is the only copy, and every
/// renderer draws *from* it. That is what lets `canvas::getBytes` answer without a
/// GPU readback.
///
/// `canvas::setBytes` swaps a fresh block in rather than writing this one, and no
/// block is ever freed, so the address doubles as the content's generation: a
/// picture's geometry header carries it, and a changed address is what makes the
/// cache, the damage diff and the GPU copy see new pixels (bug-484).
///
/// Two further words used to be reserved here, a dirty flag and a last-drawn frame
/// stamp, for a texture upload-and-deferred-free protocol. bug-484 drew pictures with
/// no texture object — the texels are copied into each frame's buffer — so neither had
/// a reader and both were removed rather than kept written for nobody.
pub(crate) const IMAGE_PIXELS: usize = 48;

// The tail must fit the canonical 96-byte envelope, and must not overlap the header.
const _: () = assert!(IMAGE_WIDTH == RESOURCE_OFFSET_STATE + 8);
const _: () = assert!(IMAGE_PIXELS + 8 <= RESOURCE_RECORD_SIZE_BYTES);

/// Bytes per pixel. RGBA8 is the one pixel format the canvas surface takes, so a
/// pixel count is always a byte count divided by exactly this.
pub(crate) const BYTES_PER_PIXEL: usize = 4;

/// Emit the closed-resource guard: branch to `closed_label` when the record's
/// `closed@16` word is non-zero.
///
/// The word is a flag *set*, not a boolean — bit 0 is closed and bit 1 is moved —
/// so the test is "non-zero", not "equals 1". That is what makes a moved resource
/// refuse every operation with no extra code.
pub(crate) fn emit_closed_guard(
    builder: &mut CodeBuilder,
    record: impl Into<Operand>,
    closed_label: &str,
) {
    let flags = builder.temporary_vreg();
    builder.emit(abi::load_u64(&flags, record, RESOURCE_OFFSET_CLOSED));
    builder.emit(abi::compare_immediate(&flags, "0"));
    builder.emit(abi::branch_ne(closed_label));
}
