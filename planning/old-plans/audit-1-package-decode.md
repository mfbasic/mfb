# Audit 1 — .mfp Package Decode & Verification

Scope: the untrusted-input decode path for compiled `.mfp` packages consumed by
the compiler at resolve/build time — `src/binary_repr/{reader,util,sections,mod}.rs`
(the MFPC container + section decoders), `src/ir/binary.rs` (the structured
Binary Representation / function-body decoder), the merge/verify seam in
`src/target/shared/nir/lower.rs` + `src/ir::verify_package`, the NIR structural
validator `src/target/shared/validate.rs`, and the signature/crypto surface in
`repository/src/{crypto,package.rs}` + `src/cli/pkg.rs`. The attacker model is a
malicious `packages/<name>.mfp` file dropped into a project the victim then
`mfb build`s (or `mfb pkg`s). Findings are ordered by severity.

---

## PKG-01 — CRITICAL: Compiler never verifies the package Ed25519 signature or content hash at import/build time

**Location:** `src/binary_repr/mod.rs:363` (`read_package_ir_with_identity`), `src/binary_repr/reader.rs:163` (`read_package_binary_repr`), `src/target/shared/nir/lower.rs:82`, `src/cli/pkg.rs:504-519` (`verify_package_dependency`); crypto that *is not called* on this path lives in `repository/src/package.rs:104` (`verify_package_signature`) and `repository/src/crypto.rs:35` (`verify`).

**Issue:** The import path decodes the container and reads the signed bytes purely as a *region to skip*. `mfp_binary_repr_payload` (reader.rs:189-197) reads `signature_type`/`signature_length`, validates only the *shape* via `validate_mfp_signature_header` (`(0,0) | (1,64)`), then advances the cursor past the signature and discards it — the 64 Ed25519 bytes are never checked against any key, and `package_content_hash` is never recomputed and compared. `read_package_ir_with_identity` (mod.rs:363-378) — the entry point `merge_packages` uses at build time — does the same and then hands the IR straight to lowering. `verify_package_signature`/`crypto::verify` exist only in the `repository` crate and are invoked at registry *publish/download*, not by the compiler. `mfb pkg verify` (cli/pkg.rs:504) only compares header `name/ident/version` strings; it too never calls `verify_package_signature`. The spec even states this explicitly (`src/docs/spec/package/01_container-format.md:228`: "It does **not** verify the cryptographic signature").

Concretely a package with `signature_type = 0, signature_length = 0` (unsigned) is accepted with zero cryptographic checks, and a package with a *tampered* body but a stale/forged 64-byte blob (`type = 1, length = 64`) is also accepted because the blob is never verified. The `identKey`/`identFingerprint`/`signingFingerprint` in the container are only cross-checked against the *manifest section inside the same file* (`validate_container_manifest_identity`, reader.rs:242) — a self-referential check the attacker fully controls on both sides.

**Trigger:** Attacker writes any bytes they like into `packages/evil.mfp` (or MITMs a download / compromises a mirror). Victim project imports it:
```
IMPORT evil AS evil
FUNC main AS Integer
  RETURN evil::run()
END FUNC
```
`mfb build` decodes and lowers the attacker's IR with no signature gate. Combined with PKG-02, the attacker's IR is also unverified for type/resource safety.

**Fix:** Add a real verification step in `read_package_ir_with_identity` and `read_package_binary_repr` (and `read_package_type_exports`, since resolve consumes it first). Reuse `repository::package::{parse_mfp_package, package_content_hash, verify_package_signature}` semantics inside the compiler: (a) recompute the SHA-256 content hash over `[..26] ++ zeroed-signature ++ [signature_end..]` and (b) if `signature_type == 1`, verify the Ed25519 signature over `package_signature_message(hash, ident, version)` using a project-configured trust anchor (the `identKey` pinned in the importing project's `project.json packages[]` entry, not the key embedded in the untrusted file). Do not trust the file-embedded fingerprints as the verification key.

**Verification logging + build gate:** Every imported package must be verified (no package is skipped), and the compiler must emit one line per package recording its verification state:

```
uses <package name> - [Verified | Unsigned | Tampered]
```

- **Verified** — `signature_type == 1`, the recomputed content hash matches, and the Ed25519 signature verifies against the project-pinned trust anchor.
- **Unsigned** — `signature_type == 0` (no signature present). Not fatal by itself; policy still applies: reject unsigned packages during `mfb build` unless the project explicitly opts into an unsigned local dependency (`source: "local://…"`) with a --unsigned CLI option.
- **Tampered** — `signature_type == 1` but the content hash mismatches or the signature fails to verify (or the shape/header is otherwise inconsistent with a valid signed package).

Check **all** packages first (accumulate a per-package result and log each line) rather than aborting on the first failure, so the operator sees the full report. If **any** package is classified **Tampered**, the compile must stop with a non-zero exit after the report is printed — a tampered dependency is a hard error, never a warning.

---

## PKG-02 — CRITICAL: Decoded package IR is trusted for typing / resource-linearity / Result-handling — only structural name checks are re-run

**Location:** `src/ir/binary.rs:1079` (`verify_package`) + `:1114` (`verify_ops`); `src/target/shared/nir/lower.rs:85`; `src/target/shared/validate.rs:23` (`validate_project` is a no-op) and `:27` (`validate_nir`); the comment admitting the gap is `src/target/shared/validate.rs:137-140` ("the type checker is the primary enforcer; this guards against a malformed NIR").

**Issue:** The source-level type checker (`src/typecheck/`) runs on the *project's own AST*, never on the IR decoded from an imported `.mfp`. The only re-verification applied to decoded package IR is `verify_package` (binary.rs:1079), which checks exactly three things: function names non-empty, function/type names unique, and every `MATCH` has ≥1 case (`verify_ops`, binary.rs:1128). It performs **no** type checking, **no** move/ownership (resource linearity) checking, **no** Result-handling exhaustiveness, and **no** argument-count/kind checking. The merged IR then flows through `lower.rs` to NIR; `validate_nir` (validate.rs:27) is purely structural (name resolution, unique locals, mutability, visibility strings) and by its own comment assumes the type checker already ran. `validate_project` is `Ok(())`.

Because codegen assumes well-typed IR (e.g. `IrValue::MemberAccess` field offsets, `Constructor` arg layout, `UnionExtract`/`UnionWrap` tag/payload sizing, `Binary`/`Unary` operand types, `Capture { index }` closure-slot indices), a malicious package can emit type-confused IR — a record `MemberAccess` on an `Integer`, a `Constructor` with the wrong arg count/types, a `Capture` index past the closure's captured slots, a resource used non-linearly (double-free / use-after-close), or a `Result` unwrapped without an Ok check — producing memory-unsafe native code (OOB read/write, arbitrary field-offset dereference) in the victim's binary. This is exactly the "malicious .mfp bypassing source checks" scenario, and it is fully open.

**Trigger:** Attacker hand-builds (or patches) a `.mfp` whose `SECTION_BINARY_REPR` encodes, e.g., an exported `FUNC` whose body does `IrValue::MemberAccess { target: <Integer local>, member: "anything" }`, or `IrValue::Capture { index: 9999, .. }`, or reuses a `File`/`Socket` resource local after it was moved into `fs::close`. The victim imports and calls the export (see PKG-01 trigger). Since no typecheck runs on the decoded IR, it lowers and codegens.

**Fix:** Run the real semantic checks on decoded package IR before merge. The cleanest implementation-only path: after `read_package_ir_with_identity`, reconstruct enough of the type environment from the merged IR (types + function signatures) and run the existing typechecker / resource-linearity / result-exhaustiveness passes over the imported functions (adapt `src/typecheck/` to accept an `IrProject`/`IrFunction` instead of AST, or add an IR-level verifier that mirrors those rules). At minimum, extend `verify_package`/`verify_ops` to enforce: every `Capture.index` within bounds, `Constructor`/`Call` arg counts match the referenced type/function, `MemberAccess`/`WithUpdate` fields exist on the target's declared record/union type, `UnionWrap.member_type` is a real variant of `union_type`, and resource locals are used linearly. This changes no MFBASIC surface syntax — it is a verifier addition.

---

## PKG-03 — HIGH: Unbounded recursion in the Binary Representation body decoder (`decode_op` / `decode_value`) → stack-overflow DoS

**Location:** `src/ir/binary.rs:659` (`decode_op`), `:953` (`decode_value`), `:1114` (`verify_ops`), and the mutual recursion via `decode_vec` (`:407`).

**Issue:** `decode_op` recurses into nested bodies (`If.then_body/else_body`, `While/ForEach/Trap/For/DoUntil` bodies via `decode_vec(r, decode_op)`) and `decode_value` recurses into itself (`Binary.left/right`, `Unary.operand`, `UnionWrap/UnionExtract/ResultIsOk/…`, `MemberAccess.target`, `Closure.captures`, nested `Call.args`, `ListLiteral`, `MapLiteral`). None of these carry a recursion-depth limit; each nested level is one native stack frame. A payload of a few hundred KB expressing a deeply nested expression (e.g. tens of thousands of nested `Unary`/`Binary` or nested `IF`) overflows the thread stack and aborts the process (SIGSEGV/abort) — a decode-time DoS that happens *before* PKG-02's structural checks and before any codegen. `verify_ops` (binary.rs:1114) recurses over the same tree with the same unbounded depth, as does the `decode_type_name` graph (see PKG-04).

**Trigger:** A crafted `.mfp` whose `SECTION_BINARY_REPR` encodes one exported function whose body is `IrValue::Unary { op:"-", operand: Unary { operand: Unary { … } } }` nested ~50k deep (each level is ~7 bytes: tag `19` + `put_str("-")` + inner). `mfb build` (or `mfb pkg info/doc`, which also decode) crashes on decode.

**Fix:** Thread a depth counter through `IrReader` (increment on entry to `decode_op`/`decode_value`/`decode_link_expr`, decrement on exit, error past a fixed cap such as MAX_DECODE_DEPTH = 256 with a `PACKAGE_BINARY_REPRESENTATION_DECODE_FAILED`-style message). Apply the same cap to `verify_ops`. This is a decoder-internal limit; no language change.

---

## PKG-04 — HIGH: `decode_type_name` has no cycle guard and no depth limit → stack-overflow on self/mutually-referential type payloads

**Location:** `src/binary_repr/reader.rs:600` (`decode_type_name`), `:686` (`read_payload_type`), `:580` (`type_entry_names` driving it), `:666` (`decode_function_type`).

**Issue:** `decode_type_name` resolves composite type names (List/Map/Result/Thread/Function/MapEntry) by recursively decoding the type ids stored in a type entry's `payload`. It memoizes into `decoded` only *after* the recursive call returns (reader.rs:662) and has no "in progress" marker, so a payload that references its own id — or two entries that reference each other — recurses forever until the stack overflows. Contrast `AbiSerializer::serialize_type` (reader.rs:1194-1218), which *does* guard cycles with a `type_refs` map assigned *before* recursing; `decode_type_name` lacks that guard. `read_type_entries` (reader.rs:537) accepts arbitrary `payload` bytes for each entry with no constraint that a composite type's referenced ids are smaller/acyclic, and `type_entry_names` calls `decode_type_name` for every id.

**Trigger:** A crafted `TYPE_TABLE` (section id 3) with one entry of `kind = 4` (List) whose 4-byte payload is the little-endian encoding of the entry's own id (`FIRST_TABLE_TYPE_ID = 10`, i.e. bytes `0a 00 00 00`). Decoding this package (during resolve, `read_package_type_exports`) recurses `List OF List OF …` forever → stack overflow. Two mutually-referential entries (id10 payload→11, id11 payload→10) do the same.

**Fix:** Give `decode_type_name` the same cycle guard as `AbiSerializer::serialize_type`: insert a placeholder (or an `in_progress: HashSet<u32>`) *before* recursing and return an error (`unknown/cyclic type id`) if an id is re-entered. Optionally also add a depth cap. Verifier-only change.

---

## PKG-05 — MEDIUM: Allocation pre-sized from attacker-controlled counts (`Vec::with_capacity(count)`) → memory-exhaustion before validation

**Location:** many count-driven sites, e.g. `src/ir/binary.rs:412` (`decode_vec`: `Vec::with_capacity(n)` where `n = r.u32()`), `src/binary_repr/reader.rs:515` (`read_string_pool`), `:551` (`read_type_entries`), `:724` (`read_function_table`), `:801` (`read_const_pool`), `:952`/`:967`/`:978` (`read_abi_index`), `:879` (`read_used_symbols`), `:892` (`read_resource_table` — note it has *no* trailing-byte check), `:923` (`read_export_table`), `src/binary_repr/util.rs:32`/`:45` (prose/pair lists).

**Issue:** Each of these reads a `u32` count directly from the untrusted stream and immediately does `Vec::with_capacity(count as usize)`. `count` can be up to `0xFFFF_FFFF` (~4.29e9). For `decode_vec` the element type can be large (`IrOp`, `IrValue`, `IrLinkFunction`), so a single 4-byte count of `0xFFFFFFFF` requests a multi-gigabyte-to-hundreds-of-GB allocation up front — before any per-element bounds are read — aborting or OOM-killing the compiler. Several readers (e.g. `read_type_entries`, `read_function_table`) *do* bound the count against a fixed per-entry stride (`count * 20`, `count * 24`) which limits the header size, but `Vec::with_capacity` for the *decoded* struct vec and `decode_vec` (no stride bound at all) are not so protected. `read_resource_table` also lacks the trailing-byte consistency check the sibling readers have.

**Trigger:** A `.mfp` whose `SECTION_BINARY_REPR` `functions` count (or any nested `decode_vec` count, or the string-pool/const-pool count) is `0xFFFFFFFF` with a tiny actual body. Decode attempts a huge `with_capacity` and aborts.

**Fix:** Replace `Vec::with_capacity(count)` with a bounded reserve: cap the pre-allocation to `min(count, remaining_bytes / MIN_ELEMENT_SIZE)` (elements can never be smaller than their minimum on-wire size, so `remaining_bytes` bounds the real element count), or simply `Vec::new()` + `reserve` incrementally / cap capacity to a sane constant. Add a helper on `IrReader` (`fn count_bounded(&self, min_elem: usize)`) and use it everywhere. Add the missing `offset != bytes.len()` trailing check to `read_resource_table`.

---

## PKG-06 — MEDIUM: Duplicate MFPC section IDs are silently accepted (last one wins); singleton/required-section uniqueness is not enforced

**Location:** `src/binary_repr/reader.rs:294-307` (`read_binary_repr_package` section-table loop), specifically `sections.insert(id, &bytes[offset..end]);` at `:306`.

**Issue:** The section table is decoded into a `HashMap<u16, &[u8]>` via `sections.insert(...)`, which silently overwrites on a duplicate id. There is no check that each singleton section (STRING_POOL, TYPE_TABLE, CONST_POOL, FUNCTION_TABLE, BINARY_REPR, MANIFEST, IMPORT_TABLE, ABI_INDEX) appears exactly once. An attacker can therefore ship two `SECTION_BINARY_REPR` entries (or two `SECTION_ABI_INDEX`): the *first* may satisfy a cheap validator/inspector while the *second* (the one `HashMap` keeps) carries the payload actually decoded and lowered — a decode/verify desync. Because `validate_abi_index` (reader.rs:1001) is computed over whichever ABI/string/type sections survived the map, a mismatched pair of duplicate sections can be used to make the ABI-consistency check pass over one view while the body decoded from another. It also enables ambiguity between tooling that scans the raw section table (e.g. `mfb pkg info`) and the reader.

**Trigger:** Craft an MFPC payload with `section_count = 2` both `id = SECTION_BINARY_REPR`; the reader keeps the second. More generally, duplicate any singleton section to present two different views.

**Fix:** In the section-table loop, reject duplicate ids: `if sections.insert(id, slice).is_some() { return Err("duplicate MFPC section <id>"); }`. After the loop, require the mandatory singleton sections are present (already partly done via `ok_or_else`) and that no unknown/duplicate ids smuggle a second copy. SHould be concidered **Tampered** with signed or unsigned packages.

---

## PKG-07 — LOW: `IrReader::need` uses unchecked `self.pos + n` (usize add) instead of a checked/saturating comparison

**Location:** `src/ir/binary.rs:83-93` (`need`), reached by every `u8/u16/u32/string` read.

**Issue:** `need` computes `if self.pos + n > self.bytes.len()`. `n` for a string read comes from an attacker `u32` (`string()` at binary.rs:126: `let len = self.u32()? as usize; self.need(len)?`). On a 64-bit host `pos ≤ bytes.len() ≤ isize::MAX` and `n ≤ u32::MAX`, so `pos + n` cannot overflow `usize` in practice, making this only a latent/defensive issue rather than a live overflow (hence LOW). It is still fragile: the same pattern in `src/binary_repr/util.rs:171/179/187` (`checked_u16_at`/`checked_u32_at`/`checked_u64_at` use `offset + 2/4/8`) relies on the same "offset stays small" invariant.

**Trigger:** Not independently exploitable given current 64-bit offset bounds; noted for defense-in-depth and 32-bit-target safety.

**Fix:** Use `self.pos.checked_add(n).map_or(true, |end| end > self.bytes.len())` in `need`, and `bytes.get(offset..offset.checked_add(2)?)`-style reads in `util.rs` (or `offset.checked_add(N)`), so the bound is overflow-safe on all targets.

---

## Checked and OK

- **Container/section length arithmetic (reader.rs `mfp_binary_repr_payload`, `read_binary_repr_package`):** offset/length combinations use `checked_add`/`checked_mul` and compare against `bytes.len()` before slicing (reader.rs:192-216, 282-307, 560-565). No unchecked `offset+length` slice on the container framing itself. Good.
- **String-pool / length-prefixed reads:** `read_string_pool` (reader.rs:512), `cursor_string`/`read_length_prefixed` (util.rs:154, 94) validate UTF-8 and bounds with `checked_add` + `get(..)`; malformed → clean `Err`, not panic.
- **`string_at` / `type_name`:** use `.get(id as usize)` and return `Err` on out-of-range string/type ids (reader.rs:1132, 1122) rather than indexing/panicking.
- **`function_sig_hash` / `AbiSerializer::serialize_type`:** the ABI serializer guards type-graph cycles with `type_refs` assigned before recursing and bounds `next_ref` with `checked_add` (reader.rs:1202-1218) — cycle- and overflow-safe (unlike `decode_type_name`, see PKG-04).
- **`repository::crypto::verify`:** is a real Ed25519 verify over the given message using `try_into`-checked 32/64-byte keys/signatures (crypto.rs:35-48); the crypto primitive itself is sound. The problem is that the *compiler* never calls it on the import path (PKG-01), not that it is weak.
- **`validate_mfp_signature_header` (reader.rs:230 / repository package.rs:147):** correctly rejects mismatched type/length shapes `(0,!=0)`, `(1,!=64)`, and unknown types. Shape-only, which is fine as far as it goes — the missing piece is the actual signature check (PKG-01).
- **`decode_export_kind` / `decode_callable_export_kind` / import `pin` / type-field visibility:** enumerated tags are validated and unknown values rejected (reader.rs:1101, 498-503, 862-866), so reserved/unknown enum values do not silently mis-decode.
