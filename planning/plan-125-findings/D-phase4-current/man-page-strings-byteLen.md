The edge probe passed on every claim (empty 0, 2/3/4-byte scalars, AttributedString overload matches, no mutation of the input, 1,000,000-byte string). Writing up the result.

### 1. "does not walk the text" describes the implementation, not the behavior
UNIT:      man-page:strings/byteLen
CLAIM:     "The answer is immediate however long the string is — byteLen does not walk the text."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/strings/func_byte_len.rs:lower` — a single `abi::load_u64` of the count word. The developer-visible fact is the cost (constant, independent of length), which the first half of the sentence already states; the second half tells the reader *how* the compiler achieves it, which is a §1/§3 implementation detail. §4.3's own rule for speed claims is "say cheap or give the complexity", not describe the mechanism. Minor; every other sentence on the page verified true.
SUGGESTED: "The answer takes the same, constant time however long the string is."

Everything else verified against the binary at `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb`:

- Both page examples, copied verbatim into `mfb init` projects under `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-byteLen/p_ex1` and `p_ex2`, compile and print `5`/`4` and `7`/`3`, matching what the prose implies (one byte per ASCII scalar; byte length ≥ `len`).
- Edge probe `p_edges` printed: `empty=0`, `two=2` ("é"), `three=3` ("€"), `four=4` ("😀"), `combining len=1 bytes=2 graphemes=1`, `after s=héllo b=6` (input unchanged), `attr=7` and `attrbold=7` (AttributedString overload via `is_tier_a_query` in `strings/mod.rs` returns the same Integer as the String overload, styling ignored), `long=1000000`, `toBytes count=7` (agrees with byteLen).
- Errors: descriptor declares `errors: vec![]`, the page renders no Errors section, and the lowering has no raising path. Consistent.
- Parameter description covers the empty case; there is no numeric range to bound.
- No banned memory vocabulary and no borrow/ownership model taught; "inspects value only: it changes nothing" uses only permitted terms.
