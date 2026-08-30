//! The `canvas::Image` resource record, and the pieces its members share.
//!
//! An `Image` is a plain RES resource on the canonical header — `tag@0`,
//! `handle@8`, `closed@16`, `STATE@24` — with the image's own fields in the tail at
//! 32+. Ownership is MFB scope, exactly like an open file: `destroyImage` (or
//! scope-drop of the owner) sets the closed flag, and using a closed one is the
//! universal `ErrResourceClosed`. There is no refcount and no generation table.
//!
//! `handle@8` is the backend's id for the image, and it is the *only* thing a scene
//! ever carries (through an `ImageRef`). That is what makes an installed scene
//! independent of every resource's lifetime: it holds an integer, so destroying an
//! image a scene still names cannot dangle anything. The backend defers freeing the
//! real object until the GPU has drained the last frame that drew it — a rule that
//! lives entirely runtime-side (plan-98-D) and is invisible from MFBASIC.

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
/// The shadow is not a cache of the backend's copy — it is the source of truth the
/// backend is uploaded *from*. That is what lets `canvas::getBytes` answer without a
/// GPU readback, and what lets a lost device be recovered by re-uploading rather
/// than by asking the program to redraw.
pub(crate) const IMAGE_PIXELS: usize = 48;
/// Non-zero when the shadow has changed since the backend last saw it, so the
/// upload can be coalesced to at most one per frame rather than one per `setBytes`.
pub(crate) const IMAGE_DIRTY: usize = 56;
/// The frame counter value when the backend last drew this image (plan-98-D stamps
/// it). Reserved here because the free gate is `closed AND lastUsedFrame <
/// lastCompletedFrame` — a monotonic compare, not a reference count.
pub(crate) const IMAGE_LAST_USED_FRAME: usize = 64;

// The tail must fit the canonical 96-byte envelope, and must not overlap the header.
const _: () = assert!(IMAGE_WIDTH == RESOURCE_OFFSET_STATE + 8);
const _: () = assert!(IMAGE_LAST_USED_FRAME + 8 <= RESOURCE_RECORD_SIZE_BYTES);

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
