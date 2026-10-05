# bug-890: stdlib spec drift in 12_bits / 13_money / 14_os / 15_astrings / 17_transports — dangling citations, a stale SWAR claim, missing rows and values

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (spec)

Status: Open
Regression Test: `mfb spec` citation sweep (tests/spec citation gate), plus prose review

Each item below is a SPEC defect. The code is the reference, and each item gives the
evidence for that.

References: src/docs/spec/stdlib/{12_bits,13_money,14_os,15_astrings,17_transports}.md; found by batch B06.

## Items

1. **12_bits §"Value and operand model" — two dangling citations.**
   `[[src/codegen/builtins/mod.rs:resolve_call_return_type]]` and
   `[[src/codegen/registry/mod.rs:call_return_type]]`. `resolve_call_return_type` does not
   exist anywhere. Only `resolve_call_return_type_typed` / `resolve_call_return_type_with_kinds`
   (src/codegen/builtins/mod.rs) are defined, and the bare name occurs only in comments.
   `call_return_type` exists, but in **src/codegen/builtins/mod.rs** (line 649). The cited
   src/codegen/registry/mod.rs defines only `call_return_type_typed`, so the citation names the wrong file.
   Command: `grep -rn "fn resolve_call_return_type\b\|fn call_return_type\b" src` → only
   `src/codegen/builtins/mod.rs:649: pub(crate) fn call_return_type`.

2. **12_bits §"Bit counting" and §"One native instruction per op" — popCount is NEON on AArch64, not SWAR.**
   The spec says popCount "is computed with the standard SWAR fold … **not a SIMD
   population-count**, so it produces the identical count through the same
   integer-only path on every backend", and "with no floating point, no SIMD
   population-count". `src/codegen/builtins/bits/func_pop_count.rs:lower_bits_pop_count`
   emits `vector_dup_from_x` / `vector_cnt8b` / `vector_addv8b` on AArch64 (comment:
   "plan-39 K2 … instead of the 12-instruction SWAR") and keeps SWAR only on other ISAs.
   Measured: `otool -tv` of a `--debug` build of `/tmp/hardening/B06/bits4` shows
   `dup.2d v0, x10; cnt.8b v0, v0; addv.8b b0, v0` inside `_mfb_fn_f`. The counts are
   still correct: popCount(0)=0, (-1)=64, (min)=1, (0x5555…)=32
   (`/tmp/hardening/B06/bits1`). The fix is to rewrite the claim: NEON `CNT`/`ADDV` on
   AArch64, SWAR elsewhere, identical results.

3. **13_money §"Operation formulas" — the table omits `toMoney(Float)` and `toMoney(String)`, which do read the mode.**
   `mfb man money` says the mode drives "conversion from Float or String"; the spec
   says "Setting the mode affects **only** `Money` arithmetic" and gives no row for
   either. Measured (`/tmp/hardening/B06/money3`):
   `toMoney("0.000005")` = `0.00001` Commercial / `0.00000` Banker.
   `toMoney(0.000025)` = `0.00003` / `0.00002`: the binary64 product `0.000025*1e5` is
   exactly `2.5`, although the double's exact value is above the tie. So the
   conversion is binary64 multiply-then-settle, like the Money×Float row.
   `toMoney(0.000075)` = `0.00007` in both modes (the binary64 product is 7.4999…).
   Add the two rows and the binary64 caveat.

4. **14_os §"Build-target constants" — `os::name` value set omits `"windows"`.**
   The table says `"macos"` or `"linux"`. `src/codegen/builtins/os/gen_shared.rs:os_family`
   returns `"windows"` for `PlatformFamily::Windows`, and the same topic elsewhere
   documents `windows-x86_64` support (appResourcePath, appDataPath).

5. **14_os §"Error model" — `ErrEncoding` is missing, and the prose contradicts the args section.**
   §"The argument vector" says `os::args` and `os::prog` raise `ErrEncoding` on
   malformed host bytes. The error-model table has no `ErrEncoding` row, and the
   closing paragraph says `os::args` has "no failure mode of their own beyond a
   possible `ErrOutOfMemory`". Measured: `./os3.out "$(printf '\xff\xfe')"` →
   `args ERR:77020004`; `exec -a "$(printf '\xff')" os4.out` → `ERR:77020004`
   (= `ErrEncoding`, src/codegen/builtins/errorcode/mod.rs).

6. **14_os §"The OS seam" — "The seam is entirely libc".** This is false on Windows
   (`GetEnvironmentVariableW`, `GetModuleFileNameW`, `SHGetKnownFolderPath`, documented in
   the same topic). Scope the sentence to POSIX targets.

7. **15_astrings §toMarkdown "Escaping" — "font names additionally escape `;` and a literal `-`".**
   Only a font name that is exactly `-` is escaped (`\-`). An embedded `-` is not:
   `font("a-b\\c*")` → `::a-b\\c\*::`, while `font("-")` → `::\-::`
   (`/tmp/hardening/B06/as1`; src/codegen/builtins/astrings/helper_md_escape_font.rs).
   The code is right, because only a bare `-` reads as a reset. Reword to "a font name that is exactly `-`".

8. **17_transports §"Handles" — "`listen`'s `backlog` defaults to `128` on both transports … queue the same depth".**
   On macOS `tls::listen` ignores `backlog` entirely: `src/codegen/builtins/tls/gen_macos/server.rs:lower_tls_listen_macos`
   says "x4 (backlog) is accepted for ABI parity but unused: Network.framework …", and
   `mfb man tls listen` says "Ignored on macOS". The spec should carry the same caveat.

9. **17_transports §"The shared endpoint" — the IPv4-only claim is false for macOS `tls::listen`.**
   `tls::listen("::1", …)` binds `[::1]` (lsof). The defect is filed separately as
   B06-tls-listen-macos-unvalidated-host-binds-all-interfaces. Listed here only so the
   spec sentence is re-checked when that fix lands.

## Goal

- Each item's sentence matches the cited code / measurement; items 1's citations resolve.
