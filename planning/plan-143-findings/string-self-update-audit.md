# plan-143 findings: which `String` self-updates the compiler performs in place

Measured against: **a `MUT` updating itself mutates in place, with no copy.**
Every verdict below is read from the compiler source at `efdb54bb7` (plan-142
landed) and cross-checked against `mfb build --ncode` (Appendix C). This is the
`String` counterpart of `planning/plan-141-findings/inplace-audit.md`; its method,
gate codes (G1–G26) and marker technique are reused unchanged.

## Binding sites (the columns)

| Site | Meaning |
|---|---|
| S1 local | `MUT s` declared in a FUNC/SUB body |
| S2 global | `MUT s` at module level |
| S7 loop-live | `MUT s` while a `FOR EACH` walks the same binding |
| S9 captured | `MUT s` assigned inside a non-escaping `forEach` `LAMBDA` that captures it (a `by_ref` local) |

Records, `RES` and `STATE` are excluded (plan §1 non-goals): a `String` record
field is not a column.

Cell values: `y` = in place; `n (<gate>)` = the named gate declines (the first one
in code order that does) and the statement takes the copying path;
`n (no arm)` = no arm of `SELF_UPDATE_ARMS` recognises the call, so the statement
always takes the copying path; `n/a` = the form cannot be written (reason given).

## 1. Overloads

| function definition | form | S1 | S2 | S7 | S9 | evidence |
|---|---|---|---|---|---|---|
| `astrings::addAttribute(value AS AttributedString, start AS Integer, endIndex AS Integer, attr AS astrings::Attribute) AS AttributedString` | same-len | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: the text is unchanged; only the span list is rewritten. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: `astrings::writeSpans(a, spans)` (`gen_astrings.rs`) builds a new `AttributedString` record with a copy of the text. Probes: `r01_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r01_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda0` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `astrings::clearAttributes(value AS AttributedString) AS AttributedString` | same-len | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: the text is unchanged; the span list is emptied or split. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: `astrings::writeSpans(a, [])`. Probes: `r02_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r02_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda1` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `astrings::clearAttributes(value AS AttributedString, start AS Integer, endIndex AS Integer) AS AttributedString` | same-len | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: the text is unchanged; the span list is emptied or split. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: `astrings::writeSpans(a, out)`. Probes: `r03_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r03_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda2` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `astrings::removeAttribute(value AS AttributedString, start AS Integer, endIndex AS Integer, attr AS astrings::Attribute) AS AttributedString` | same-len | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: the text is unchanged; only the span list is rewritten. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: `astrings::writeSpans(a, spans)`. Probes: `r04_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r04_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda3` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `encoding::formUrlDecode(value AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: never longer (`%XX` → 1 byte, `+` → space), but not a subsequence of `value`. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: the body builds and `RETURN`s a new `String`. Probes: `r05_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r05_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda4` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `encoding::formUrlEncode(value AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: never shorter. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: the body builds and `RETURN`s a new `String`. Probes: `r06_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r06_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda5` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `encoding::htmlEscape(value AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: never shorter. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: the body builds and `RETURN`s a new `String`. Probes: `r07_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r07_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda6` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `encoding::htmlUnescape(value AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: never longer. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: the body builds and `RETURN`s a new `String`. Probes: `r08_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r08_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda7` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `encoding::percentDecode(value AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: never longer. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: the body builds and `RETURN`s a new `String`. Probes: `r09_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r09_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda8` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `encoding::percentEncode(value AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: never shorter. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: the body builds and `RETURN`s a new `String`. Probes: `r10_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r10_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda9` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `encoding::punycodeDecode(asciiDomain AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: label-wise decode. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: the body builds and `RETURN`s a new `String`. Probes: `r11_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r11_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda10` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `encoding::punycodeEncode(domain AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: label-wise encode. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: the body builds and `RETURN`s a new `String`. Probes: `r12_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r12_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda11` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `fs::canonicalPath(path AS String) AS String` | not-derived | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: the result is copied from `realpath`'s output buffer (symlinks resolved against the filesystem), not from `path`'s bytes. Not-derived: `s` is read, and copied once into a helper-scratch C string (`c_path`, freed before return) — a copy, finding F4. `self_update_builtin` answers `None` for the `abi_function` helper (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block copied from the `realpath` `PATH_MAX` scratch buffer; `path` itself is marshalled into a scratch C string `c_path` first (bug-574 scratch, freed at the helper's exit). Probes: `r13_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r13_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda12` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `fs::pathBaseName(path AS String) AS String` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: the last component's span. `self_update_builtin` names it (`pathBaseName`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `pathBaseName`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `emit_materialize_string_from_bytes` of the last component's span. Probes: `r14_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r14_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda13` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `fs::pathDirName(path AS String) AS String` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: the directory span — or a read-only `.`/`/` constant (F3). `self_update_builtin` names it (`pathDirName`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `pathDirName`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block of the directory span (`emit_materialize_string_from_bytes`), **or a read-only constant**: `load_string_constant(".")` / `("/")` for a path with no directory part / the root (`gen_path_builder.rs`, labels `dot`, `root`) — see §3.2 finding F3. Probes: `r15_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r15_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda14` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `fs::pathExtension(path AS String) AS String` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: the extension span. `self_update_builtin` names it (`pathExtension`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `pathExtension`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `emit_materialize_string_from_bytes` of the extension span. Probes: `r16_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r16_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda15` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `fs::pathNormalize(path AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: segments removed or collapsed (a subsequence of `path`), or `.`; not a single window. `self_update_builtin` names it (`pathNormalize`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `pathNormalize`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: the normalized bytes are assembled and materialized. Probes: `r17_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r17_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda16` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `fs::readText(path AS String) AS String` | not-derived | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: the file's bytes. Not-derived: `s` is read, and copied once into a helper-scratch C path (`{symbol}_path_copy_loop`) — a copy, finding F4. `self_update_builtin` answers `None` for the `abi_function` helper (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block holding the file's bytes; `path` is copied into a scratch C path first (`{symbol}_path_copy_loop`). Probes: `r18_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r18_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda17` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `io::input([prompt AS String]) AS String` | not-derived | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: a line read from stdin. Not-derived: `s` is only read (written to stdout as the prompt); not copied. `self_update_builtin` answers `None` for the `abi_function` helper (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: a grown line buffer copied into the result (`result_copy_loop`); `prompt` is only written to stdout. Probes: `r19_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r19_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda18` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `net::percentDecode(s AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: never longer. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: the body builds and `RETURN`s a new `String`. Probes: `r20_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r20_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda19` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `os::getEnv(name AS String) AS String` | not-derived | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: the variable's value. Not-derived: `s` is read, and copied once into a helper-scratch C string (`marshal_cstring`, freed at `done`) — a copy, finding F4. `self_update_builtin` answers `None` for the `abi_function` helper (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block holding the variable's value; `name` is copied into a scratch C string (`marshal_cstring`, bug-574 scratch, freed at `done`). Probes: `r21_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r21_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda20` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `os::getEnvOr(name AS String, fallback AS String) AS String` | not-derived | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: the variable's value, or `fallback`. Not-derived: `s` is read, and copied once into a helper-scratch C string (`marshal_cstring`) — a copy, finding F4. `self_update_builtin` answers `None` for the `abi_function` helper (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block holding the value or a copy of `fallback`; `name` is copied into a scratch C string (`marshal_cstring`). Probes: `r22_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r22_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda21` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `os::resourcePath(relative AS String) AS String` | grow | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: `base + "/" + relative`: bytes inserted before `relative`. `self_update_builtin` answers `None` for the `abi_function` helper (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `base + "/" + relative` concatenated into an owned arena `String`. Probes: `r23_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r23_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda22` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `regex::replace(value AS String, pattern AS String, replacement AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: each match replaced; longer or shorter. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: the body builds and `RETURN`s a new `String`. Probes: `r24_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r24_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda23` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::caseFold(value AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: same length on the all-ASCII path only; the Unicode path counts a mapped length that can differ (`gen_case_map.rs` count loop). `self_update_builtin` names it (`caseFold`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `caseFold`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `emit_arena_alloc_call` of `byteLen + 9` on the all-ASCII path, or of the counted mapped length on the Unicode path (`gen_case_map.rs`). Probes: `r25_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r25_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda24` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::graphemeAt(value AS String, index AS Integer) AS String` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: one grapheme's contiguous byte span of `value`. `self_update_builtin` names it (`graphemeAt`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `graphemeAt`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `emit_materialize_string_from_bytes` of the grapheme's byte span. Probes: `r26_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r26_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda25` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::left(value AS String, count AS Integer) AS String` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: a prefix window of `value`. `self_update_builtin` names it (`left`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `left`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: computes a `(ptr, len)` window into `value`, then `emit_materialize_string_from_bytes` (`builder_collection_layout.rs`) copies it. Probes: `r27_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r27_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda26` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::lower(value AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: as `caseFold`. `self_update_builtin` names it (`lower`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `lower`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `emit_arena_alloc_call` of `byteLen + 9` on the all-ASCII path, or of the counted mapped length on the Unicode path (`gen_case_map.rs`). Probes: `r28_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r28_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda27` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::mid(value AS String, start AS Integer, count AS Integer) AS String` | shrink | n (G10) | n (G10) | n/a (not iterable) | n (G10) | Form: a contiguous span of `value`. The `mid` arm (`try_inplace_mid_assign`, via `resolve_self_update`, `inplace_dest.rs`) passes G2–G6 and declines at G10: `CollectionTypeLayout::from_type(String)` is `None` (`validation.rs`); at S9 the `Ref` destination discharges G1 first. `self_update_builtin("strings.mid")` = `mid` because `native_builtin_target` dequalifies `strings.`/`collections.` `mid` alike (`builtins/mod.rs`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `emit_arena_alloc_call` (`mid_alloc_ok`) and a span copy. Probes: `r29_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r29_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda28` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::normalizeNfc(value AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: composition can shorten or reorder; length changes either way. `self_update_builtin` names it (`normalizeNfc`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `normalizeNfc`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `emit_arena_alloc_call` (three sites in `func_normalize_nfc.rs`). Probes: `r30_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r30_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda29` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::padLeft(value AS String, width AS Integer, [padChar AS String]) AS String` | grow | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: pad bytes inserted before `value` (unchanged when already wide enough). `self_update_builtin` names it (`padLeft`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `padLeft`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `emit_arena_alloc_call` of the padded length (`gen_pad.rs`). Probes: `r31_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r31_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda30` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::padLeftToWidth(value AS String, columns AS Integer, [padChar AS String]) AS String` | grow | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: pad inserted before `value`. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: the helper builds and `RETURN`s a new `String`. Probes: `r32_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r32_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda31` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::padRight(value AS String, width AS Integer, [padChar AS String]) AS String` | grow | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: pad bytes appended after `value`. `self_update_builtin` names it (`padRight`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `padRight`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `emit_arena_alloc_call` of the padded length (`gen_pad.rs`). Probes: `r33_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r33_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda32` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::padRightToWidth(value AS String, columns AS Integer, [padChar AS String]) AS String` | grow | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: pad appended after `value`. `self_update_builtin` answers `None` for a `bl` to the MFBASIC body (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: the helper builds and `RETURN`s a new `String`. Probes: `r34_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r34_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda33` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::repeat(value AS String, times AS Integer) AS String` | grow | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: `value` repeated `times` (`times = 0` gives the empty string). `self_update_builtin` names it (`repeat`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `repeat`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `emit_arena_alloc_call` of `len × times + 9`. Probes: `r35_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r35_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda34` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::replace(value AS String, old AS String, new AS String) AS String` | rewrite | n (G10) | n (G10) | n/a (not iterable) | n (G10) | Form: each match replaced; longer or shorter. The `replace` arm (`try_inplace_replace_assign`, via `resolve_self_update`, `inplace_dest.rs`) passes G2–G6 and declines at G10: `CollectionTypeLayout::from_type(String)` is `None` (`validation.rs`); at S9 the `Ref` destination discharges G1 first. `self_update_builtin("strings.replace")` = `replace` because `native_builtin_target` dequalifies `strings.`/`collections.` `replace` alike (`builtins/mod.rs`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block on both arms: `emit_arena_alloc_call` when a match is replaced, `copy_flat_block` of `value` when none is (bug-536 shape B). Probes: `r36_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r36_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda35` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::right(value AS String, count AS Integer) AS String` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: a suffix window. `self_update_builtin` names it (`right`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `right`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: computes a `(ptr, len)` window into `value`, then `emit_materialize_string_from_bytes` (`builder_collection_layout.rs`) copies it. Probes: `r37_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r37_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda36` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::stripPrefix(value AS String, prefix AS String) AS String` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: a suffix window. `self_update_builtin` names it (`stripPrefix`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `stripPrefix`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: window into `value` (the whole of it when the affix is absent), then `emit_materialize_string_from_bytes`. Probes: `r38_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r38_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda37` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::stripSuffix(value AS String, suffix AS String) AS String` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: a prefix window. `self_update_builtin` names it (`stripSuffix`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `stripSuffix`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: window into `value` (the whole of it when the affix is absent), then `emit_materialize_string_from_bytes`. Probes: `r39_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r39_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda38` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::trim(value AS String) AS String` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: an interior window. `self_update_builtin` names it (`trim`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `trim`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `[start, end)` window into `value`, then `emit_materialize_string_from_bytes`. Probes: `r40_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r40_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda39` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::trimChars(value AS String, chars AS String) AS String` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: an interior window. `self_update_builtin` names it (`trimChars`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `trimChars`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: window into `value`, then `emit_materialize_string_from_bytes`. Probes: `r41_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r41_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda40` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::trimEnd(value AS String) AS String` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: a prefix window. `self_update_builtin` names it (`trimEnd`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `trimEnd`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `[start, end)` window into `value`, then `emit_materialize_string_from_bytes`. Probes: `r42_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r42_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda41` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::trimStart(value AS String) AS String` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: a suffix window. `self_update_builtin` names it (`trimStart`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `trimStart`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `[start, end)` window into `value`, then `emit_materialize_string_from_bytes`. Probes: `r43_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r43_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda42` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::upper(value AS String) AS String` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: as `caseFold` (e.g. `ß` → `SS`). `self_update_builtin` names it (`upper`, a `Body::abi_inline` bare name via `native_bare_target`), so S2 builds a `Global` site and S9 a `Ref` site and runs `SELF_UPDATE_ARMS`; every call arm declines at G3 (no arm is named `upper`) and the concat arm at G19/G20 (no shadow; not a `&` chain). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh block: `emit_arena_alloc_call` of `byteLen + 9` on the all-ASCII path, or of the counted mapped length on the Unicode path (`gen_case_map.rs`). Probes: `r44_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r44_S2` ARM=- SEAM=su_global_block STRCAP=- GLOBAL=y; `$lambda43` ARM=- SEAM=su_ref_block STRCAP=- GLOBAL=-. |
| `strings::left(value AS AttributedString, count AS Integer) AS AttributedString` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (a prefix window of `value`); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_left (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r45_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r45_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda44` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::right(value AS AttributedString, count AS Integer) AS AttributedString` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (a suffix window); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_right (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r46_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r46_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda45` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::mid(value AS AttributedString, start AS Integer, count AS Integer) AS AttributedString` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (a contiguous span of `value`); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_mid (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r47_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r47_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda46` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::trim(value AS AttributedString) AS AttributedString` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (an interior window); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_trim (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r48_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r48_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda47` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::trimStart(value AS AttributedString) AS AttributedString` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (a suffix window); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_trimStart (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r49_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r49_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda48` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::trimEnd(value AS AttributedString) AS AttributedString` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (a prefix window); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_trimEnd (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r50_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r50_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda49` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::trimChars(value AS AttributedString, chars AS String) AS AttributedString` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (an interior window); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_trimChars (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r51_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r51_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda50` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::stripPrefix(value AS AttributedString, prefix AS String) AS AttributedString` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (a suffix window); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_stripPrefix (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r52_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r52_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda51` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::stripSuffix(value AS AttributedString, suffix AS String) AS AttributedString` | shrink | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (a prefix window); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_stripSuffix (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r53_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r53_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda52` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::padLeft(value AS AttributedString, width AS Integer, [padChar AS String]) AS AttributedString` | grow | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (pad bytes inserted before `value` (unchanged when already wide enough)); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_padLeft (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r54_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r54_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda53` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::padLeftToWidth(value AS AttributedString, columns AS Integer, [padChar AS String]) AS AttributedString` | grow | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (pad inserted before `value`); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_padLeftToWidth (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r55_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r55_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda54` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::padRight(value AS AttributedString, width AS Integer, [padChar AS String]) AS AttributedString` | grow | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (pad bytes appended after `value`); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_padRight (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r56_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r56_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda55` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::padRightToWidth(value AS AttributedString, columns AS Integer, [padChar AS String]) AS AttributedString` | grow | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (pad appended after `value`); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_padRightToWidth (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r57_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r57_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda56` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::repeat(value AS AttributedString, times AS Integer) AS AttributedString` | grow | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (`value` repeated `times` (`times = 0` gives the empty string)); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_repeat (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r58_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r58_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda57` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::replace(value AS AttributedString, old AS String, new AS String) AS AttributedString` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (each match replaced; longer or shorter); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_replace (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r59_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r59_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda58` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::upper(value AS AttributedString) AS AttributedString` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (as `caseFold` (e.g. `ß` → `SS`)); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_upper (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r60_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r60_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda59` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::lower(value AS AttributedString) AS AttributedString` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (as `caseFold`); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_lower (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r61_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r61_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda60` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::caseFold(value AS AttributedString) AS AttributedString` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (same length on the all-ASCII path only; the Unicode path counts a mapped length that can differ (`gen_case_map.rs` count loop)); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_caseFold (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r62_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r62_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda61` ARM=- SEAM=- STRCAP=- GLOBAL=-. |
| `strings::normalizeNfc(value AS AttributedString) AS AttributedString` | rewrite | n (no arm) | n (no arm) | n/a (not iterable) | n (no arm) | Form: text as the `String` overload (composition can shorten or reorder; length changes either way); the span list is remapped (or dropped, for the case maps and `normalizeNfc`). `self_update_builtin` answers `None` for #astrings_normalizeNfc (not a `strings.`/`collections.` native), so S2/S9 never build a self-update site; S1 runs `SELF_UPDATE_ARMS` and every call arm declines at G3 (the name) and the concat arm at G19/G20; the record-field arms that follow decline at G2 (not a `WITH`). Copying path: S1 `lower_value_owned` + free of the old block (`src/codegen/engine/control/builder_control.rs` `NirOp::Assign`); S2 `StoreGlobal` copying path (`store_global_new`); S9 the by-ref reassign (`reassign_ref_old`). Result: fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`). Probes: `r63_S1` ARM=- SEAM=- STRCAP=- GLOBAL=-; `r63_S2` ARM=- SEAM=- STRCAP=- GLOBAL=y; `$lambda62` ARM=- SEAM=- STRCAP=- GLOBAL=-. |

## 2. Operator and unqualified forms

| form | S1 | S2 | S7 | S9 | evidence |
|---|---|---|---|---|---|

## 3. Summary

## Appendix A — census script

`census.py rows` generated the 63 rows of §1 (signatures verbatim from each
`mfb man <pkg> <f>` page for the 44 literal hits; the 19 `AttributedString` rows
substitute `AttributedString` for the leading `String` parameter and the return of
the `String` overload, as `strings::resolve_return_type` does for a Tier-B
transform). Run from the repo root against `target/release/mfb` built from
`efdb54bb7`:

```
$ python3 census.py stats
packages 42 overloads 828
literal 44 {'astrings': 4, 'encoding': 8, 'fs': 6, 'io': 1, 'net': 1, 'os': 3, 'regex': 1, 'strings': 20}
generic-candidates 25
$ python3 census.py rows | wc -l
63
```

The 25 generic candidates (`census.py generic`) are: 7 `astrings` overloads and
2 `io` overloads whose "variable" is the nominal `AttributedString`, 5 `strings::is*`
predicates over the nominal `Scalar`, and 11 true generics (`collections::get`×2,
`getOr`×2, `reduce`, `reduceRight`; `thread::accept`×2, `receive`×2, `waitFor`).
None of the 11 can be a `String` self-update: each one's first parameter is a
`List`/`Map`/`Thread`/`ThreadWorker`, never a `String`, so `s = f(s, …)` does not
type-check.

```python
"""Row census for planning/plan-143-findings/string-self-update-audit.md.

plan-142-A's Appendix census over every `mfb man <pkg> <f>` page, with the type
test replaced by the String/AttributedString rule, plus a generic scan: every
overload whose first parameter or return type is a bare type variable (one
capitalised identifier that is not a builtin type name) is listed so the reader
can decide whether it can be instantiated to a String self-update.

Run from the repo root (it calls target/release/mfb).

  python3 census.py literal   -> the literal hits (first param type == return type,
                                  both String / AttributedString)
  python3 census.py generic   -> overloads with a type-variable first param or return
  python3 census.py stats     -> packages / overloads / literal per package
  python3 census.py rows      -> one empty §1 table row per overload: the literal
                                  hits, then the Tier-B `AttributedString` overloads
                                  of `strings::` (prose-documented, so invisible to
                                  the man census; list from TIER_B_TRANSFORMS in
                                  src/codegen/builtins/strings/mod.rs)
"""
import re
import subprocess
import sys
from collections import Counter

M = "target/release/mfb"
STRINGY = ("String", "AttributedString", "astrings::AttributedString")
BUILTIN = {"String", "Integer", "Float", "Boolean", "Byte", "Fixed", "Money",
           "Nothing", "Error", "Duration", "Big", "Char"}


def is_var(t):
    return bool(re.fullmatch(r"[A-Z][A-Za-z0-9]*", t)) and t not in BUILTIN


top = subprocess.run([M, "man"], capture_output=True, text=True).stdout
pkgs = re.findall(r"^│ ([a-zA-Z]+) +│", top[top.index("Builtin packages"):], re.M)
pkgs = [p for p in pkgs if p != "Package"]
literal, generic, total = [], [], 0
for pkg in pkgs:
    page = subprocess.run([M, "man", pkg], capture_output=True, text=True).stdout
    if "\nFunctions\n" not in page:
        continue
    funcs = []
    for f in re.findall(rf"│ {pkg}::([a-zA-Z0-9_]+)", page[page.index("\nFunctions\n"):]):
        if f not in funcs:
            funcs.append(f)
    for f in funcs:
        fp = subprocess.run([M, "man", pkg, f], capture_output=True, text=True).stdout
        lines = fp.splitlines()
        try:
            i = next(k for k, l in enumerate(lines) if l.strip() in ("Overloads", "Declaration"))
        except StopIteration:
            continue
        j, block = i + 2, []
        while j < len(lines) and not (lines[j].strip() and j + 1 < len(lines)
                                      and set(lines[j + 1].strip()) == {"─"}):
            block.append(lines[j])
            j += 1
        text = " ".join(l.strip() for l in block)
        for s in re.findall(rf"`({pkg}::[^`]+)`", text):
            s = re.sub(r"\s+", " ", s)
            m = re.match(r"\w+::\w+\((.*)\) AS (.*)$", s)
            if not m:
                continue
            total += 1
            params, ret = m.group(1), m.group(2)
            first = re.match(r"\[?\w+ AS (.*?)(?:, \[?\w+ AS |\]?$)", params)
            ft = first.group(1).rstrip("]") if first else ""
            if ft in STRINGY and ft == ret:
                literal.append((pkg, s))
            if is_var(ft) or is_var(ret):
                generic.append((pkg, s))
TIER_B = ["left", "right", "mid", "trim", "trimStart", "trimEnd", "trimChars",
          "stripPrefix", "stripSuffix", "padLeft", "padLeftToWidth", "padRight",
          "padRightToWidth", "repeat", "replace", "upper", "lower", "caseFold",
          "normalizeNfc"]
mode = sys.argv[1] if len(sys.argv) > 1 else "stats"
if mode == "rows":
    out = [s for _, s in literal]
    by_name = {re.match(r"strings::(\w+)\(", s).group(1): s
               for _, s in literal if s.startswith("strings::")}
    for f in TIER_B:
        s = by_name[f]
        s = re.sub(r"^(strings::\w+\(\w+) AS String", r"\1 AS AttributedString", s)
        s = re.sub(r"\) AS String$", ") AS AttributedString", s)
        out.append(s)
    for s in out:
        print(f"| `{s}` | | | | | | |")
elif mode == "literal":
    for _, s in literal:
        print(s)
elif mode == "generic":
    for _, s in generic:
        print(s)
else:
    print("packages", len(pkgs), "overloads", total)
    print("literal", len(literal), dict(Counter(p for p, _ in literal)))
    print("generic-candidates", len(generic))
```

## Appendix B — recogniser and lowering map

Paths: `bia` = `src/codegen/collection/assign/builder_inplace_assign.rs`,
`bc` = `src/codegen/engine/control/builder_control.rs`,
`su` = `src/codegen/collection/assign/self_update.rs`,
`idest` = `src/codegen/collection/assign/inplace_dest.rs`. Symbols are cited by
name (line numbers at `efdb54bb7` in parentheses where useful).

### B.1 Where a `String` self-update statement is dispatched, per site

- **S1 (a local).** `HirStatement::Assign` of a function local lowers to
  `IrOp::Assign` → `NirOp::Assign` (plan-141 Appendix B.1, unchanged).
  `NirOp::Assign` in `bc` (`:1192`) builds one `SelfUpdateSite` (`dest =
  InPlaceDest::Direct`, `by_ref = false`) and calls `try_inplace_self_update`
  (`su`), which runs `SELF_UPDATE_ARMS` in order; then the eight
  `try_inplace_record_field_*` arms; then the copying reassignment
  (`lower_value_owned`, then `emit_owned_value_drop` of the old block when the
  type is freeable-flat, `reassign_value` slot).
- **S2 (a module-level global).** `NirOp::StoreGlobal` in `bc` (`:1076`) builds a
  `SelfUpdateSite` with `dest = InPlaceDest::Global` **only when**
  `is_global_self_update_call(value, name)` (a `Call` whose target
  `self_update_builtin` names and whose first argument is the global, `su`) or
  `string_self_append_operands_of(value, <the global>)` (a `&` chain rooted at
  the global) holds. The site's `open_inplace_ref_dest` loads the global's block
  into the `su_global_block` slot before the arms run — emitted even when every
  arm then declines (observation O1). Otherwise, or when no arm fires: the
  copying path — `lower_value_owned`, a free of the old block through
  `store_global_old`/`store_global_new`, the store, and a reset of the hidden
  capacity global when one exists.
- **S7 (a `FOR EACH` over the binding).** Unreachable: `FOR EACH c IN s` on a
  `String` or an `AttributedString` is rejected by the type checker —
  `error[2-203-0050 TYPE_FOR_EACH_REQUIRES_COLLECTION]: FOR EACH source must be a
  List or Map` (probes `/tmp/plan-143-probes/s7`, `s7a`). Every S7 cell is
  `n/a (not iterable)`.
- **S9 (a by-ref lambda capture).** The lifted lambda's `NirOp::Assign` to the
  captured local: `by_ref = true`. When `is_self_update_call(value, name)` holds
  (a `Call` that `self_update_builtin` names, first argument the local), the site
  gets `dest = InPlaceDest::Ref` (`su_ref_block` slot), which discharges G1 for the
  collection arms (`resolve_self_update` sets `InPlaceGate.by_ref` only for a
  non-`Ref` destination, `idest`). A `&` chain is a `Binary`, so it never gets a
  `Ref` destination and the concat arm declines at G1. The copying path is the
  by-ref reassignment (`reassign_ref_old`: free the parent's old block through the
  reference, store the new one).

**Which call targets the seam can see.** `self_update_builtin` (`su`) answers
for (a) `native_builtin_target`'s bare names — `strings.`/`collections.`
`find`/`mid`/`replace` by name, and every `Body::abi_inline` member of any package
through `native_bare_target` (`src/codegen/registry/mod.rs`), which is why
`strings.trim` → `trim` and `fs.pathBaseName` → `pathBaseName` — and (b)
`#collections_*` monomorphs. It answers `None` for a `Body::mfb`/`Rewrite`
member (the call target is an internalized `#pkg_f` MFBASIC function), for a
`Body::abi_function` helper, and for the `#astrings_*` targets the Tier-B
rewrite produces. So of the §1 rows, the 22 `String` rows lowered
`Body::abi_inline`/`Body::Intrinsic` build a site at S2/S9 (marker
`su_global_block`/`su_ref_block`) and the other 41 never do.

**No arm recognises a `String` builtin.** The builtin names the arms match
(`grep -rhoE '(resolve_[a-z_]+|self_update_builtin\(target\) != Some)\([^)]*"[a-zA-Z]+"' src/codegen/collection/assign/`
→ `add append distinct drop filter insert mapValues merge mid prepend remove
removeAt removeKey replace set sort sortBy symmetricDifference take transform
union`, plus `intersection`/`difference` through `try_inplace_filter_set` and
`math.*` through `math_self_update_function`) meet the §1 function names only in
`mid` and `replace`. Both arms resolve through `resolve_self_update` (`idest`):
G2 (a `Call`) → G3/G4 (name, arity) → G5/G6 (`args[0]` is the binding) →
G-global-operand (S2) → `InPlaceGate` {G1, G7, **G10**}. G10 —
`CollectionTypeLayout::from_type(type)` is `None` — declines every `String` and
`AttributedString` by construction: `from_type`
(`src/codegen/engine/validation/validation.rs`) answers only `ListOf`, a set, or
a map type. `strings.mid`/`strings.replace` reach the arm at all only because
`native_builtin_target` dequalifies the `strings.` and `collections.` spellings to
the same bare name (`src/codegen/builtins/mod.rs`, comment there). The concat arm
(`try_inplace_concat_assign`, `bia:1609`) matches only a `Binary` `&` chain.

### B.2 Lowering of each §1 row

Every row's result is a freshly allocated block (or record) that does not reuse
`s`'s block; no builtin argument is moved into its result.

| # | function definition | body kind | result block |
|---|---|---|---|
| r01 | `astrings::addAttribute(value AS AttributedString, start AS Integer, endIndex AS Integer, attr AS astrings::Attribute) AS AttributedString` | `Body::mfb` → MFBASIC `__astrings_addAttribute` | fresh record: `astrings::writeSpans(a, spans)` (`gen_astrings.rs`) builds a new `AttributedString` record with a copy of the text |
| r02 | `astrings::clearAttributes(value AS AttributedString) AS AttributedString` | `Body::mfb` → MFBASIC `__astrings_clearAttributes` | fresh record: `astrings::writeSpans(a, [])` |
| r03 | `astrings::clearAttributes(value AS AttributedString, start AS Integer, endIndex AS Integer) AS AttributedString` | `Body::mfb` → MFBASIC `__astrings_clearAttributesRange` | fresh record: `astrings::writeSpans(a, out)` |
| r04 | `astrings::removeAttribute(value AS AttributedString, start AS Integer, endIndex AS Integer, attr AS astrings::Attribute) AS AttributedString` | `Body::mfb` → MFBASIC `__astrings_removeAttribute` | fresh record: `astrings::writeSpans(a, spans)` |
| r05 | `encoding::formUrlDecode(value AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_formUrlDecode` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r06 | `encoding::formUrlEncode(value AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_formUrlEncode` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r07 | `encoding::htmlEscape(value AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_htmlEscape` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r08 | `encoding::htmlUnescape(value AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_htmlUnescape` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r09 | `encoding::percentDecode(value AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_percentDecode` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r10 | `encoding::percentEncode(value AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_percentEncode` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r11 | `encoding::punycodeDecode(asciiDomain AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_punycodeDecode` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r12 | `encoding::punycodeEncode(domain AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_punycodeEncode` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r13 | `fs::canonicalPath(path AS String) AS String` | `Body::abi_function` → `gen_canonical::lower_fs_canonical_path_helper` | fresh block copied from the `realpath` `PATH_MAX` scratch buffer; `path` itself is marshalled into a scratch C string `c_path` first (bug-574 scratch, freed at the helper's exit) |
| r14 | `fs::pathBaseName(path AS String) AS String` | `Body::abi_inline` → `lower_fs_path_base_name_nl` (`gen_path_builder.rs`) | fresh block: `emit_materialize_string_from_bytes` of the last component's span |
| r15 | `fs::pathDirName(path AS String) AS String` | `Body::abi_inline` → `lower_fs_path_dir_name_nl` (`gen_path_builder.rs`) | fresh block of the directory span (`emit_materialize_string_from_bytes`), **or a read-only constant**: `load_string_constant(".")` / `("/")` for a path with no directory part / the root (`gen_path_builder.rs`, labels `dot`, `root`) — see §3.2 finding F3 |
| r16 | `fs::pathExtension(path AS String) AS String` | `Body::abi_inline` → `lower_fs_path_extension_nl` (`gen_path_builder.rs`) | fresh block: `emit_materialize_string_from_bytes` of the extension span |
| r17 | `fs::pathNormalize(path AS String) AS String` | `Body::abi_inline` → `lower_fs_path_normalize_nl` (`gen_path_builder.rs`) | fresh block: the normalized bytes are assembled and materialized |
| r18 | `fs::readText(path AS String) AS String` | `Body::abi_function` → `lower_fs_read_text_path_helper` (`gen_atomic_write.rs`) | fresh block holding the file's bytes; `path` is copied into a scratch C path first (`{symbol}_path_copy_loop`) |
| r19 | `io::input([prompt AS String]) AS String` | `Body::abi_function` → `lower_read_line_family(with_prompt = true)` (`gen_read_line_family.rs`) | fresh block: a grown line buffer copied into the result (`result_copy_loop`); `prompt` is only written to stdout |
| r20 | `net::percentDecode(s AS String) AS String` | `Body::mfb` → MFBASIC `__net_percentDecode` → `__net_percentDecodeImpl` | fresh block: the body builds and `RETURN`s a new `String` |
| r21 | `os::getEnv(name AS String) AS String` | `Body::abi_function` → `gen_env::lower_get_env(with_fallback = false)` | fresh block holding the variable's value; `name` is copied into a scratch C string (`marshal_cstring`, bug-574 scratch, freed at `done`) |
| r22 | `os::getEnvOr(name AS String, fallback AS String) AS String` | `Body::abi_function` → `gen_env::lower_get_env(with_fallback = true)` | fresh block holding the value or a copy of `fallback`; `name` is copied into a scratch C string (`marshal_cstring`) |
| r23 | `os::resourcePath(relative AS String) AS String` | `Body::abi_function` → `lower_resource_path` (`os/func_resource_path.rs`) | fresh block: `base + "/" + relative` concatenated into an owned arena `String` |
| r24 | `regex::replace(value AS String, pattern AS String, replacement AS String) AS String` | `Body::mfb` → MFBASIC `__regex_replace` | fresh block: the body builds and `RETURN`s a new `String` |
| r25 | `strings::caseFold(value AS String) AS String` | `Body::abi_inline` → `gen_case_map::lower_strings_case_map` | fresh block: `emit_arena_alloc_call` of `byteLen + 9` on the all-ASCII path, or of the counted mapped length on the Unicode path (`gen_case_map.rs`) |
| r26 | `strings::graphemeAt(value AS String, index AS Integer) AS String` | `Body::abi_inline` → `func_grapheme_at::lower` | fresh block: `emit_materialize_string_from_bytes` of the grapheme's byte span |
| r27 | `strings::left(value AS String, count AS Integer) AS String` | `Body::abi_inline` → `gen_left_right::lower_strings_left_right` | fresh block: computes a `(ptr, len)` window into `value`, then `emit_materialize_string_from_bytes` (`builder_collection_layout.rs`) copies it |
| r28 | `strings::lower(value AS String) AS String` | `Body::abi_inline` → `gen_case_map::lower_strings_case_map` | fresh block: `emit_arena_alloc_call` of `byteLen + 9` on the all-ASCII path, or of the counted mapped length on the Unicode path (`gen_case_map.rs`) |
| r29 | `strings::mid(value AS String, start AS Integer, count AS Integer) AS String` | `Body::Intrinsic` → `native_builtin_target` = `mid` → `lower_mid` (`collection/search/builder_search.rs`, `String` branch) | fresh block: `emit_arena_alloc_call` (`mid_alloc_ok`) and a span copy |
| r30 | `strings::normalizeNfc(value AS String) AS String` | `Body::abi_inline` → `func_normalize_nfc::lower` | fresh block: `emit_arena_alloc_call` (three sites in `func_normalize_nfc.rs`) |
| r31 | `strings::padLeft(value AS String, width AS Integer, [padChar AS String]) AS String` | `Body::abi_inline` → `gen_pad::lower_strings_pad` | fresh block: `emit_arena_alloc_call` of the padded length (`gen_pad.rs`) |
| r32 | `strings::padLeftToWidth(value AS String, columns AS Integer, [padChar AS String]) AS String` | `Body::Rewrite("__strings_padLeftToWidth")` → MFBASIC helper (`helper_pad_to_width.rs`) | fresh block: the helper builds and `RETURN`s a new `String` |
| r33 | `strings::padRight(value AS String, width AS Integer, [padChar AS String]) AS String` | `Body::abi_inline` → `gen_pad::lower_strings_pad` | fresh block: `emit_arena_alloc_call` of the padded length (`gen_pad.rs`) |
| r34 | `strings::padRightToWidth(value AS String, columns AS Integer, [padChar AS String]) AS String` | `Body::Rewrite("__strings_padRightToWidth")` → MFBASIC helper (`helper_pad_to_width.rs`) | fresh block: the helper builds and `RETURN`s a new `String` |
| r35 | `strings::repeat(value AS String, times AS Integer) AS String` | `Body::abi_inline` → `func_repeat::lower` | fresh block: `emit_arena_alloc_call` of `len × times + 9` |
| r36 | `strings::replace(value AS String, old AS String, new AS String) AS String` | `Body::Intrinsic` → `native_builtin_target` = `replace` → `lower_replace` (`string/repr/builder_strings.rs`, `String` branch) | fresh block on both arms: `emit_arena_alloc_call` when a match is replaced, `copy_flat_block` of `value` when none is (bug-536 shape B) |
| r37 | `strings::right(value AS String, count AS Integer) AS String` | `Body::abi_inline` → `gen_left_right::lower_strings_left_right` | fresh block: computes a `(ptr, len)` window into `value`, then `emit_materialize_string_from_bytes` (`builder_collection_layout.rs`) copies it |
| r38 | `strings::stripPrefix(value AS String, prefix AS String) AS String` | `Body::abi_inline` → `gen_strip::lower_strings_strip` | fresh block: window into `value` (the whole of it when the affix is absent), then `emit_materialize_string_from_bytes` |
| r39 | `strings::stripSuffix(value AS String, suffix AS String) AS String` | `Body::abi_inline` → `gen_strip::lower_strings_strip` | fresh block: window into `value` (the whole of it when the affix is absent), then `emit_materialize_string_from_bytes` |
| r40 | `strings::trim(value AS String) AS String` | `Body::abi_inline` → `gen_trim::lower_strings_trim` | fresh block: `[start, end)` window into `value`, then `emit_materialize_string_from_bytes` |
| r41 | `strings::trimChars(value AS String, chars AS String) AS String` | `Body::abi_inline` → `func_trim_chars::lower` | fresh block: window into `value`, then `emit_materialize_string_from_bytes` |
| r42 | `strings::trimEnd(value AS String) AS String` | `Body::abi_inline` → `gen_trim::lower_strings_trim` | fresh block: `[start, end)` window into `value`, then `emit_materialize_string_from_bytes` |
| r43 | `strings::trimStart(value AS String) AS String` | `Body::abi_inline` → `gen_trim::lower_strings_trim` | fresh block: `[start, end)` window into `value`, then `emit_materialize_string_from_bytes` |
| r44 | `strings::upper(value AS String) AS String` | `Body::abi_inline` → `gen_case_map::lower_strings_case_map` | fresh block: `emit_arena_alloc_call` of `byteLen + 9` on the all-ASCII path, or of the counted mapped length on the Unicode path (`gen_case_map.rs`) |
| r45 | `strings::left(value AS AttributedString, count AS Integer) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_left` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_left` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r46 | `strings::right(value AS AttributedString, count AS Integer) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_right` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_right` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r47 | `strings::mid(value AS AttributedString, start AS Integer, count AS Integer) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_mid` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_mid` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r48 | `strings::trim(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_trim` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_trim` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r49 | `strings::trimStart(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_trimStart` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_trimStart` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r50 | `strings::trimEnd(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_trimEnd` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_trimEnd` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r51 | `strings::trimChars(value AS AttributedString, chars AS String) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_trimChars` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_trimChars` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r52 | `strings::stripPrefix(value AS AttributedString, prefix AS String) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_stripPrefix` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_stripPrefix` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r53 | `strings::stripSuffix(value AS AttributedString, suffix AS String) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_stripSuffix` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_stripSuffix` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r54 | `strings::padLeft(value AS AttributedString, width AS Integer, [padChar AS String]) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_padLeft` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_padLeft` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r55 | `strings::padLeftToWidth(value AS AttributedString, columns AS Integer, [padChar AS String]) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_padLeftToWidth` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_padLeftToWidth` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r56 | `strings::padRight(value AS AttributedString, width AS Integer, [padChar AS String]) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_padRight` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_padRight` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r57 | `strings::padRightToWidth(value AS AttributedString, columns AS Integer, [padChar AS String]) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_padRightToWidth` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_padRightToWidth` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r58 | `strings::repeat(value AS AttributedString, times AS Integer) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_repeat` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_repeat` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r59 | `strings::replace(value AS AttributedString, old AS String, new AS String) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_replace` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_replace` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r60 | `strings::upper(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_upper` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_upper` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r61 | `strings::lower(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_lower` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_lower` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r62 | `strings::caseFold(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_caseFold` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_caseFold` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r63 | `strings::normalizeNfc(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_normalizeNfc` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_normalizeNfc` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |

### B.3 The `String` representation facts the form column depends on

1. **The tight block.** A `String` is `{U64 byteLength, Byte[byteLength],
   U8 NUL}` and its allocation size is exactly `byteLength + 9`
   (`mfb spec memory heap-values`, "Standalone String";
   `src/docs/spec/memory/03_heap-values.md`). The arena free takes that size
   (`emit_owned_value_drop` sizes a `String` as `len + 9`, plus the capacity
   shadow when there is one — bug-560). So a block whose length changed in
   place must have its true size tracked somewhere, or the drop frees the wrong
   size: an in-place **shrink** leaves spare bytes that only a capacity shadow can
   describe, exactly as the self-append's **grow** does today. A `shrink` or
   `rewrite` arm therefore needs the capacity shadow (fact 2) at every site it
   runs, not just the self-append targets.
2. **The capacity shadow.** `prescan_string_self_appends` (`bc:2374`) allocates a
   frame slot `strcap_<name>` for every local that is the target of a `&`
   self-append anywhere in the function, unless the local is in
   `address_taken_locals` (a by-ref capture, `rt_byref_string_capture_capacity`).
   It holds the spare bytes past `byteLength` in the live block and is zeroed in
   the prologue; `reset_string_capacity_shadow` zeroes it on every other
   bind/assign (which installs a tight block); only the concat arm's regrow makes
   it non-zero (`lower_string_self_append_one`, geometric step, `bia:1688`);
   `string_capacity_slot_for` (`bc:2351`) hands it to the drop so the grown block
   is freed at its real size (bug-560). A global's shadow is the hidden global
   `$strcap$<name>` (`add_global_string_capacities`, `su`), declared only for a
   global `String` that is a `&` self-append target, reset to 0 by every other
   `StoreGlobal` (plan-142-H). The shadow never escapes: every copy, return or
   transfer reads `byteLength` bytes and freezes the value to the tight form.
3. **Read-only data.** A `String` literal and the Unicode property names are
   rodata. A `MUT` binding never holds one at the moment of a self-update: its
   bind goes through `lower_value_owned`, whose `value_needs_owning_copy`
   (`src/codegen/engine/value/builder_values.rs`) copies a `static_string_value`,
   a `call_returns_rodata_string` target, an aliasing source or a parameter
   borrow (probe `/tmp/plan-143-probes/rodata`: `bindOnly` carries
   `flat_copy_result`; a global's initializer is a `StoreGlobal` and takes the
   same copy). **Two producers escape that predicate** and hand an owning store a
   block the binding must not free — findings F1 (`toString(<String>)` returns
   its argument's block) and F3 (`fs::pathDirName` returns a rodata `.`/`/`) —
   plus the known bug-667 (`toString(<Boolean>)` returns rodata). A fix that
   writes into `s`'s block in place inherits every one of these: it must only
   run on a block `s` owns.
4. **How a `String` argument is passed.** Borrowed, never copied, for every §1
   row: a native lowering (`abi_inline`/`Intrinsic`) reads `args[0]`'s pointer
   directly; a `bl` to an MFBASIC body or an `abi_function` helper passes the
   pointer in the argument register (`mfb spec language memory-semantics` §14:
   "Native lowering passes a global argument without copying"). The exceptions
   are copies made **to protect** the borrow: `want_arguments_the_call_can_free`
   (`src/codegen/engine/value/operand_snapshot.rs`) snapshots a global argument
   when the call can reach a store to that global (bug-665), and the sibling
   rule `operand_reachable_by_later_call` snapshots an argument a *later*
   operand's call could reassign — observed at S2 and S9 for `addAttribute` and
   `removeAttribute`, whose later operand `astrings::bold()` is such a call
   (slot `operand_snapshot` in `r01_S2`, `r04_S2`, `$lambda0`, `$lambda3`).
   The `abi_function` helpers `fs::readText`, `fs::canonicalPath`, `os::getEnv`
   and `os::getEnvOr` additionally copy the argument's bytes into a scratch C
   string inside the helper (`marshal_cstring` / the path copy loop), freed
   before the helper returns.

## Appendix C — `--ncode` probes

All probes were built with the release compiler built in this worktree from
`efdb54bb7` (`target/release/mfb`), `mfb build --ncode <project>` at the default
optimization level, in `/tmp/plan-143-probes/`. Each project uses plan-141's
`project.json` (`kind: executable`, `entry: main`, `targets: [native]`).

**How a dump is read** (plan-141 Appendix C's method). Every function in the
`.ncode` JSON lists its `stackSlots` by type name. Each arm of
`SELF_UPDATE_ARMS` allocates a slot whose type name `ArmId::markers` records
(`su`), so the slot's presence in a function proves that arm fired there and its
absence proves the statement took another path. `su_global_block` /
`su_ref_block` mark that a `Global` / `Ref` self-update site was built (the seam
ran, whether or not an arm then fired); `strcap_<name>` / `concat_global_strcap`
mark a capacity shadow; `store_global_new` marks the `StoreGlobal` copying path.
Each case's S9 statement lowers in its lifted lambda: every S9 SUB creates exactly
one lambda and lambdas are numbered in source order, so case *k* (0-based) is
`$lambda<k>`.

**Reading vs dump.** `table.py` (C.5) derives each cell's verdict from the reading
(Appendix B) and refuses to print it unless the dump agrees: ARM absent at every
site; `su_global_block` present at S2 exactly for the 22 seam rows and
`store_global_new` present at every S2; `su_ref_block` present at S9 exactly for
the 22 seam rows. All 63 × 3 generated cells and the §2 rows agreed; there was no
disagreement to record.

### C.1 `markers.py`

```python
"""Per-function self-update markers from an `mfb build --ncode` dump (plan-143).

Usage: python3 markers.py <file.ncode> [function-name-prefix] [--slots]

plan-141 Appendix C.1's marker method, with the marker table taken from
`ArmId::markers` in src/codegen/collection/assign/self_update.rs at efdb54bb7
(every one of those slot type names occurs once in src/, so its presence in a
function proves that arm fired there). Prints per function:

  ARM     - the arm(s) whose marker slot is present, or `-` (no arm fired: the
            statement took the copying path);
  SEAM    - `su_global_block` (S2: `StoreGlobal` built a global SelfUpdateSite and
            ran the arms) / `su_ref_block` (S9: a by-ref local got a `Ref`
            destination), or `-`;
  STRCAP  - a `String` capacity shadow exists: a frame slot `strcap_<name>` or the
            global's `concat_global_strcap` working slot, or `-`;
  GLOBAL  - `store_global_new` present (the `StoreGlobal` copying path ran: fresh
            value, free of the old block, store);
  CALLS   - `bl` targets that are MFBASIC functions (`_mfb_fn_…`/`_mfb_ifn_…`):
            a `Body::Mfb`/`Rewrite` member lowers to one of these.

--slots also prints every stack-slot type name of each function.
"""
import json
import re
import sys

ARMS = {
    "inplace_append_item": "append",
    "inplace_bulk_append_rhs": "bulk_append",
    "inplace_set_add_item": "set_add",
    "inplace_set_index": "set",
    "inplace_set_key": "set",
    "inplace_remove_key": "remove_key",
    "inplace_prepend_item": "prepend",
    "inplace_remove_at_index": "remove_at",
    "inplace_insert_index": "insert",
    "inplace_set_remove_item": "set_remove",
    "concat_self_right": "concat",
    "inplace_filter_action": "filter",
    "inplace_take_count": "take",
    "inplace_drop_count": "drop",
    "inplace_mid_start": "mid",
    "inplace_distinct_count": "distinct",
    "inplace_math_result": "math",
    "inplace_replace_old": "replace",
    "inplace_transform_action": "transform",
    "inplace_sort_count": "sort",
    "inplace_sortby_action": "sortBy",
    "inplace_union_other": "union",
    "inplace_intersection_other": "intersection",
    "inplace_difference_other": "difference",
    "inplace_symmetricDifference_other": "symmetricDifference",
    "inplace_merge_prefer": "merge",
    "inplace_mapvalues_action": "mapValues",
}

args = [a for a in sys.argv[1:] if not a.startswith("--")]
show_slots = "--slots" in sys.argv
path = args[0]
prefix = args[1] if len(args) > 1 else ""
text = open(path).read()
data = json.loads(text[text.index("{"):])


def walk(node, out):
    if isinstance(node, dict):
        if "stackSlots" in node and "name" in node:
            out.append(node)
        for v in node.values():
            walk(v, out)
    elif isinstance(node, list):
        for v in node:
            walk(v, out)


functions = []
walk(data, functions)
for fn in functions:
    name = fn["name"]
    if not name.startswith(prefix):
        continue
    types = {s["type"] for s in fn.get("stackSlots", [])}
    arms = sorted({ARMS[t] for t in types if t in ARMS})
    seam = ",".join(t for t in ("su_global_block", "su_ref_block") if t in types) or "-"
    strcap = ",".join(sorted(t for t in types
                             if t.startswith("strcap_") or t == "concat_global_strcap")) or "-"
    glob = "y" if "store_global_new" in types else "-"
    body = json.dumps(fn)
    calls = sorted({c for c in re.findall(r'"target": "(_mfb_i?fn_[^"]+)"', body)})
    print(f"{name}: ARM={','.join(arms) or '-'} SEAM={seam} STRCAP={strcap} GLOBAL={glob} "
          f"CALLS={','.join(calls) or '-'}")
    if show_slots:
        print("   slots:", " ".join(sorted(types)))
```

### C.2 The self-update probe — generator and result

`gen.py` writes `str/src/main.mfb`: one SUB per (case, site) for S1, S2 and S9,
67 cases (the 63 §1 rows, then the four §2 forms `o1`–`o4`) = 201 SUBs. It built
without diagnostics. `join.py` prints each case's markers; every §1/§2 cell's
marker line is quoted in its evidence column.

```python
"""Generate /tmp/plan-143-probes/str/src/main.mfb: one SUB per (row, site).

Rows are the 63 §1 overloads (rows.md, in table order) followed by the §2 forms.
SUB names are `r<NN>_<site>` (NN = 1-based row index, `o<N>` for §2 forms);
markers.py then reports, per SUB (and, for S9, per lifted lambda), which
in-place arm fired, whether the self-update seam ran, and whether the copying
StoreGlobal path ran. Every SUB prints the binding so the update is live.

Sites: S1 local, S2 global, S9 by-ref capture in a `collections::forEach`
lambda. S7 is not generated: a `String`/`AttributedString` is not a `FOR EACH`
iterable (probe `s7/`).
"""
import os
import re

ROWS = [re.match(r"\| `([^`]+)`", l).group(1)
        for l in open("/tmp/plan-143-probes/rows.md") if l.startswith("| `")]

T = {
    "caseFold": "strings::caseFold({X})",
    "graphemeAt": "strings::graphemeAt({X}, 0)",
    "left": "strings::left({X}, 2)",
    "lower": "strings::lower({X})",
    "mid": "strings::mid({X}, 0, 2)",
    "normalizeNfc": "strings::normalizeNfc({X})",
    "padLeft": "strings::padLeft({X}, 12)",
    "padLeftToWidth": "strings::padLeftToWidth({X}, 12)",
    "padRight": "strings::padRight({X}, 12)",
    "padRightToWidth": "strings::padRightToWidth({X}, 12)",
    "repeat": "strings::repeat({X}, 2)",
    "replace": "strings::replace({X}, \"b\", \"zz\")",
    "right": "strings::right({X}, 2)",
    "stripPrefix": "strings::stripPrefix({X}, \" \")",
    "stripSuffix": "strings::stripSuffix({X}, \" \")",
    "trim": "strings::trim({X})",
    "trimChars": "strings::trimChars({X}, \" \")",
    "trimEnd": "strings::trimEnd({X})",
    "trimStart": "strings::trimStart({X})",
    "upper": "strings::upper({X})",
}
PKG = {
    "astrings::addAttribute": "astrings::addAttribute({X}, 0, 1, astrings::bold())",
    "astrings::clearAttributes/1": "astrings::clearAttributes({X})",
    "astrings::clearAttributes/3": "astrings::clearAttributes({X}, 0, 1)",
    "astrings::removeAttribute": "astrings::removeAttribute({X}, 0, 1, astrings::bold())",
    "encoding::formUrlDecode": "encoding::formUrlDecode({X})",
    "encoding::formUrlEncode": "encoding::formUrlEncode({X})",
    "encoding::htmlEscape": "encoding::htmlEscape({X})",
    "encoding::htmlUnescape": "encoding::htmlUnescape({X})",
    "encoding::percentDecode": "encoding::percentDecode({X})",
    "encoding::percentEncode": "encoding::percentEncode({X})",
    "encoding::punycodeDecode": "encoding::punycodeDecode({X})",
    "encoding::punycodeEncode": "encoding::punycodeEncode({X})",
    "fs::canonicalPath": "fs::canonicalPath({X})",
    "fs::pathBaseName": "fs::pathBaseName({X})",
    "fs::pathDirName": "fs::pathDirName({X})",
    "fs::pathExtension": "fs::pathExtension({X})",
    "fs::pathNormalize": "fs::pathNormalize({X})",
    "fs::readText": "fs::readText({X})",
    "io::input": "io::input({X})",
    "net::percentDecode": "net::percentDecode({X})",
    "os::getEnv": "os::getEnv({X})",
    "os::getEnvOr": "os::getEnvOr({X}, \"d\")",
    "os::resourcePath": "os::resourcePath({X})",
    "regex::replace": "regex::replace({X}, \"b\", \"z\")",
}
# §2: (id, type, template)
OPS = [
    ("o1", "String", "{X} & t"),
    ("o2", "String", "{X} & t & u"),
    ("o3", "AttributedString", "{X} & astrings::fromString(t)"),
    ("o4", "String", "toString({X})"),
]


def template(sig):
    fq = re.match(r"(\w+::\w+)\(", sig).group(1)
    pkg, name = fq.split("::")
    if pkg == "strings":
        return T[name]
    if name == "clearAttributes":
        return PKG[f"{fq}/{1 if 'start' not in sig else 3}"]
    return PKG[fq]


cases = []
for i, sig in enumerate(ROWS, 1):
    ty = "AttributedString" if "(value AS AttributedString" in sig else "String"
    cases.append((f"r{i:02d}", ty, template(sig)))
cases += OPS

INIT = {"String": "mk()", "AttributedString": "astrings::fromString(mk())"}
GINIT = {"String": "\"\"", "AttributedString": "astrings::fromString(\"\")"}

out = ["IMPORT io", "IMPORT strings", "IMPORT astrings", "IMPORT encoding",
       "IMPORT fs", "IMPORT os", "IMPORT net", "IMPORT regex", "IMPORT collections", ""]
for cid, ty, _ in cases:
    out.append(f"MUT g_{cid} AS {ty} = {GINIT[ty]}")
out += ["", "FUNC mk() AS String", "  RETURN \" a b c \" & toString(len(os::args()))",
        "END FUNC", ""]
for cid, ty, op in cases:
    pre = ["LET t AS String = mk()", "LET u AS String = mk()"]
    out += [f"SUB {cid}_S1()"] + ["  " + l for l in pre] + [
        f"  MUT s AS {ty} = {INIT[ty]}",
        f"  s = {op.format(X='s')}",
        "  io::print(s)", "END SUB", ""]
    out += [f"SUB {cid}_S2()"] + ["  " + l for l in pre] + [
        f"  g_{cid} = {op.format(X=f'g_{cid}')}",
        f"  io::print(g_{cid})", "END SUB", ""]
    out += [f"SUB {cid}_S9()"] + ["  " + l for l in pre] + [
        f"  MUT s AS {ty} = {INIT[ty]}",
        f"  collections::forEach([1], LAMBDA(v AS Integer) -> s = {op.format(X='s')})",
        "  io::print(s)", "END SUB", ""]
out += ["FUNC main() AS Integer"]
for cid, _, _ in cases:
    for site in ("S1", "S2", "S9"):
        out.append(f"  {cid}_{site}()")
out += ["  RETURN 0", "END FUNC", ""]

os.makedirs("/tmp/plan-143-probes/str/src", exist_ok=True)
open("/tmp/plan-143-probes/str/src/main.mfb", "w").write("\n".join(out))
print(len(cases), "cases,", 3 * len(cases), "SUBs")
```

```python
"""Join markers.txt to the probe cases: one line per case with S1, S2 and S9
(the S9 statement lowers in the lifted lambda `$lambda<k>`, k = the case's
0-based index, since each case's S9 SUB creates exactly one lambda and lambdas
are numbered in source order; the S9 SUB itself is also printed)."""
import re

rows = [re.match(r"\| `([^`]+)`", l).group(1)
        for l in open("/tmp/plan-143-probes/rows.md") if l.startswith("| `")]
ids = [f"r{i:02d}" for i in range(1, len(rows) + 1)] + ["o1", "o2", "o3", "o4"]
names = rows + ["s = s & t", "s = s & t & u", "a = a & b (AttributedString)", "s = toString(s)"]
m = {}
for line in open("/tmp/plan-143-probes/str/markers.txt"):
    k, v = line.split(": ", 1)
    m[k] = v.strip()
for k, (cid, name) in enumerate(zip(ids, names)):
    print(f"{cid} {name}")
    print(f"   S1  {m[cid + '_S1']}")
    print(f"   S2  {m[cid + '_S2']}")
    print(f"   S9  sub {m[cid + '_S9']}")
    print(f"   S9  $lambda{k} {m['$lambda' + str(k)]}")
```

### C.3 Runtime aliasing sweep (added; Correction 3)

`--ncode` shows which path a statement takes, not whether that path is sound, and
the plan asks whether a `not-derived` row copies `s`. `rt_gen.py` builds every
(case, site) as a runnable SUB that computes the expected result from an
independent copy first, runs the self-update, allocates a same-sized junk string
(which reuses the block if the update freed the one it kept), and compares;
`rt_run.sh` runs each in its own process (stdin: eight identical lines, so
`io::input` is deterministic). Result: **198 of 201 runs pass**; the
failures are exactly finding F1:

```
o4_S1 exit=139 BAD o4_S1 got=[#############] want=[ a<b> c%20d 1] junk=13
o4_S2 exit=139 BAD o4_S2 got=[#############] want=[ a<b> c%20d 1] junk=13
o4_S9 exit=139 BAD o4_S9 got=[#############] want=[ a<b> c%20d 1] junk=13
```

Before the per-case input overrides in `INIT_OVERRIDE` were added, two more rows
failed for reasons outside the self-update: `fs::pathDirName(" a<b> c%20d 1")`
died with SIGBUS at every site — finding F3, reproduced alone by
`/tmp/plan-143-probes/dirname` (`LET a AS String = fs::pathDirName("abc")` prints
`[.]` then exits 138) — and `os::getEnv` of an unset name raised `ErrNotFound`
(documented behaviour). With a directory in the path, `pathDirName`'s self-update
passes at all three sites.

```python
"""Runtime aliasing sweep: for every probe case at S1, S2 and S9, compute the
expected result from an independent copy first, run the self-update, allocate a
same-sized junk string (which reuses a freed block if the update freed the one it
kept), and compare. Prints `BAD <case>_<site> ...` on a mismatch, `ok N` at end.

Reuses gen.py's case table. Cases whose builtin needs a real environment get a
case-specific initial value (INIT_OVERRIDE)."""
import os
import re
import sys

sys.argv = ["gen.py"]
src = open("/tmp/plan-143-probes/gen.py").read()
ns = {}
exec(src.split("INIT = {")[0], ns)  # the case table only, not the writer
cases = ns["cases"]

INIT_OVERRIDE = {
    "fs::readText": "\"/etc/hosts\"",
    "fs::canonicalPath": "\"/tmp/../tmp\"",
    "io::input": "\"\"",
    "os::getEnv(": "\"HOME\"",
    "fs::pathDirName": "\"/a/b/c\"",
}


def init_for(ty, op):
    for k, v in INIT_OVERRIDE.items():
        if op.startswith(k):
            return v
    return "mk()" if ty == "String" else "astrings::fromString(mk())"


def text(ty, expr):
    return expr if ty == "String" else f"toString({expr})"


out = ["IMPORT io", "IMPORT strings", "IMPORT astrings", "IMPORT encoding",
       "IMPORT fs", "IMPORT os", "IMPORT net", "IMPORT regex", "IMPORT collections", ""]
for cid, ty, op in cases:
    init = init_for(ty, op)
    g_init = "\"\"" if ty == "String" else "astrings::fromString(\"\")"
    out.append(f"MUT g_{cid} AS {ty} = {g_init}")
out += ["MUT bad AS Integer = 0", "",
        "FUNC mk() AS String", "  RETURN \" a<b> c%20d \" & toString(len(os::args()))",
        "END FUNC", "",
        "SUB check(id AS String, got AS String, want AS String)",
        "  LET junk AS String = strings::repeat(\"#\", len(want))",
        "  IF got <> want THEN",
        "    bad = bad + 1",
        "    io::print(\"BAD \" & id & \" got=[\" & got & \"] want=[\" & want & \"] junk=\" & toString(len(junk)))",
        "  END IF",
        "END SUB", ""]
for cid, ty, op in cases:
    init = init_for(ty, op)
    pre = ["LET t AS String = mk()", "LET u AS String = mk()",
           f"LET w0 AS {ty} = {init}", f"LET want AS String = {text(ty, op.format(X='w0'))}"]
    for site in ("S1", "S2", "S9"):
        out.append(f"SUB {cid}_{site}()")
        out += ["  " + l for l in pre]
        if site == "S1":
            out += [f"  MUT s AS {ty} = {init}", f"  s = {op.format(X='s')}"]
            target = "s"
        elif site == "S2":
            out += [f"  g_{cid} = {init}", f"  g_{cid} = {op.format(X=f'g_{cid}')}"]
            target = f"g_{cid}"
        else:
            out += [f"  MUT s AS {ty} = {init}",
                    f"  collections::forEach([1], LAMBDA(v AS Integer) -> s = {op.format(X='s')})"]
            target = "s"
        out += [f"  LET junk AS String = strings::repeat(\"#\", len(want))",
                f"  check(\"{cid}_{site}\", {text(ty, target)}, want)", "END SUB", ""]
out += ["FUNC main() AS Integer", "  LET which AS String = collections::get(os::args(), 0)"]
for cid, _, _ in cases:
    for site in ("S1", "S2", "S9"):
        out.append(f"  IF which = \"{cid}_{site}\" THEN")
        out.append(f"    {cid}_{site}()")
        out.append("  END IF")
out += ["  io::print(\"done \" & which & \" bad=\" & toString(bad))", "  RETURN 0", "END FUNC", ""]
os.makedirs("/tmp/plan-143-probes/rt/src", exist_ok=True)
open("/tmp/plan-143-probes/rt/src/main.mfb", "w").write("\n".join(out))
print(len(cases), "cases")
```

```sh
#!/bin/sh
# Run every (case, site) of the runtime aliasing sweep in its own process.
# Prints one line per run: `<id> exit=<code> <last output line>`.
B=/tmp/plan-143-probes/rt/build/probe.out
IN=/tmp/plan-143-probes/rt/stdin.txt
for c in $(grep -oE '^SUB (r[0-9]+|o[0-9]+)_S[0-9]' /tmp/plan-143-probes/rt/src/main.mfb | sed 's/^SUB //'); do
  out=$("$B" "$c" < "$IN" 2>&1)
  code=$?
  echo "$c exit=$code $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-240)"
done
```

### C.4 Single-question probes

| probe | question | result |
|---|---|---|
| `try` | do the Tier-B `AttributedString` self-updates, `a = a & b` and `s = toString(s)` compile? | yes; `a & b` lowers to `_mfb_ifn_astrings_5Fconcat` (ARM=-) |
| `s7`, `s7a` | can a `String` / `AttributedString` be a `FOR EACH` source? | no: `error[2-203-0050 TYPE_FOR_EACH_REQUIRES_COLLECTION]` for both |
| `rodata` | does a `MUT` bind of a literal copy it? is `toString(s)` copied at the store? | `bindOnly` carries `flat_copy_result` (copied); `toStr` has no `flat_copy_*` slot (F1); `bindThenConcat` ARM=concat, `globalConcat` ARM=concat + `concat_global_strcap` |
| `tostr` | does `s = toString(s)` alias freed memory? | yes: prints `XYZWXYZW…` (the next allocation) for `s`, instead of `abcdefgh…` |
| `dirname` | does `fs::pathDirName("abc")` survive? | no: prints `[.]`, exit 138 (SIGBUS) |
