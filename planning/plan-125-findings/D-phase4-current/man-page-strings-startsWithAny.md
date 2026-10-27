Probe done: every behavioral claim on the page checked out. Writing up the result.

### 1. Short-circuit sentence describes nothing a program can observe
UNIT:      man-page:strings/startsWithAny
CLAIM:     "Candidates are tested in list order and the scan stops at the first match; which candidate matched is not reported, only that one did."
VERDICT:   out-of-scope (minor)
EVIDENCE:  `gen_with_any.rs:lower_strings_with_any` does scan in order and branch to `true_label` on the first byte-range match, so the sentence is true. But the result is a `Boolean`, candidates are plain `String` values with no side effects, and the call never fails, so list order and early exit have no effect a developer can see. The only consequence is speed, which `.ai/man-content.md` §4.3 says to state as "cheap" or a complexity, not as a mechanism. The second half ("which candidate matched is not reported") is the useful, developer-facing part and should stay.
SUGGESTED: "Which candidate matched is not reported, only that one did; the order of `prefixes` does not affect the result."

No other findings. Everything else on the page is true of the binary at HEAD:

- **Examples compile and run.** Both were copied verbatim into `mfb init` projects under `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-startsWithAny/p_ex1` and `p_ex2`, built with the P-125 release `mfb build -q`, and printed `TRUE` and `FALSE` respectively, matching the page's prose.
- **Probe program** (`p_probe/src/main.mfb`, same build) printed:
  ```
  withEmpty on 'abc': TRUE            ("" candidate matches everything)
  withEmpty on '': TRUE               (empty value, "" candidate)
  longer candidate on 'abc': FALSE    (longer candidate skipped, no error)
  empty list on '': FALSE             (empty list → FALSE)
  case 'ABC' vs abc: FALSE            (no case folding)
  decomposed starts with e: TRUE      (e + U+0301 begins with "e")
  precomposed starts with e: FALSE    (U+00E9 does not)
  decomposed starts with precomposed: FALSE (the two spellings never match)
  attributed: TRUE                    (AttributedString overload, same result)
  list after: 2                       (list not modified)
  ```
- **Errors table** is empty and the lowering has no raise path: `lower_strings_with_any` is a pure byte-compare loop ending in `emit_string_predicate_result`, so "the call is total" is correct.
- **Scope and vocabulary.** No banned words, no borrow/ownership framing. "Neither value nor the list is modified" uses permitted vocabulary.

One note for the main thread, not a page finding: the worktree's source at `src/codegen/builtins/strings/func_starts_with_any.rs` still ends the description with "(same value, type, and errors)", while the P-125 binary renders the newer wording "the same value and type, and like it, it never fails". The rendered wording is the better one. The two trees have drifted and the review above is of the rendered page.
