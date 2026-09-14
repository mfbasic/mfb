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

- **Every other `encoding` decoder has the same defect**, audited by plan-125-C
  Phase 2. `grep -n 'errors: vec!' src/codegen/builtins/encoding/func_*decode*.rs`
  finds **12** decoders declaring `errors: vec![]`. Only `codepageDecode` and
  `punycodeDecode` declare `ErrInvalidFormat`. Each of the 12 is confirmed to
  raise `77050003`:

  | Decoder | Evidence it raises `ErrInvalidFormat` |
  |---|---|
  | `base64Decode` | 2 `FAIL error(77050003, …)` in its body; probe `base64Decode("QQ")` → `77050003` |
  | `base32Decode` | 4 in its body; probe `base32Decode("A=======")` → `77050003` |
  | `base64UrlDecode` | 1 in its body plus the shared `__encoding_base64Symbols` |
  | `hexDecode` | 2 in its body; probe `hexDecode("zz")` → `77050003` |
  | `percentDecode`, `formUrlDecode` | the shared `__encoding_percentDecodeBytes`; probe (this bug's reproduction) |
  | `uleb128Decode` | 3 in its body |
  | `sleb128Decode` | 3 in its body |
  | `varintDecode` | through `__encoding_uleb128Decode`; probe `varintDecode([128])` → `77050003` |
  | `utf8Decode` | `helper_utf8_decode.rs`; probe `utf8Decode([255])` and `utf8Decode([195])` → `77050003` |
  | `utf16Decode` | 4 in its body; probe on a lone surrogate `[0xD800]` → `77050003` |
  | `utf32Decode` | 2 in its body |
  | `htmlUnescape` (not named `*Decode`, same defect) | 2 `FAIL error(77050003, …)` in its body: probes `&#;`, `a &amp b`, `&#1114112;` and `&nosuch;` → `77050003`. It also raises **`ErrEncoding`** for a surrogate reference (`&#55296;` → `77020004`); that error is not declared either (`/tmp/p125-ex/enchtml`) |

  Probes: `/tmp/p125-ex/encclaims`, `/tmp/p125-ex/encb32`, `/tmp/p125-ex/utf8err`.
  In the meantime, the man pages of the decoders reviewed so far name
  `ErrInvalidFormat` in their Description prose.
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
