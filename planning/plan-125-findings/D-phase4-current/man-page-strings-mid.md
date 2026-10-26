### 1. Errors table omits `ErrOutOfMemory`, which the String lowering raises
UNIT:      man-page:strings/mid
CLAIM:     The Errors table lists only `77050001 ErrIndexOutOfRange` (and the description's "Violating that raises ErrIndexOutOfRange" is the page's only error statement).
VERDICT:   incomplete
EVIDENCE:  `src/codegen/collection/search/builder_search/mid.rs:lower_mid`, String branch: after `emit_arena_alloc_call()` it emits `self.raise_error_bare("ErrOutOfMemory")?` before `mid_alloc_ok`, then later `raise_error("strings.mid", "ErrIndexOutOfRange")`. The descriptor `src/codegen/builtins/strings/func_mid.rs` declares `errors: vec!["ErrIndexOutOfRange"]` only, so the derived table drops the allocation failure. The sibling `src/codegen/builtins/strings/func_left.rs` declares `errors: vec!["ErrInvalidArgument", "ErrOutOfMemory"]` and `mfb man strings left` renders `77010001 ErrOutOfMemory` in its table, so the surface convention is to list it. Per `.ai/man-content.md` §0 this is a descriptor bug, not a prose fix: "if a derived table is wrong, the descriptor is wrong, and that is a code bug." The raise itself could not be triggered by a probe (no way to exhaust the arena from a scratch program), so the runtime path is UNVERIFIED by execution and CONFIRMED by source.
SUGGESTED: In `func_mid.rs`, change the descriptor to `errors: vec!["ErrIndexOutOfRange", "ErrOutOfMemory"]` so the Errors table renders both rows, matching `strings::left` and `strings::right`. No prose change is needed; the page's description already covers the only developer-controllable condition.

Everything else on the page verified true against the binary. Probes under `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-mid` (built with `mfb init` + `mfb build -q`):

- All three page examples compile and print `ell` / `[]`, `😀é`, and `out of range`, exactly as the page implies.
- Edge probe: `("",0,0)` → `[]`; `("",1,0)`, `("",0,1)`, `("hello",6,0)`, `("hello",0,6)`, negative start, negative count, and both 64-bit overflow sums all raise `77050001 List or string index/range is outside valid bounds.`; `("hello",5,0)` → `[]`; `("hello",0,5)` → `[hello]`. Start bound is inclusive and `start + count` bound is inclusive, as the parameter descriptions say.
- Grapheme cut: `mid("e\u0301x", 0, 1)` → `e` (combining mark dropped), `mid(…, 0, 2)` → `é`. The page's "can cut a cluster in half" is true.
- Source value unchanged after the call (`orig after: hello part: el`).
- AttributedString overload via `strings::mid`: bold over `world` on `hello world`, `mid(styled,4,4)` → `o **wo**`, `mid(styled,6,5)` → `**world**`, `mid(styled,11,0)` → `[]`, `mid(styled,6,6)` raises `77050001`. Spans are remapped as claimed.
- `MFB=… ./scripts/man-census.sh --memory-scope strings` → `unclassified memory-vocabulary hits: 0`. No banned vocabulary and no sentence teaching a borrow/ownership model; "new String" and "not mutated" are within the permitted four words.
- `mfb man collections mid` renders and agrees (validated, not clamped, `ErrIndexOutOfRange`), so the cross-reference is sound.
