### 1. ctz claims clz special-cases negative values; clz does no such thing
UNIT:      man-pkg:bits
PAGE:      bits::ctz (and bits::clz)
CATEGORY:  consistency
CLAIM:     "Negative values are not special-cased the way they are for bits::clz: bits::ctz(-1) is 0 (every bit is set), while bits::ctz(-2) is 1."
VERDICT:   inconsistent
EVIDENCE:  `grep -n -i "negative\|special" src/codegen/builtins/bits/func_clz.rs` prints nothing; the clz page says only "value is treated as a raw two's-complement 64-bit Integer bit pattern; clz does not interpret sign ... When bit 63 is set the result is 0." `lower_bits_clz` in func_clz.rs emits a single `abi::count_leading_zeros` with no sign branch. Probe (probe1): `clz(-1)=0`, `ctz(-1)=0`, `ctz(-2)=1` — both functions apply the same plain bit-pattern rule. A reader of ctz is sent to clz looking for a negative-value special case that does not exist.
SUGGESTED: "Negative values follow the same rule as any other bit pattern: bits::ctz(-1) is 0 (every bit is set), while bits::ctz(-2) is 1." Drop the comparison to clz (or make it accurate: "as with bits::clz, a negative value is just a bit pattern with bit 63 set").

### 2. The unit never says how to write a mask with bit 63 set, and the rl64 example only compiles by accident
UNIT:      man-pkg:bits
PAGE:      package-wide (examples on bits::rl64; relevant to bnot, sra, sl, rr64, bswap64)
CATEGORY:  coverage
CLAIM:     rl64 example: `LET rotated AS Integer = bits::rl64(0xFF00000000000000, 8)`
VERDICT:   missing
EVIDENCE:  probe4 (first version): `LET x AS Integer = 0xFF00000000000000` → `error[2-203-0009 TYPE_INTEGER_LITERAL_OVERFLOW]: Integer literal 18374686479671623680 is outside the Integer range`. The same literal passed inline as a call argument compiles and runs (probe1: `rl64(0xFF00000000000000,8)=255`), and `toString(0x8000000000000000)` prints `9223372036854775808` (unsigned) while `toString(-0x8000000000000000)` prints `-9223372036854775808`. `src/docs/spec/language/04_types.md:622` says any integer-looking literal that does not parse as i64 is rejected. So the one place a developer will copy the idiom from (the rl64 example) uses a form that fails the moment they hoist it into a LET, and no page in the unit says how to write a 64-bit mask with the top bit set (negative literal, `bits::bnot` of the low-bit mask, or `bits::sl(1, 63)`). The inline acceptance looks like a compiler inconsistency with the spec; the main thread should file it as a bug rather than document it.
SUGGESTED: In the overview, after the "raw two's-complement" paragraph, add: "An Integer literal must fit in a signed 64-bit value, so a mask with bit 63 set cannot be written as 0x8000000000000000. Write it as bits::sl(1, 63), as bits::bnot of the complementary mask, or as a negative decimal literal." Change the rl64 example to build its operand that way (e.g. `bits::sl(0xFF, 56)`). File the literal-range bug separately.

### 3. Two bswap pages invent a term ("variable-shift ops") for what every other page calls the shifts
UNIT:      man-pkg:bits
PAGE:      bits::bswap32, bits::bswap64 (vs overview, bswap16, rotates)
CATEGORY:  consistency
CLAIM:     "never raises; only the variable-shift ops (sl/sr/sra) can raise a bits:: error"
VERDICT:   inconsistent
EVIDENCE:  `grep -rn "variable-shift" src/codegen/builtins/bits/` → only `func_bswap32.rs:21`, `func_bswap64.rs:23`, and a Rust module comment in `mod.rs:18`. The overview says "Shifts (sl, sr, sra) validate their count argument", every rotate page says "Unlike the bits shifts (sl/sr/sra)", and bswap16 (the direct sibling) omits the clause entirely. "variable-shift" is lowering vocabulary that leaked from the module doc comment; `bits::` with a trailing double colon is also not how any other page names the package's errors.
SUGGESTED: Use the same sentence the other 12 total pages use: "The operation is total — it is defined for every Integer and never raises — and has no side effects." If a pointer is wanted, "Only the shifts bits::sl, bits::sr and bits::sra can raise."

### 4. ctz carries a per-architecture cost warning that contradicts the package framing and is a compiler fact
UNIT:      man-pkg:bits
PAGE:      bits::ctz (vs overview)
CATEGORY:  overview-mismatch
CLAIM:     "it is the one function here whose cost varies noticeably between them: it is cheap on arm64 and x86-64 and markedly more expensive on RISC-V, which lacks the operations it is built from. On a hot RISC-V path ... prefer bits::band(value, -value) and a comparison over calling ctz."
VERDICT:   misleading
EVIDENCE:  Overview: "These are primitive operations, like math::abs: they have no side effects and give the same answer on every platform." `grep -rln -i "expensive\|RISC-V\|cheap" src/codegen/builtins/bits/` → only func_ctz.rs; no other page in the unit discusses cost, so a reader has no way to know whether the silence on the other 16 pages means "cheap everywhere" or "nobody checked". `lower_bits_ctz` (func_ctz.rs) emits `abi::reverse_bits` + `abi::count_leading_zeros`, which is an implementation detail per `.ai/man-content.md` §3 ("Lowers to a single native ... instruction is a compiler fact"). The relative-cost claim itself is UNVERIFIED here: no RISC-V machine in this sandbox, and `.ai/arch-abi.md` is the place such a note belongs.
SUGGESTED: Delete the paragraph from the ctz page. If the project wants the performance note preserved, it is a candidate for `.ai/arch-abi.md` or the spec, not the developer page.

### 5. The overview says the shifts "validate their count" without saying the rule or the error it produces
UNIT:      man-pkg:bits
PAGE:      package overview
CATEGORY:  discoverability
CLAIM:     "Shifts (sl, sr, sra) validate their count argument." ... "All functions are total except the three shifts."
VERDICT:   misleading
EVIDENCE:  The overview's derived Errors table lists `77050002 ErrInvalidArgument` with no condition anywhere on the page; the range 0..63 appears only on the three shift pages. probe2 `bits::sl(1, 64)` and probe3 `bits::sr(1, -1)` both abort with `Error: 7-705-0002 Argument value is not valid for the requested operation.` A developer deciding between a shift and a rotate from the overview alone cannot learn that 64 raises while rotates accept it, which is exactly the sharp edge the standard (§2, "clamps vs. raises") says the page must state.
SUGGESTED: "Shifts (sl, sr, sra) require a count from 0 to 63 inclusive and raise ErrInvalidArgument for anything else — a count is never clamped or reduced. Rotates accept any count, including negatives, and reduce it modulo their width."

### 6. No page tells the developer how to look at a bit pattern, so half the examples print opaque numbers
UNIT:      man-pkg:bits
PAGE:      package-wide (overview; examples on bswap64, rl64, rr64, sl, bnot)
CATEGORY:  coverage
CLAIM:     Example captions such as "Byte order flips between little-endian and big-endian:" followed by `io::print(toString(bits::bswap64(0x1122334455667788)))`
VERDICT:   missing
EVIDENCE:  probe4: that example prints `-8613303245920329199`; the rr64 example prints `-72057594037927936`; `sl(1,63)` prints `-9223372036854775808`. Nothing in the output lets a reader confirm the claimed `0x8877665544332211`. `grep -rhn 'name: "..."' src/codegen/builtins/strings/ src/codegen/builtins/general/` shows no Integer-to-hex or radix formatter (the only hex functions are `encoding::hexEncode/hexDecode`, which take bytes). The unit's stated purpose is "byte-level codecs and other bit-twiddling", yet it never answers the first question such a developer has: how do I see the result in hex? Every example in all 17 pages prints signed decimal.
SUGGESTED: Add one sentence to the overview: "toString renders an Integer as signed decimal, so a result with bit 63 set prints as a negative number; there is no built-in hex formatter for Integer — to inspect bit patterns, extract bytes with bits::sr and bits::band (see the bits::sr example) or compare against an expected literal." And change the bswap64/rr64 second examples to print a comparison (`bits::bswap64(0x1122334455667788) = <expected>`) or an extracted byte, so the printed output demonstrates the caption.
