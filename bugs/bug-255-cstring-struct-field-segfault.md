# bug-255: a `CSTRUCT` with `CString` fields segfaults at runtime

Last updated: 2026-07-16
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: — (the failing repro is below; it is NOT committed as a test,
because a committed failing test would make the suite red)

plan-50-F implemented `const char *` struct fields, and they compile, verify, and
emit. But a wrapper whose struct slot has `CString` fields **segfaults when
called**. `bindings/libsnd`'s `getFormats()` — the whole point of plan-50 — hits
it.

## Repro

```
$ cd bindings/libsnd && mfb build          # builds fine
```

Then an importer calling `getFormats()`, or the smaller probe:

```basic
' In the binding:
EXPORT FUNC probeOne(i AS Integer) AS AudioFormat
  RETURN sndLink::getFormat(i)
END FUNC
```

```basic
IMPORT io
IMPORT libsnd
FUNC main AS Integer
  io::print("calling")
  LET f = libsnd::probeOne(0)          ' <-- SIGSEGV here
  io::print("format=" & toString(f.format))
  RETURN 0
END FUNC
```

```
calling
[exit 139]
```

The crash is `EXC_BAD_ACCESS` at `ldr x11, [x10]` / `add x11, x11, #0x9` — the
`len + 9` of a **String copy**, dereferencing a garbage pointer. The faulting
address is different every run (0xa16515bc065ba848, 0x4f98035915cf9678), so it is
uninitialized memory, not a fixed bad offset. The record's `String` field holds
garbage by the time the caller copies it.

## What is already ruled out

This is a narrow fault, not "struct marshaling is broken":

- **Scalar struct fields work.** `tests/rt-behavior/native/native-struct-scalar-rt`
  passes: `clock_gettime` fills a 16-byte `timespec` (OUT), read back as a record,
  and the values match C exactly (`sec=1784257884` from both, same second).
  `nanosleep` accepts a struct built by `BIND IN` (IN direction).
- **The scalar OUT path works.** `getFormatCount()` returns `17` through
  `count OUT CInt32`, so libsndfile loads, the dlopen/dlsym initializer runs, and
  `sf_command` is called correctly.
- **The emitted thunk reads the right offsets.** From `-ncode` on the importer:
  the buffer address handed to C (`add_imm x8, sp, #96`) matches the bytes zeroed
  (`str xzr, [sp,#96/#104/#112]` — 24 bytes); `format` is read with `ldr_u32
  [sp,#96]`, sign-extended, and stored to `record[0]`; the copied String is stored
  to `record[8]`; `extension`'s `char *` is read from `[sp,#112]`. The frame is
  rebased by +16 for the spill area, **uniformly** — the `add_imm sp` and the
  loads/stores agree, so that is not the fault.
- **The save-slot/register question.** `emit_copy_cstring_to_string`'s NULL path
  originally left its result only in `RESULT_VALUE_REGISTER` and never wrote
  `ret_off`. That was fixed (both paths now store to the slot, and the caller
  reads the slot rather than trusting a physical register across a label) — the
  crash survives the fix, so it was a real latent bug but not this one.

## Where to look next

1. **The record's ownership/copy contract for `String` fields.** The scalar case
   proves the `8*i` record layout is right. What differs here is that a field is
   an owned arena value. A record returned across a package boundary is copied by
   the caller (`collections::append` deep-copies; the crash is inside that copy),
   so the question is whether a `String` the thunk allocated is a legal record
   field value at that moment — e.g. whether copy-insertion/scope-drop expects
   something the thunk does not establish.
2. **Arena state across the two allocations.** `marshal_struct_out` allocates the
   record, then allocates once per `CString` field. Check `_mfb_arena_alloc`'s
   contract when a second allocation happens while an earlier block is live but
   referenced only from a stack slot.
3. **The `-ncode` dump is the fastest instrument.** `mfb build -ncode <importer>`
   and read `_mfb_linker_sndLink_getFormat`; the labels are `..._sf<N>_*` per
   field.

## Why it is filed rather than fixed

plan-50-A/B/C/D/E/F/H/I are landed and green (964 acceptance tests). This is the
last gap, and it is specific enough to name precisely. Committing the libsnd
runtime test in a failing state would make the suite red for everyone, so the
test is held back with this document instead. The binding source, the manifest,
and the `.mfp` are committed and build — only the runtime behavior is wrong.
