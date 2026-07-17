# bug-255: a `CSTRUCT` with `CString` fields segfaults at runtime

Last updated: 2026-07-16
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Fixed
Regression Test: `tests/rt-behavior/native/native-struct-cstring-rt`

plan-50-F implemented `const char *` struct fields, and they compiled, verified,
and emitted. But a wrapper whose struct slot had `CString` fields **crashed when
called**. `bindings/libsnd`'s `getFormats()` — the whole point of plan-50 — hit
it.

## Root cause

plan-50-F was built on a wrong model of records. It assumed a record field of
type `String` holds a **pointer** to a String block, so `marshal_struct_out`
allocated `8 * field_count` bytes, copied each `const char *` into its own arena
String, and stored that pointer at `8*i`.

A record does not work that way. Per `record_field_is_inlined`
(`builder_collection_layout.rs:586`), only `Address`, `Datagram`,
`DatagramText` and `AudioDevice` keep pointer strings — **every other record
INLINES its `String` fields**, and a `CSTRUCT` can map to none of those four. The
real layout, from `emit_build_inlined_record`:

- the fixed area is `8 * field_count`;
- a `String` field's word at `8*i` is **not a pointer** — it is the offset,
  relative to the record's own block, of an inlined `{len, bytes, NUL}`
  sub-block;
- those sub-blocks live contiguously in a trailing data region, each 8-aligned,
  each `len + 9` bytes.

So the thunk wrote a pointer where the caller expected an offset, and allocated
no data region at all. The caller then walked the missing region
(`emit_record_block_size_to_slot` sizes a record by reading each inlined block's
length **contiguously**, ignoring the stored offsets), read garbage as a length,
and added 9 — which is precisely the reported faulting instruction:

```
ldr x11, [x10]        ; garbage "length"
add x11, x11, #0x9    ; + 9  -> the String block size
```

Both symptoms follow: a garbage length that is huge fails the allocation
("Allocation failed", 7-701-0001), and one that is merely wrong dereferences
wild memory (SIGSEGV at a different address every run).

## The fix

`marshal_struct_out` now builds a real inlined record, mirroring
`emit_build_inlined_record`:

1. **Pass 0 — measure.** `strlen` every `CString` field, stash `[char*, len]` per
   field, and validate the bytes as UTF-8 (§12.4) before anything is copied. A
   NULL `char *` becomes the empty String (len 0), not a crash.
2. **Pass 1 — size and allocate.** `total = 8*n`, then for each `CString` field
   `total = align8(total) + len + 9`. **One** allocation, because every length is
   known before it.
3. **Pass 2 — write.** Scalars go to `8*i`; each `CString` field writes the
   block-relative cursor to `8*i`, then `{len, bytes, NUL}` at `record + cursor`,
   then advances the cursor.

This also removes the per-field allocation entirely, so the old "record pointer
must survive N allocations" hazard is gone.

## Why it took a while to see

Every narrower hypothesis was consistent with the evidence and wrong:

- **Scalar struct fields worked** (`native-struct-scalar-rt`, `clock_gettime`
  into a `timespec`, matching C exactly) — because a scalar-only record has no
  data region, so `8*n` is exactly right. That is what made the layout look
  proven.
- **The pointer in the C struct was valid.** A probe declaring the same 24-byte
  `SF_FORMAT_INFO` with `CInt64` in place of the two `CString`s returned real
  pointers (`4344360535` / `4344360563`, 28 apart — adjacent statics), and read
  the real format code. So the offsets, the buffer, `INOUT`, `BIND IN` and
  `SIZEOF` were all correct.
- **The emitted thunk read the right offsets**, confirmed against `-ncode`.
- The frame was correct: 224 bytes, max slot touched 216. (An earlier
  `add_sp #192` that suggested an overflow was a *different function's*
  epilogue.)

Two measurements broke it open, both by ruling out the size rather than
inspecting more code:

1. Clamping the length to a constant 8 still failed — so the **allocation
   itself** was failing, not a huge `strlen`.
2. Skipping the `CString` copy entirely, storing 0 in the field, **still**
   failed — so a record with a `String` field was broken *regardless of what the
   field contained*, which pointed at the record's shape rather than the copy.

## Verified

`bindings/libsnd`'s `getFormats()` returns 17 correct formats through the real
libsndfile:

```
count=17
aiff | AIFF (Apple/SGI 16 bit PCM)
wav  | WAV (Microsoft 16 bit PCM)
flac | FLAC 16 bit
...
```
