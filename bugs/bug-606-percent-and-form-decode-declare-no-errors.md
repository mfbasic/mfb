# bug-606: `encoding::percentDecode` and `encoding::formUrlDecode` raise `ErrInvalidFormat` but declare no errors

Last updated: 2026-09-13
Effort: small (<1h)
Severity: LOW
Class: Correctness (registry error declaration) / Documentation

Status: Open
Regression Test: none yet — see Phase 1

`encoding::percentDecode("%")` and `encoding::formUrlDecode("%")` fail with
`ErrInvalidFormat` (`77050003`), and the package overview promises it
("Decoders reject malformed input with `ErrInvalidFormat`"). But both
descriptors declare `errors: vec![]`, so:

- their man pages render **no Errors section** (`mfb man encoding percentDecode`);
  a developer reading the page cannot learn which error to trap;
- every consumer of the declared-error list sees them as infallible. Per
  `mfb spec language error-model` §8.6 rule 11, "a member is infallible here only
  if it declares no error AND raises none — the verdict is read off the member's
  declared errors, so a member that raises an error it does not declare is
  wrongly proved infallible". Today these two are MFBASIC-source members, not
  inline-lowered, and the probe below shows a handler still runs; the declaration
  is the latent hazard.

Its sibling `encoding::codepageDecode` declares `ErrInvalidFormat` and renders it.

**The single correct behavior a fix produces:** both descriptors declare
`ErrInvalidFormat`, so each page's Errors table lists it and no analysis can read
either member as infallible.

Found by plan-125-B Phase 3's Codex review of `encoding`
(`planning/plan-125-findings/B-phase3/man-pkg-encoding.md`, finding 1). The
review's suggestion was to add the error to the descriptor; that edits compiler
data, not prose. **Filed, not fixed**, by user instruction during a
documentation-only plan ("file all bugs, make no fixes").

## Reproduction

```basic
IMPORT io
IMPORT encoding

SUB main()
  LET a = encoding::percentDecode("%") TRAP(e)
    io::print("percent caught " & toString(e.code))
    RECOVER "fallback"
  END TRAP
  LET b = encoding::formUrlDecode("%") TRAP(e2)
    io::print("form caught " & toString(e2.code))
    RECOVER "fallback"
  END TRAP
END SUB
```

Observed (macos-aarch64, `worktree-P-125` HEAD): builds with no warning, prints
`percent caught 77050003` and `form caught 77050003`.

```
mfb man encoding percentDecode | grep -c 'ErrInvalidFormat'   # the Errors table is absent
grep -n 'errors: vec!' src/codegen/builtins/encoding/func_percent_decode.rs src/codegen/builtins/encoding/func_form_url_decode.rs
```

Expected: an Errors table with the `ErrInvalidFormat` row on both pages.

## Root cause

`src/codegen/builtins/encoding/func_percent_decode.rs:register` and
`func_form_url_decode.rs:register` declare `errors: vec![]`, while their bodies call
the shared rejecting percent decoder, which raises `ErrInvalidFormat`.

## Non-goals

- Changing what either function accepts or rejects.
- Rewording the page prose to hide the error.

## Blast-radius audit

- Every other `encoding` decoder: `grep -n 'errors: vec!' src/codegen/builtins/encoding/func_*decode*.rs`
  — audit each against its body in Phase 1. `codepageDecode` already declares it.
- Other packages: a member whose body can raise an error its descriptor does not
  declare is the same class. A census that walks each descriptor's body for
  `FAIL`/raising calls against its `errors` list is out of scope here and
  recorded for the fixer.

## Fix

Phase 1 — a test that `percentDecode` and `formUrlDecode` declare
`ErrInvalidFormat` (RED), plus the decoder audit above. Commit:

Phase 2 — add `ErrInvalidFormat` to both descriptors (GREEN); regenerate and check
any golden that carries the error list (memory: registry description drifts `.ir`
goldens); full suite. Commit:
