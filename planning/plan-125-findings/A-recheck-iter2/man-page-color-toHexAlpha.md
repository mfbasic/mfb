Every sentence checked out against the binary. The only gap is a §2 omission: the Errors table lists an error the prose never explains.

### 1. ErrOutOfMemory is listed but its condition is never stated
UNIT:      man-page:color/toHexAlpha
CLAIM:     Errors table row `77010001 │ ErrOutOfMemory │ Allocation failed.` with no sentence in the Description naming when it is raised.
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/func_to_hex_alpha.rs` declares `errors: vec!["ErrOutOfMemory"]` and the DESC string never mentions it. §2 of `.ai/man-content.md` requires "every raisable error and the condition that raises it", and the sibling `color::fromHex` page (same package, same author) does state it: "`ErrOutOfMemory` means the program ran out of memory." The table row's "Allocation failed." text is the derived message carve-out (§4.4 item 2) and is not itself a finding.
SUGGESTED: Append to the first Description paragraph: "The only error is `ErrOutOfMemory`, raised when the program runs out of memory while building the string; no colour value can make `toHexAlpha` fail."

Everything else verified. Probes under /tmp/plan-125-scratch/A-recheck-iter2/man-page-color-toHexAlpha, built with `mfb init` + `mfb build -q`:

- Example 1 printed `#ff000080` and `#01020304`, matching "alpha kept" and "leading zero below 16".
- Example 2 printed `#3366cc40`, matching the exact round trip.
- Probe printed `#00000000` for both toHexAlpha and toString of a fully transparent black, `#ffffffff` for opaque white, `#0aabcdef` for toHexAlpha versus `#0aabcd` for toHex, `#0aabcdef` for fromHex of uppercase `#0AABCDEF`, length 9 and `#ff00100f` for clamped inputs 300/-5/16/15. This confirms lowercase digits, fixed nine-character width, toString identity, and the lossless round trip.
- "Every colour has exactly one spelling" holds because all four Color record properties are `ParameterType::Byte` (`src/codegen/builtins/color/mod.rs`, `RegistryRecord` for `COLOR_TYPE`), so no out-of-range channel can exist.
- Banned-vocabulary grep over the rendered page hit only the derived "Allocation failed." message, which §4.4 exempts. No sentence teaches a borrow or ownership model, and nothing requires a compiler mental model.
