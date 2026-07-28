# bug-364: `bindings/libsnd` declares `SF_INFO` without its leading `frames` field, so every field reads 8 bytes early and `sf_open` overflows the thunk's struct buffer by 12 bytes

Last updated: 2026-07-19
Effort: small (<1h)
Severity: HIGH
Class: Memory corruption + silent wrong values

Status: FIXED (2026-07-19)
Regression Test: `tests/rt-behavior/native/libsnd-open-file-info-rt`

## Resolution

`bindings/libsnd/src/lib.mfb` now declares `frames` in both the `FileInfo` record
and the `SfFileInfo` `CSTRUCT`, so `compute_c_layout` derives the real 32-byte,
align-8 layout. Both companion defects in §Also in this file are fixed in the same
change: `errMessage` binds `sf_error_number`, and `sndError`'s prefix reads
`"libsnd: "`. Manifest `version` bumped `1.2.2` → `1.2.3` and `libsnd.mfp`
regenerated.

The regression test is a genuine discriminator, proven both ways on
macos-aarch64. With the corrected layout it reads, off a real libsndfile handle
opened on a hand-written 4-frame stereo WAV:

```
frames=4  samplerate=8000  channels=2  format=65538  sections=1  seekable=1
```

With `frames` stripped back out (the pre-fix declaration, run from a scratch
copy) the same probe prints exactly the shifted values this report predicted:

```
samplerate=4  channels=0  format=8000  sections=2  seekable=65538
```

`channels=0` is the high half of `frames`, confirming the discriminator argument
in the Validation Plan — a mono probe could not have told the layouts apart.
`errMessage(1)` returns libsndfile's real `"Format not recognised."` rather than
the result of dereferencing an int.

Two Validation Plan items did not apply as written:

- **The syntax fixtures needed no change.** `native-bind-state-valid` and
  `native-bind-state-wrong-resource` already carried the correct `SfFileInfo`
  (as §The in-tree fixture already has it right observed), so there was no golden
  churn to review.
- **There is no `openFile` DOC block to update.** `openFile`, `FileInfo` and
  `SoundFile` are package-internal — only `getFormats` and `AudioFormat` are
  `EXPORT`ed — so `PROP frames` has no page to land on. `plan-58-D` is what makes
  this surface public, and it owns the DOC work.

The test vendors the same libsndfile blobs `bindings/libsnd` ships, by symlink
rather than by copy, so there is no duplicated binary in the tree and no way for
the test's library to drift from the binding's.

`bindings/libsnd/src/lib.mfb:78` declares libsndfile's `SF_INFO` **without its
leading `sf_count_t frames` field**. The real struct opens with an 8-byte
`frames`; the binding starts at `samplerate`.

Two consequences, both live on every `libsnd::openFile` call:

1. The `CSTRUCT` lays out at **20 bytes** where libsndfile writes **32**, so
   `sf_open` writes 12 bytes past the end of the marshaling thunk's stack buffer.
2. Every field reads from the wrong offset, so `.state` reports plausible but
   wrong numbers.

## The authority

`/Users/justinzaun/local/brew/include/sndfile.h:379-388`:

```c
struct SF_INFO
{	sf_count_t	frames ;   /* int64_t — sndfile.h:368 */
	int			samplerate ;
	int			channels ;
	int			format ;
	int			sections ;
	int			seekable ;
} ;
```

Real layout: 32 bytes, align 8 — `frames@0(8) samplerate@8 channels@12 format@16
sections@20 seekable@24`.

## What the binding declares

```basic
  CSTRUCT SfFileInfo AS FileInfo   ' bindings/libsnd/src/lib.mfb:78
    samplerate CInt32
    channels   CInt32
    format     CInt32
    sections   CInt32
    seekable   CInt32
  END CSTRUCT
```

Laid out by `compute_c_layout` (`src/ir/link.rs:135-156`): 20 bytes, align 4 —
`samplerate@0 channels@4 format@8 sections@12 seekable@16`.

So each declared field reads the *previous* real field:

| declared field | reads offset | actually holds |
|---|---|---|
| `samplerate` | 0 | low 32 bits of `frames` |
| `channels` | 4 | high 32 bits of `frames` — `0` for any file under 4G frames |
| `format` | 8 | the real `samplerate` |
| `sections` | 12 | the real `channels` |
| `seekable` | 16 | the real `format` |

And the 12-byte overflow: the thunk stages the struct in its own frame at
`struct_cursor` (`src/target/shared/code/link_thunk.rs:375-391`), sized from
`CLayout.size` = 20. `sf_open` writes 32. The excess lands in whatever the frame
layout placed next — `cstr_area`, `cursor_off`, `rec_handle_off`
(`link_thunk.rs:402-414`).

## The in-tree fixture already has it right

`tests/syntax/native/native-cstruct-valid/src/lib.mfb:28-35` declares the same
struct **correctly**, with the comment:

> `' 32 bytes, align 8: channels lands at 12, which a naive one-word-per-field`
> `' record layout would put at 16.`

So the test agrees with the header, and only the shipped binding disagrees. This
is not a case of a stale assertion — it is a case of the assertion never being
applied to the real binding.

## Why the suite did not catch it

- **Goldens carry field names and ctypes, never offsets.** `.mfp` encoding
  deliberately transports only the ctypes and recomputes layout on decode
  (`src/ir/binary.rs:282-293`), so a wrong field list round-trips cleanly through
  every AST/IR/package golden.
- **No test calls `libsnd::openFile` against real libsndfile.**
  `tests/rt-behavior/resources/resource-state-{return,import}-rt` only *mention*
  libsnd in comments (`resource-state-return-rt/src/main.mfb:10-11`) and actually
  use `fs::openFile`; `tests/syntax/native/native-bind-state-{valid,wrong-resource}`
  and `native-cstruct-valid` are syntax-only and never execute.
- plan-53's runtime proof used a **stand-in**, not the real library
  (`planning/old-plans/plan-53-C-libsnd-integration.md`).

## Fix

Add the missing field to both the `CSTRUCT` and its mapped record — coverage must
be total (`NATIVE_STRUCT_FIELD_MISMATCH`, `src/ir/link.rs:245-265`), and
`compute_c_layout` derives the corrected offsets with no compiler change:

```basic
TYPE FileInfo
  frames     AS Integer          ' NEW — sf_count_t, int64_t
  samplerate AS Integer
  channels   AS Integer
  format     AS Integer
  sections   AS Integer
  seekable   AS Integer
END TYPE

  ' 32 bytes, align 8: frames@0, samplerate@8, channels@12, format@16,
  ' sections@20, seekable@24. The leading sf_count_t is load-bearing — without
  ' it every field reads 8 bytes early and the callee writes 12 bytes past the
  ' thunk's struct buffer (bug-364).
  CSTRUCT SfFileInfo AS FileInfo
    frames     CInt64            ' NEW
    samplerate CInt32
    channels   CInt32
    format     CInt32
    sections   CInt32
    seekable   CInt32
  END CSTRUCT
```

Field order in the `CSTRUCT` is load-bearing (it drives offsets); order in the
record is not (matching is by name) — keep them parallel for readability.

Rejected: declaring `frames` as two `CInt32` halves to avoid changing the
record's field count. `sf_count_t` is `int64_t`; `CInt64` models it exactly, and
splitting it would misreport any file over 2^31 frames.

Rejected: keeping `frames` `CSTRUCT`-only and out of the record. Not possible —
coverage is total by rule, and `plan-58-D` needs `frames` to size a PCM buffer.

## Also in this file

Two further defects found in the same review, both confirmed against the header.
Fix them here or as their own change, but do not leave them:

- **`errMessage` binds the wrong symbol** (`lib.mfb:130-134`). It declares
  `SYMBOL "sf_error"`, but:
  ```c
  int         sf_error        (SNDFILE *sndfile) ;   /* sndfile.h:619 */
  const char* sf_error_number (int errnum) ;         /* sndfile.h:634 */
  ```
  With `ABI (errNum CInt32) AS message CPtr` it passes an **integer where
  `sf_error` expects a `SNDFILE *`**, then marshals the returned **`int`** as a
  `CPtr` to copy out as a String — two wild dereferences per call, on the error
  path. Repoint at `sf_error_number`. The returned pointer is library-owned
  static storage, so copy-and-leave is correct and **no `FREE` block** is wanted.
- **`sndError` reports the wrong package** (`lib.mfb:139`): the message prefix is
  `"sqlite3: "`, copied from `bindings/sqlite3`. Should be `"libsnd: "`.

## Validation Plan

- **Regression test**: new `tests/rt-behavior/native/libsnd-open-file-info-rt/`.
  Generate a **stereo** WAV of known frame count at run time (hand-write the
  44-byte RIFF header via `fs::writeAllBytes`; do not commit a binary fixture),
  open it with `libsnd::openFile`, and print all six `.state` fields.
  **`channels` is the discriminator** — pre-fix it reads the high half of
  `frames`, i.e. `0`. A mono or silent probe cannot distinguish the two layouts.
- **Error path**: force a failure (open a nonexistent file) and assert the
  message is libsndfile's real text, which today dereferences an integer.
- **Fixtures**: update `tests/syntax/native/native-bind-state-valid/src/lib.mfb`
  and `native-bind-state-wrong-resource/src/lib.mfb` to carry the corrected
  `SfFileInfo`, so no in-tree fixture teaches the wrong layout. Re-sync their
  `.ast` goldens **only after** confirming the diff is exactly the added field.
- **Runtime proof is required** (`.ai/compiler.md` Hard Completion Gate): reading
  correct values off a real libsndfile handle is the only proof, because goldens
  structurally cannot carry offsets. Run on macOS/aarch64 and
  Linux/{aarch64,x86_64,riscv64} × {glibc,musl} per `.ai/remote_systems.md`.
- **Manifest**: bump `bindings/libsnd/project.json` `version` (`1.2.2` → `1.2.3`)
  — the exported `FileInfo` record gains a field, a visible API change for any
  importer reading `.state`. Update the `openFile` DOC block and add
  `PROP frames`.
- **Acceptance**: `scripts/test-accept.sh target/debug/mfb target/accept-actual`,
  with golden churn confined to the two syntax fixtures and the new test.

## Notes

There is no general compiler-side guard available here: the compiler has no C
header, so it cannot know a `CSTRUCT` is smaller than the struct it models, and
for an `OUT` slot the callee's write size is unknowable. The defense is a runtime
test per binding, which is what this bug adds. Do not reopen the question of a
static check without new information.

Blocks `planning/plan-58-D-libsnd-loadsound.md`, which sizes its PCM buffer from
`frames * channels`.
