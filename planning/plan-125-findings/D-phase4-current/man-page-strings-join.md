Probes all passed. Writing up the single finding now.

### 1. Errors table row has no condition in the prose
UNIT:      man-page:strings/join
CLAIM:     "Neither argument is mutated; the result is a new String. parts is also accepted under the name values, and delimiter under the name separator." (the description's last paragraph; nowhere on the page is `ErrOutOfMemory` explained)
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/strings/func_join.rs:register` lists `errors: vec!["ErrOutOfMemory"]`, and `lower` raises it in two places: when the arena allocation of the result fails, and when the summed length of all elements plus delimiters overflows a 64-bit size (`emit_checked_size_add` → `overflow` label). The rendered page (`mfb man strings join`) shows the row in the Errors table but no sentence in `desc` states the condition, which `.ai/man-content.md` §2 requires ("Every raisable error and the condition that raises it"). Not reproducible at runtime in a sandbox (would need to exhaust memory), so the condition is taken from the lowering, not a probe.
SUGGESTED: Add to the final paragraph: "The only error `join` can raise is `ErrOutOfMemory`, when the combined length of all elements and delimiters is larger than the program can hold."

Everything else on the page checked out. All three examples built and ran under the release compiler from scratch projects and printed exactly what the page implies:

```
Hello World
abc
left--right
[]
left/right
```

The extra probe program (`/tmp/plan-125-scratch/D-phase4-current/man-page-strings-join/p_probe/src/main.mfb`) confirmed the alternate names `values :=` and `separator :=` work, a single-element list yields that element with no delimiter, all-empty elements still take delimiters (`[,,]`), multi-byte UTF-8 passes through verbatim, the split/join round trip holds for leading, trailing and consecutive delimiters and for a multi-character delimiter (`[-a--b-]`, `[a::b::::c]`, `[]`, `[abc]`), and the list and delimiter are unchanged after the call. The split round trip with an empty delimiter raised 7-705-0002 from `split`, which is why the page's "for a non-empty delimiter" qualifier is correct as written. No banned memory vocabulary appears on the rendered page, and the only occurrence of "Allocation failed." is the derived error message covered by the §4.4 carve-out.
