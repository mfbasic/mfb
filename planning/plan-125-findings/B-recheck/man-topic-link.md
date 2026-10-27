### 1. The mandatory `project.json` `libraries` entry is never explained
UNIT:      man-topic:link
PAGE:      package-wide (Description / Loading and calls)
CATEGORY:  coverage
CLAIM:     The prose never says that a `LINK "name"` will not build until `project.json` carries a `libraries` entry mapping that name to per-target locators. The only hints are nine Diagnostics rows (`NATIVE_LIBRARY_MISSING` … `NATIVE_LIBRARY_VENDOR_COLLISION`) that use "locator", "vendor/" and "libraries entry" as if already introduced.
VERDICT:   missing
EVIDENCE:  Probe at /tmp/plan-125-scratch/B-recheck/man-topic-link/probe: `mfb build` with the page's sqlite example and no `libraries` key → `./project.json:1 error[2-203-0114 NATIVE_LIBRARY_MISSING]: a LINK block names a library with no matching project.json `libraries` entry`. With `"libraries": {"sqlite3": [{"os":"macos","type":"system","source":"libsqlite3.dylib"}]}` it builds (and warns `libraries.sqlite3 has no locator covering windows/x86_64`). Contract: `mfb spec tooling project-manifest`, "Library Locator Entries" (`libraries maps each native LINK logical library name … to an ordered array of locators`).
SUGGESTED: Add a short "Telling the build where the library is" section after Description: the string in `LINK "sqlite3"` is a logical name; `project.json` must map it under `libraries` to one locator per OS/arch (`type: system` for an installed library, `type: vendor` for a file under `vendor/`); show the three-line macOS/Linux entry; note the uncovered-target warning; point to `mfb spec tooling project-manifest`. Then the nine NATIVE_LIBRARY_* rows have a referent.

### 2. FREE is a block, but the page describes a one-line clause
UNIT:      man-topic:link
PAGE:      Result gates
CATEGORY:  consistency
CLAIM:     "FREE <slot> runs a declared native deallocator on a produced CPtr slot once its value has been copied into an MFBASIC value." Nothing says how the deallocator is "declared", and the Diagnostics row for NATIVE_FREE_INVALID calls it "a FREE block … a deallocator taking one CPtr and returning CVoid". The page uses two shapes for one construct and shows neither.
VERDICT:   inconsistent
EVIDENCE:  `tests/rt-behavior/native/native-link-free-rt/src/main.mfb`: the real syntax is
```
FREE text
  SYMBOL "sqlite3_free"
  ABI (ptr CPtr) AS CVoid
END FREE
```
nested inside the wrapper, after `RETURN text` on an `AS text CPtr` return paired with a wrapper `AS String`. `mfb spec language native-libraries` ("a binding that needs to release a returned pointer must take it as the wrapper's own CPtr result and use a FREE block").
SUGGESTED: Give FREE its own short subsection with the block above, stating: the wrapper returns `AS String` while the ABI return is `CPtr`; the bytes are copied into the String first; then the FREE block's SYMBOL/ABI (one CPtr, returning CVoid) is called on the original pointer. Also move it out of "Result gates", which it is not.

### 3. The ABI types table omits two slot types the page itself uses
UNIT:      man-topic:link
PAGE:      ABI types
CATEGORY:  consistency
CLAIM:     "The ABI type names below are the C-facing types an ABI signature may use" — the table lists CInt*/CUInt*/CBool/CFloat/CDouble/CByte/CString/CPtr/CVoid only. The Structs and buffers section puts `CBuffer` and CSTRUCT names (`info INOUT SfFormatInfo`, `buf OUT CBuffer`) in ABI slots.
VERDICT:   inconsistent
EVIDENCE:  Rendered page, Structs and buffers vs ABI types sections. `mfb spec language native-libraries` ABI table has a `CBuffer` row ("A runtime-sized writable byte buffer the callee fills, surfaced as List OF Byte. Valid only as an OUT slot …") and notes `CFloat32/CFloat64, CIntPtr, CUIntPtr, and CSize do not exist … rejected with NATIVE_ABI_UNKNOWN_CTYPE`.
SUGGESTED: Add rows `CBuffer` (OUT-only byte buffer, see Structs and buffers) and `<CSTRUCT name>` (a struct declared in this LINK block, IN/OUT/INOUT). Add one sentence that `CSize`, `CIntPtr`, `CFloat32` and C spellings like `int`/`long` are not names and are rejected with NATIVE_ABI_UNKNOWN_CTYPE, so a developer reading a C header knows to pick the fixed-width name.

### 4. The Errors table and prose omit the value-conversion failures a binding author will hit
UNIT:      man-topic:link
PAGE:      Native functions / Errors
CATEGORY:  coverage
CLAIM:     The page never says how MFBASIC `Integer`/`Float`/`String`/`Boolean` map onto C slot types or what happens when a value does not fit. Its Errors table lists only ErrResourceClosed, ErrNativeBindingUnavailable, ErrNativeBindingCallFailed and ErrNativeBufferOverrun. (This table is authored markdown in `src/docs/man/link/package.md`, not derived.)
VERDICT:   missing
EVIDENCE:  Probe (same directory): `sql::bindInt(stmt, 1099511627776, 5)` on an `iCol CInt32` slot printed `Error: 7-705-0010 … Arithmetic overflow or numeric conversion outside the destination range.` (ErrOverflow 77050010), exit 255. `mfb spec language native-libraries` "marshaling boundary" list: CInt32 arguments are range-checked (ErrOverflow); CFloat too large → ErrFloatOverflow 77050015; NaN/Inf CFloat/CDouble return → ErrFloatNaN/ErrFloatInf; CPtr-to-String with malformed UTF-8 → ErrEncoding 77020004, NULL → `""`; narrower integers and CUInt* are passed through unchecked.
SUGGESTED: In Native functions, add a short "Values across the boundary" paragraph: Integer→CInt32 is range-checked and raises ErrOverflow; CInt8/16 and CUInt* are not checked; Float→CFloat rounds and raises ErrFloatOverflow when too large; a NaN/infinite C float result raises ErrFloatNaN/ErrFloatInf; a `CPtr` return paired with `AS String` is copied and UTF-8-validated (ErrEncoding), NULL becomes an empty String; Boolean/CBool is nonzero→TRUE. Add ErrOverflow, ErrFloatOverflow, ErrFloatNaN, ErrFloatInf and ErrEncoding rows to the Errors table.

### 5. The per-target argument-count limit is not mentioned
UNIT:      man-topic:link
PAGE:      Native functions
CATEGORY:  coverage
CLAIM:     Nothing on the page tells a developer that an ABI with more integer-class slots than the target passes in registers is rejected at build time (six on x86-64, eight on AArch64/riscv64), so a binding that builds on a Mac can fail on Linux x86-64.
VERDICT:   missing
EVIDENCE:  `mfb spec language native-libraries`, "Integer-slot limit (target-dependent) … SysV x86-64 passes only six … a native function declaring more integer (non-CDouble) ABI slots than the target passes in registers is rejected at build time." UNVERIFIED which diagnostic code is raised; I did not build a seven-slot wrapper for x86-64 in this sandbox.
SUGGESTED: One sentence under Native functions: "A native function may declare at most as many non-floating-point slots as the target passes in registers (six on x86-64, eight on AArch64 and riscv64), including OUT and CONST slots; more is a build error." Add the code to the Diagnostics table once confirmed.

### 6. CBuffer SIZE has a configurable ceiling the page does not mention
UNIT:      man-topic:link
PAGE:      Structs and buffers
CATEGORY:  coverage
CLAIM:     "SIZE is worked out before the call … and must be at least the most the function can write." The page gives a floor but not the ceiling: SIZE above `options.maxBuffer` (default 64 MiB) is a runtime failure, and the knob lives in `project.json`.
VERDICT:   missing
EVIDENCE:  `mfb spec language native-libraries`: "That ceiling is 64 MiB by default and is set per project by options.maxBuffer in project.json, in MiB … Valid values are whole numbers from 1 to 4096". `mfb spec tooling project-manifest` "options.maxBuffer sets that ceiling". UNVERIFIED at runtime which error a too-large SIZE raises; not probed.
SUGGESTED: After the LENGTH sentence add: "SIZE may not exceed the project's buffer ceiling, 64 MiB unless `options.maxBuffer` in project.json raises it (1–4096 MiB); a larger SIZE fails the call." Name the error once confirmed.

Checked and found consistent: all three `See also` spec targets and `mfb man variable/errors/types` render; the sqlite and sndfile examples match the spec's and the test fixtures' syntax; the SUCCESS_ON/ERROR_ON, CONST, BIND IN/STATE and RETURN rules agree with `mfb spec language native-libraries`; `link` is discoverable from the `mfb man` index with the same one-line intro.
