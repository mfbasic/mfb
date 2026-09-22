# plan-144 findings: which record-field and `RES … STATE` self-updates the compiler performs in place

Measured against: **a `MUT` updating itself mutates in place, with no copy, for
every kind of value.** For a record, "itself" is the field:
`r = WITH r { f := op(r.f, …) }` should mutate `r.f` inside `r`'s existing block.

- **HEAD read:** `efdb54bb7` (worktree `worktree-P-144`, forked from `main`).
- **Compiler probed:** `target/release/mfb` built in the worktree from that HEAD
  (`cargo build --release`, finished 2026-09-21 14:13; newest `src/` commit
  `6ed5cc234`, 14:03).
- **Probes:** `/tmp/plan-144-probes/` (generators and scripts in Appendix C).

Files are abbreviated `bc` = `src/codegen/engine/control/builder_control.rs`,
`bia` = `src/codegen/collection/assign/builder_inplace_assign.rs`,
`ipd` = `src/codegen/collection/assign/inplace_dest.rs`,
`bvs` = `src/codegen/memory/value/builder_value_semantics.rs`,
`su` = `src/codegen/collection/assign/self_update.rs`,
`bcl` = `src/codegen/collection/layout/builder_collection_layout.rs`.

## Site legend

Record sites (§1; plan-144-A §5):

| Site | Meaning | Statement shape |
|---|---|---|
| S3 local rec, not-last | field `a` of a local `MUT` record `Rec { a AS T, b AS T }` | `r = WITH r { a := op(r.a, …) }` |
| S4 local rec, last | field `b` of the same record: the last inlined field, and the last field | `r = WITH r { b := op(r.b, …) }` |
| S5 global rec | field `b` of a module-level `MUT gR AS Rec` | `gR = WITH gR { b := op(gR.b, …) }` |
| S6 nested | field `b` of `inner AS Rec` inside `Out { n AS Integer, inner AS Rec }` | `o = WITH o { inner := WITH o.inner { b := op(o.inner.b, …) } }` |
| S7 loop-live | S4 inside `FOR EACH v IN r.b` | `n/a` for a type `FOR EACH` rejects (diagnostic in the evidence) |
| S9 captured | S4 inside a `collections::forEach([k], LAMBDA(v AS Integer) -> …)` capturing `r` | the update lowers in the lifted lambda |
| S10 two-field | S4 plus a scalar update in the same `WITH`, on `RecN { a, b AS T, n AS Integer }` | `r = WITH r { b := op(r.b, …), n := k }` |

`STATE` sites (§2): filled by plan-144-B.

**What "last" means.** The record arms admit a field only when
`record_collection_last_inlined` (`bc:295`) says it is a `List`/`Map`/`Set` that is
itself inlined and **no later field is inlined** (§1 Appendix B.4). In `Rec { a, b }`
with both fields of type `T`, `a` is never last when `T` is inlined; `b` is always
last (it is the last field). For a non-inlined `T` position cannot matter, because no
arm handles a non-collection field (G17), and the probes confirm that S3 and S4 agree
for every non-collection row.

**Cell values.** `y` = in place. `n (Gxx)` = the named gate is the first in code
order to decline, and the statement takes the copying path. `n (no arm)` = no
record-field arm exists for this function on this field type, so the statement
always takes the copying path, whatever the gate order (the code's first decline is
G17 for a non-collection field or a not-last collection field, else `G2`/`G3` in
`inplace_call_args`, `ipd:435`; plan-141's convention, kept so the two audits
compare). `n (StoreGlobal)` = the statement lowers through `NirOp::StoreGlobal`,
which does not reach any record arm (path P2). `n/a` = the form cannot be written
(reason given).

**Paths** (cited in each row's evidence):

- **A** — the record-field arm fires. `NirOp::Assign` (`bc:1192`) first calls the
  plan-142 seam `try_inplace_self_update` (`bc:1240`), whose 26 arms all decline at
  `G2` (the value is a `WithUpdate`, not a `Call`: `resolve_self_update`, `ipd:237`;
  concat needs a `&` chain). The record arm chain (`bc:1241–1283`) then runs, and the
  matching arm grows or rewrites the field's sub-block inside the record's own block
  (`InPlaceDest::Inlined`, no `with_target` slot).
- **P1** — the copying path on a local. Every arm declines, so the fallback
  `lower_value_owned(value)` (`bc:1294`) lowers the `WithUpdate` through
  `lower_with_update` (`bvs:703`): each updated field's new value is computed, every
  other field is read from the old block (`bvs:761–783`), and a **new record block**
  is built (`emit_build_inlined_record`, `bvs:784`). The old block is then freed
  (`bc:1369`), except under a live `FOR EACH` over one of its fields (`bc:1380`: the
  free is skipped, so the block leaks) and for a by-ref capture, whose old block is
  freed through the reference (`reassign_ref_old`, `bc:1342`, plan-142-G).
- **P2** — the copying path on a global. `NirOp::StoreGlobal` (`bc:1076`) reaches the
  seam only for `g = f(g, …)` or a `&` chain rooted at `g`
  (`is_global_self_update_call`, `su:262`, plan-142-H). A `WithUpdate` is neither,
  so no arm runs (`SUG=-` in all 339 S5 probes). `lower_value_owned` →
  `lower_with_update` rebuilds the record, and the old block is freed
  (`store_global_old`/`store_global_new`, `bc:1143`).

## 0. Row census (shared by §1 and §2)

Measured by `census.py` (Appendix A) against the probed compiler:
`packages 42 overloads 828`, 323 literal self-update overloads (first parameter type
= return type), plus 18 overloads whose return type names a type variable, of which 4
type-check as a self-update (Appendix C.1).

| Family | Rows | What | Source |
|---|---|---|---|
| F1 collection overloads | 63 | 59 literal (`collections` 23, `math` 27, `compress` 6, `crypto` 3) + 4 generics that type-check (`transform`, `mapValues`, `reduce`, `reduceRight`) | census, C.1 |
| F2 `String`/`AttributedString` overloads | 44 | `strings` 20, `encoding` 8, `fs` 6, `astrings` 4, `os` 3, `io` 1, `net` 1, `regex` 1 | census |
| F3 operators | 6 | `s & t`, `s & t & u`, `x + k` / `x - k` on `Integer` and on `Float` | plan-141 §1b |
| F4 other value types | 206 | 220 overloads of 20 types. 5 types keep one row each (`color::Color` 9, `datetime::DateTime` 4, `datetime::Duration` 3, `json::Json` 2, `http::Response` 1). The other 15 are split into 201 per-overload rows by the F4 guard (Appendix B.3, Correction A1) | census |
| F5 non-self-update forms | 7 | a replacement not derived from the field, for a scalar, `String`, `List`, `Map`, `Set` and record field, and `r.prop = value` | plan-141 R2, R6 |
| **total** | **326** | | |

The `form` column (plan-143 §4) is filled for F2 rows only. plan-143's findings do not
exist yet, so each form was classified here from the function's lowering and is marked
`(plan-144)`; the citations are in Appendix A.2.

## 1. Record sites

| row | form | S3 | S4 | S5 | S6 | S7 | S9 | S10 | evidence |
|---|---|---|---|---|---|---|---|---|---|
| F1 `collections::add(value AS Set OF T, item AS T) AS Set OF T` | — | n (G17) | y | n (StoreGlobal) | n (G17) | n (G15) | n (G1) | n (G14) | Path A at S4; P1 elsewhere; P2 at S5. Field `Set OF Integer`. Probe `r000o0_S*`: S3 ARM=- WITH=y, S4 ARM=record_field_set_add WITH=-, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda0 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::append(value AS List OF T, item AS T) AS List OF T` | — | n (G17) | y | n (StoreGlobal) | n (G17) | n (G15) | n (G1) | n (G14) | Path A at S4; P1 elsewhere; P2 at S5. Field `List OF Integer`. Probe `r001o0_S*`: S3 ARM=- WITH=y, S4 ARM=record_field_append WITH=-, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda1 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::append(value AS List OF T, item AS List OF T) AS List OF T` | — | n (G17) | y | n (StoreGlobal) | n (G17) | n (G15) | n (G1) | n (G14) | Path A at S4; P1 elsewhere; P2 at S5. Field `List OF Integer`. Probe `r002o0_S*`: S3 ARM=- WITH=y, S4 ARM=record_field_append WITH=-, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda2 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::difference(a AS Set OF T, b AS Set OF T) AS Set OF T` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Set OF Integer`. Probe `r003o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda3 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::distinct(value AS List OF T) AS List OF T` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r004o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda4 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::drop(value AS List OF T, count AS Integer) AS List OF T` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r005o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda5 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::filter(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS List OF T` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r006o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda6 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::insert(value AS List OF T, index AS Integer, item AS T) AS List OF T` | — | n (G17) | y | n (StoreGlobal) | n (G17) | n (G15) | n (G1) | n (G14) | Path A at S4; P1 elsewhere; P2 at S5. Field `List OF Integer`. Probe `r007o0_S*`: S3 ARM=- WITH=y, S4 ARM=record_field_splice(insert/prepend) WITH=-, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda7 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::intersection(a AS Set OF T, b AS Set OF T) AS Set OF T` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Set OF Integer`. Probe `r008o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda8 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::merge(a AS Map OF K TO V, b AS Map OF K TO V, preferB AS Boolean) AS Map OF K TO V` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Map OF Integer TO Integer`. Probe `r009o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda9 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::mid(value AS List OF T, start AS Integer, count AS Integer) AS List OF T` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r010o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda10 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::prepend(value AS List OF T, item AS T) AS List OF T` | — | n (G17) | y | n (StoreGlobal) | n (G17) | n (G15) | n (G1) | n (G14) | Path A at S4; P1 elsewhere; P2 at S5. Field `List OF Integer`. Probe `r011o0_S*`: S3 ARM=- WITH=y, S4 ARM=record_field_splice(insert/prepend) WITH=-, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda11 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::remove(value AS Set OF T, item AS T) AS Set OF T` | — | n (G17) | y | n (StoreGlobal) | n (G17) | n (G15) | n (G1) | n (G14) | Path A at S4; P1 elsewhere; P2 at S5. Field `Set OF Integer`. Probe `r012o0_S*`: S3 ARM=- WITH=y, S4 ARM=record_field_set_remove WITH=-, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda12 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::removeAt(value AS List OF T, index AS Integer) AS List OF T` | — | n (G17) | y | n (StoreGlobal) | n (G17) | n (G15) | n (G1) | n (G14) | Path A at S4; P1 elsewhere; P2 at S5. Field `List OF Integer`. Probe `r013o0_S*`: S3 ARM=- WITH=y, S4 ARM=record_field_remove_at WITH=-, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda13 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::removeKey(value AS Map OF K TO V, key AS K) AS Map OF K TO V` | — | n (G17) | y | n (StoreGlobal) | n (G17) | n (G15) | n (G1) | n (G14) | Path A at S4; P1 elsewhere; P2 at S5. Field `Map OF Integer TO Integer`. Probe `r014o0_S*`: S3 ARM=- WITH=y, S4 ARM=record_field_remove_key WITH=-, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda14 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::replace(value AS List OF T, old AS T, new AS T) AS List OF T` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r015o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda15 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::set(value AS List OF T, index AS Integer, item AS T) AS List OF T` | — | n (G17) | y | n (StoreGlobal) | n (G17) | n (G15) | n (G1) | n (G14) | Path A at S4; P1 elsewhere; P2 at S5. Field `List OF Integer`. Probe `r016o0_S*`: S3 ARM=- WITH=y, S4 ARM=record_field_set(List) WITH=-, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda16 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::set(value AS Map OF K TO V, index AS K, item AS V) AS Map OF K TO V` | — | n (G17) | y | n (StoreGlobal) | n (G17) | n (G15) | n (G1) | n (G14) | Path A at S4; P1 elsewhere; P2 at S5. Field `Map OF Integer TO Integer`. Probe `r017o0_S*`: S3 ARM=- WITH=y, S4 ARM=record_field_set(Map) WITH=-, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda17 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::sort(value AS List OF T) AS List OF T` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r018o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda18 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::sortBy(value AS List OF T, keyFn AS FUNC(T) AS U) AS List OF T` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r019o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda19 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::symmetricDifference(a AS Set OF T, b AS Set OF T) AS Set OF T` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Set OF Integer`. Probe `r020o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda20 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::take(value AS List OF T, count AS Integer) AS List OF T` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r021o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda21 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::union(a AS Set OF T, b AS Set OF T) AS Set OF T` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Set OF Integer`. Probe `r022o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda22 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `compress::deflate(data AS List OF Byte, [level AS Integer]) AS List OF Byte` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Byte`. Probe `r023o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda23 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `compress::gzipDecode(data AS List OF Byte, [maxBytes AS Integer], [ignoreChecksum AS Boolean]) AS List OF Byte` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Byte`. Probe `r024o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda24 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `compress::gzipEncode(data AS List OF Byte, [level AS Integer]) AS List OF Byte` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Byte`. Probe `r025o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda25 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `compress::inflate(data AS List OF Byte, [maxBytes AS Integer]) AS List OF Byte` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Byte`. Probe `r026o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda26 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `compress::zlibDecode(data AS List OF Byte, [maxBytes AS Integer], [ignoreChecksum AS Boolean]) AS List OF Byte` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Byte`. Probe `r027o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda27 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `compress::zlibEncode(data AS List OF Byte, [level AS Integer]) AS List OF Byte` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Byte`. Probe `r028o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda28 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `crypto::argon2id(password AS List OF Byte, salt AS List OF Byte, memoryKiB AS Integer, iterations AS Integer, parallelism AS Integer, length AS Integer) AS List OF Byte` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Byte`. Probe `r029o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda29 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `crypto::argon2id(password AS List OF Byte, salt AS List OF Byte, profile AS crypto::Argon2Profile, length AS Integer) AS List OF Byte` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Byte`. Probe `r030o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda30 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `crypto::shake256(data AS List OF Byte, length AS Integer) AS List OF Byte` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Byte`. Probe `r031o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda31 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::abs(value AS List OF Integer) AS List OF Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r032o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda32 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::abs(value AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r033o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda33 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::abs(value AS List OF Fixed) AS List OF Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Fixed`. Probe `r034o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda34 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::acos(value AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r035o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda35 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::asin(value AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r036o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda36 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::atan(value AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r037o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda37 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::atan2(y AS List OF Float, x AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r038o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda38 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::clamp(value AS List OF Integer, low AS Integer, high AS Integer) AS List OF Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r039o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda39 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::clamp(value AS List OF Float, low AS Float, high AS Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r040o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda40 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::clamp(value AS List OF Fixed, low AS Fixed, high AS Fixed) AS List OF Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Fixed`. Probe `r041o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda41 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::cos(value AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r042o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda42 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::exp(value AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r043o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda43 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::log(value AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r044o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda44 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::log(value AS List OF Fixed) AS List OF Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Fixed`. Probe `r045o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda45 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::log10(value AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r046o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda46 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::log10(value AS List OF Fixed) AS List OF Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Fixed`. Probe `r047o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda47 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::max(a AS List OF Integer, b AS List OF Integer) AS List OF Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r048o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda48 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::max(a AS List OF Float, b AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r049o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda49 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::max(a AS List OF Fixed, b AS List OF Fixed) AS List OF Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Fixed`. Probe `r050o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda50 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::min(a AS List OF Integer, b AS List OF Integer) AS List OF Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r051o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda51 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::min(a AS List OF Float, b AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r052o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda52 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::min(a AS List OF Fixed, b AS List OF Fixed) AS List OF Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Fixed`. Probe `r053o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda53 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::pow(base AS List OF Float, exponent AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r054o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda54 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::sin(value AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r055o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda55 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::sqrt(value AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r056o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda56 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::sqrt(value AS List OF Fixed) AS List OF Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Fixed`. Probe `r057o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda57 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `math::tan(value AS List OF Float) AS List OF Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Float`. Probe `r058o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda58 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::transform(value AS List OF T, f AS FUNC(T) AS U) AS List OF U` (generic, U = T) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r059o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda59 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::mapValues(value AS Map OF K TO V, f AS FUNC(V) AS U) AS Map OF K TO U` (generic, U = T) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Map OF Integer TO Integer`. Probe `r060o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda60 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::reduce(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS U` (generic, U = T) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r061o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda61 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F1 `collections::reduceRight(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS U` (generic, U = T) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r062o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda62 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F2 `astrings::addAttribute(value AS AttributedString, start AS Integer, endIndex AS Integer, attr AS astrings::Attribute) AS AttributedString` | same-len (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `AttributedString`. Probe `r063o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda63 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `astrings::clearAttributes(value AS AttributedString) AS AttributedString` | same-len (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `AttributedString`. Probe `r064o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda64 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `astrings::clearAttributes(value AS AttributedString, start AS Integer, endIndex AS Integer) AS AttributedString` | same-len (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `AttributedString`. Probe `r065o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda65 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `astrings::removeAttribute(value AS AttributedString, start AS Integer, endIndex AS Integer, attr AS astrings::Attribute) AS AttributedString` | same-len (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `AttributedString`. Probe `r066o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda66 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `encoding::formUrlDecode(value AS String) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r067o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda67 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `encoding::formUrlEncode(value AS String) AS String` | grow (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r068o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda68 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `encoding::htmlEscape(value AS String) AS String` | grow (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r069o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda69 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `encoding::htmlUnescape(value AS String) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r070o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda70 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `encoding::percentDecode(value AS String) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r071o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda71 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `encoding::percentEncode(value AS String) AS String` | grow (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r072o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda72 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `encoding::punycodeDecode(asciiDomain AS String) AS String` | rewrite (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r073o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda73 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `encoding::punycodeEncode(domain AS String) AS String` | rewrite (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r074o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda74 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `fs::canonicalPath(path AS String) AS String` | rewrite (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r075o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda75 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `fs::pathBaseName(path AS String) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r076o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda76 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `fs::pathDirName(path AS String) AS String` | rewrite (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r077o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda77 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `fs::pathExtension(path AS String) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r078o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda78 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `fs::pathNormalize(path AS String) AS String` | rewrite (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r079o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda79 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `fs::readText(path AS String) AS String` | not-derived (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r080o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda80 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `io::input([prompt AS String]) AS String` | not-derived (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r081o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda81 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `net::percentDecode(s AS String) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r082o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda82 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `os::getEnv(name AS String) AS String` | not-derived (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r083o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda83 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `os::getEnvOr(name AS String, fallback AS String) AS String` | not-derived (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r084o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda84 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `os::resourcePath(relative AS String) AS String` | grow (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r085o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda85 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `regex::replace(value AS String, pattern AS String, replacement AS String) AS String` | rewrite (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r086o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda86 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::caseFold(value AS String) AS String` | rewrite (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r087o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda87 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::graphemeAt(value AS String, index AS Integer) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r088o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda88 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::left(value AS String, count AS Integer) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r089o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda89 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::lower(value AS String) AS String` | rewrite (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r090o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda90 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::mid(value AS String, start AS Integer, count AS Integer) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r091o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda91 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::normalizeNfc(value AS String) AS String` | rewrite (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r092o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda92 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::padLeft(value AS String, width AS Integer, [padChar AS String]) AS String` | grow (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r093o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda93 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::padLeftToWidth(value AS String, columns AS Integer, [padChar AS String]) AS String` | grow (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r094o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda94 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::padRight(value AS String, width AS Integer, [padChar AS String]) AS String` | grow (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r095o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda95 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::padRightToWidth(value AS String, columns AS Integer, [padChar AS String]) AS String` | grow (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r096o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda96 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::repeat(value AS String, times AS Integer) AS String` | rewrite (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r097o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda97 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::replace(value AS String, old AS String, new AS String) AS String` | rewrite (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r098o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda98 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::right(value AS String, count AS Integer) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r099o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda99 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::stripPrefix(value AS String, prefix AS String) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r100o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda100 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::stripSuffix(value AS String, suffix AS String) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r101o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda101 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::trim(value AS String) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r102o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda102 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::trimChars(value AS String, chars AS String) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r103o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda103 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::trimEnd(value AS String) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r104o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda104 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::trimStart(value AS String) AS String` | shrink (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r105o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda105 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F2 `strings::upper(value AS String) AS String` | rewrite (plan-144) | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r106o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda106 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F3 `s & t` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r107o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda107 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F3 `s & t & u` (chain) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r108o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda108 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F3 `x + k` (Integer) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r109o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda109 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F3 `x - k` (Integer) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r110o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda110 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F3 `x + k` (Float) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r111o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda111 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F3 `x - k` (Float) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r112o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda112 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `big::abs(a AS big::Int) AS big::Int` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `big::Int`. Probe `r113o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda113 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `big::add(a AS big::Int, b AS big::Int) AS big::Int` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `big::Int`. Probe `r114o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda114 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `big::divide(a AS big::Int, b AS big::Int) AS big::Int` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `big::Int`. Probe `r115o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda115 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `big::gcd(a AS big::Int, b AS big::Int) AS big::Int` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `big::Int`. Probe `r116o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda116 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `big::modPow(base AS big::Int, exponent AS big::Int, modulus AS big::Int) AS big::Int` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `big::Int`. Probe `r117o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda117 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `big::multiply(a AS big::Int, b AS big::Int) AS big::Int` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `big::Int`. Probe `r118o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda118 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `big::negate(a AS big::Int) AS big::Int` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `big::Int`. Probe `r119o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda119 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `big::pow(base AS big::Int, exponent AS Integer) AS big::Int` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `big::Int`. Probe `r120o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda120 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `big::remainder(a AS big::Int, b AS big::Int) AS big::Int` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `big::Int`. Probe `r121o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda121 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `big::shiftLeft(a AS big::Int, count AS Integer) AS big::Int` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `big::Int`. Probe `r122o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda122 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `big::shiftRight(a AS big::Int, count AS Integer) AS big::Int` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `big::Int`. Probe `r123o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda123 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `big::subtract(a AS big::Int, b AS big::Int) AS big::Int` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `big::Int`. Probe `r124o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda124 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `crypto::randomInt(min AS big::Int, max AS big::Int) AS big::Int` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `big::Int`. Probe `r125o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda125 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::band(a AS Integer, b AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r126o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda126 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::bnot(a AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r127o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda127 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::bor(a AS Integer, b AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r128o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda128 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::bswap16(value AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r129o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda129 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::bswap32(value AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r130o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda130 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::bswap64(value AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r131o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda131 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::bxor(a AS Integer, b AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r132o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda132 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::clz(value AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r133o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda133 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::ctz(value AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r134o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda134 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::popCount(value AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r135o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda135 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::rl32(value AS Integer, count AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r136o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda136 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::rl64(value AS Integer, count AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r137o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda137 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::rr32(value AS Integer, count AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r138o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda138 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::rr64(value AS Integer, count AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r139o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda139 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::sl(value AS Integer, count AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r140o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda140 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::sr(value AS Integer, count AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r141o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda141 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `bits::sra(value AS Integer, count AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r142o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda142 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `crypto::randomInt(min AS Integer, max AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r143o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda143 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `datetime::daysInMonth(year AS Integer, month AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r144o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda144 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `datetime::localOffset(epochSeconds AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r145o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda145 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::abs(value AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r146o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda146 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::clamp(value AS Integer, low AS Integer, high AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r147o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda147 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::max(a AS Integer, b AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r148o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda148 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::min(a AS Integer, b AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r149o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda149 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::rand(min AS Integer, max AS Integer) AS Integer` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r150o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda150 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `color::Color` (all 9 overloads: color::brighten, color::darken, color::desaturate, color::grayscale, color::invert, color::mix, color::rotateHue, color::saturate, color::withAlpha) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `color::Color`. Probe `r151o0…o8_S*` (9 overloads, identical markers): S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda151 ARM=- WITH=y; $lambda152 ARM=- WITH=y; $lambda153 ARM=- WITH=y; $lambda154 ARM=- WITH=y; $lambda155 ARM=- WITH=y; $lambda156 ARM=- WITH=y; $lambda157 ARM=- WITH=y; $lambda158 ARM=- WITH=y; $lambda159 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `datetime::add(at AS datetime::Instant, by AS datetime::Duration) AS datetime::Instant` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `datetime::Instant`. Probe `r152o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda160 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `datetime::subtract(at AS datetime::Instant, by AS datetime::Duration) AS datetime::Instant` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `datetime::Instant`. Probe `r153o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda161 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `datetime::DateTime` (all 4 overloads: datetime::addDays, datetime::addMonths, datetime::startOfDay, datetime::withZone) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `datetime::DateTime`. Probe `r154o0…o3_S*` (4 overloads, identical markers): S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda162 ARM=- WITH=y; $lambda163 ARM=- WITH=y; $lambda164 ARM=- WITH=y; $lambda165 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `datetime::Duration` (all 3 overloads: datetime::minus, datetime::negate, datetime::plus) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `datetime::Duration`. Probe `r155o0…o2_S*` (3 overloads, identical markers): S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda166 ARM=- WITH=y; $lambda167 ARM=- WITH=y; $lambda168 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `http::Response` (all 1 overloads: http::withHeader) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `http::Response`. Probe `r156o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda169 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `json::Json` (all 2 overloads: json::get, json::getOr) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `json::Json`. Probe `r157o0…o1_S*` (2 overloads, identical markers): S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda170 ARM=- WITH=y; $lambda171 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::abs(value AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r158o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda172 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::acos(value AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r159o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda173 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::asin(value AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r160o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda174 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::atan(value AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r161o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda175 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::atan2(y AS Float, x AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r162o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda176 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::clamp(value AS Float, low AS Float, high AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r163o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda177 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::cos(value AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r164o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda178 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::exp(value AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r165o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda179 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::log(value AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r166o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda180 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::log10(value AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r167o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda181 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::max(a AS Float, b AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r168o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda182 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::min(a AS Float, b AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r169o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda183 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::pow(base AS Float, exponent AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r170o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda184 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::sin(value AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r171o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda185 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::sqrt(value AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r172o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda186 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::tan(value AS Float) AS Float` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Float`. Probe `r173o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda187 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::abs(value AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r174o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda188 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::acos(value AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r175o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda189 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::asin(value AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r176o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda190 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::atan(value AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r177o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda191 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::atan2(y AS Fixed, x AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r178o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda192 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::clamp(value AS Fixed, low AS Fixed, high AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r179o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda193 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::cos(value AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r180o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda194 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::exp(value AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r181o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda195 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::log(value AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r182o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda196 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::log10(value AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r183o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda197 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::max(a AS Fixed, b AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r184o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda198 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::min(a AS Fixed, b AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r185o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda199 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::pow(base AS Fixed, exponent AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r186o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda200 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::sin(value AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r187o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda201 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::sqrt(value AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r188o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda202 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::tan(value AS Fixed) AS Fixed` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Fixed`. Probe `r189o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda203 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::abs(value AS Money) AS Money` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Money`. Probe `r190o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda204 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::clamp(value AS Money, low AS Money, high AS Money) AS Money` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Money`. Probe `r191o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda205 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::max(a AS Money, b AS Money) AS Money` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Money`. Probe `r192o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda206 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::min(a AS Money, b AS Money) AS Money` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Money`. Probe `r193o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda207 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `math::rand(min AS Money, max AS Money) AS Money` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Money`. Probe `r194o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda208 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `money::round(value AS Money, decimals AS Integer) AS Money` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Money`. Probe `r195o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda209 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::abs(v AS vector::Float2) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r196o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda210 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::clamp_length(v AS vector::Float2, max AS Float) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r197o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda211 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::cross(a AS vector::Float2) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r198o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda212 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp(a AS vector::Float2, b AS vector::Float2, t AS Float) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r199o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda213 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp_unclamped(a AS vector::Float2, b AS vector::Float2, t AS Float) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r200o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda214 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::max(a AS vector::Float2, b AS vector::Float2) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r201o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda215 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::min(a AS vector::Float2, b AS vector::Float2) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r202o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda216 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::normalize(v AS vector::Float2) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r203o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda217 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::perpendicular(v AS vector::Float2) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r204o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda218 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::project(a AS vector::Float2, b AS vector::Float2) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r205o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda219 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reflect(a AS vector::Float2, b AS vector::Float2) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r206o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda220 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reject(a AS vector::Float2, b AS vector::Float2) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r207o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda221 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::rotate_2d(v AS vector::Float2, angle AS Float) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r208o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda222 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::scale(a AS vector::Float2, b AS vector::Float2) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r209o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda223 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::slerp(a AS vector::Float2, b AS vector::Float2, t AS Float) AS vector::Float2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float2`. Probe `r210o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda224 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::abs(v AS vector::Float3) AS vector::Float3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float3`. Probe `r211o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda225 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::clamp_length(v AS vector::Float3, max AS Float) AS vector::Float3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float3`. Probe `r212o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda226 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::cross(a AS vector::Float3, b AS vector::Float3) AS vector::Float3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float3`. Probe `r213o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda227 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp(a AS vector::Float3, b AS vector::Float3, t AS Float) AS vector::Float3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float3`. Probe `r214o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda228 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp_unclamped(a AS vector::Float3, b AS vector::Float3, t AS Float) AS vector::Float3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float3`. Probe `r215o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda229 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::max(a AS vector::Float3, b AS vector::Float3) AS vector::Float3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float3`. Probe `r216o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda230 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::min(a AS vector::Float3, b AS vector::Float3) AS vector::Float3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float3`. Probe `r217o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda231 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::normalize(v AS vector::Float3) AS vector::Float3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float3`. Probe `r218o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda232 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::project(a AS vector::Float3, b AS vector::Float3) AS vector::Float3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float3`. Probe `r219o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda233 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reflect(a AS vector::Float3, b AS vector::Float3) AS vector::Float3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float3`. Probe `r220o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda234 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reject(a AS vector::Float3, b AS vector::Float3) AS vector::Float3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float3`. Probe `r221o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda235 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::scale(a AS vector::Float3, b AS vector::Float3) AS vector::Float3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float3`. Probe `r222o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda236 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::slerp(a AS vector::Float3, b AS vector::Float3, t AS Float) AS vector::Float3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float3`. Probe `r223o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda237 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::abs(v AS vector::Float4) AS vector::Float4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float4`. Probe `r224o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda238 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::clamp_length(v AS vector::Float4, max AS Float) AS vector::Float4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float4`. Probe `r225o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda239 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::cross(a AS vector::Float4, b AS vector::Float4, c AS vector::Float4) AS vector::Float4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float4`. Probe `r226o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda240 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp(a AS vector::Float4, b AS vector::Float4, t AS Float) AS vector::Float4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float4`. Probe `r227o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda241 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp_unclamped(a AS vector::Float4, b AS vector::Float4, t AS Float) AS vector::Float4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float4`. Probe `r228o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda242 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::max(a AS vector::Float4, b AS vector::Float4) AS vector::Float4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float4`. Probe `r229o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda243 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::min(a AS vector::Float4, b AS vector::Float4) AS vector::Float4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float4`. Probe `r230o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda244 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::normalize(v AS vector::Float4) AS vector::Float4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float4`. Probe `r231o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda245 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::project(a AS vector::Float4, b AS vector::Float4) AS vector::Float4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float4`. Probe `r232o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda246 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reflect(a AS vector::Float4, b AS vector::Float4) AS vector::Float4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float4`. Probe `r233o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda247 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reject(a AS vector::Float4, b AS vector::Float4) AS vector::Float4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float4`. Probe `r234o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda248 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::scale(a AS vector::Float4, b AS vector::Float4) AS vector::Float4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float4`. Probe `r235o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda249 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::slerp(a AS vector::Float4, b AS vector::Float4, t AS Float) AS vector::Float4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Float4`. Probe `r236o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda250 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::abs(v AS vector::Fixed2) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r237o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda251 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::clamp_length(v AS vector::Fixed2, max AS Fixed) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r238o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda252 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::cross(a AS vector::Fixed2) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r239o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda253 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp(a AS vector::Fixed2, b AS vector::Fixed2, t AS Float) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r240o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda254 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp_unclamped(a AS vector::Fixed2, b AS vector::Fixed2, t AS Float) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r241o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda255 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::max(a AS vector::Fixed2, b AS vector::Fixed2) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r242o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda256 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::min(a AS vector::Fixed2, b AS vector::Fixed2) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r243o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda257 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::normalize(v AS vector::Fixed2) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r244o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda258 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::perpendicular(v AS vector::Fixed2) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r245o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda259 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::project(a AS vector::Fixed2, b AS vector::Fixed2) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r246o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda260 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reflect(a AS vector::Fixed2, b AS vector::Fixed2) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r247o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda261 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reject(a AS vector::Fixed2, b AS vector::Fixed2) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r248o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda262 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::rotate_2d(v AS vector::Fixed2, angle AS Float) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r249o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda263 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::scale(a AS vector::Fixed2, b AS vector::Fixed2) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r250o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda264 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::slerp(a AS vector::Fixed2, b AS vector::Fixed2, t AS Float) AS vector::Fixed2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed2`. Probe `r251o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda265 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::abs(v AS vector::Fixed3) AS vector::Fixed3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed3`. Probe `r252o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda266 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::clamp_length(v AS vector::Fixed3, max AS Fixed) AS vector::Fixed3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed3`. Probe `r253o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda267 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::cross(a AS vector::Fixed3, b AS vector::Fixed3) AS vector::Fixed3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed3`. Probe `r254o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda268 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp(a AS vector::Fixed3, b AS vector::Fixed3, t AS Float) AS vector::Fixed3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed3`. Probe `r255o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda269 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp_unclamped(a AS vector::Fixed3, b AS vector::Fixed3, t AS Float) AS vector::Fixed3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed3`. Probe `r256o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda270 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::max(a AS vector::Fixed3, b AS vector::Fixed3) AS vector::Fixed3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed3`. Probe `r257o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda271 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::min(a AS vector::Fixed3, b AS vector::Fixed3) AS vector::Fixed3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed3`. Probe `r258o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda272 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::normalize(v AS vector::Fixed3) AS vector::Fixed3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed3`. Probe `r259o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda273 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::project(a AS vector::Fixed3, b AS vector::Fixed3) AS vector::Fixed3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed3`. Probe `r260o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda274 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reflect(a AS vector::Fixed3, b AS vector::Fixed3) AS vector::Fixed3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed3`. Probe `r261o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda275 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reject(a AS vector::Fixed3, b AS vector::Fixed3) AS vector::Fixed3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed3`. Probe `r262o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda276 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::scale(a AS vector::Fixed3, b AS vector::Fixed3) AS vector::Fixed3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed3`. Probe `r263o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda277 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::slerp(a AS vector::Fixed3, b AS vector::Fixed3, t AS Float) AS vector::Fixed3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed3`. Probe `r264o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda278 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::abs(v AS vector::Fixed4) AS vector::Fixed4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed4`. Probe `r265o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda279 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::clamp_length(v AS vector::Fixed4, max AS Fixed) AS vector::Fixed4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed4`. Probe `r266o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda280 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::cross(a AS vector::Fixed4, b AS vector::Fixed4, c AS vector::Fixed4) AS vector::Fixed4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed4`. Probe `r267o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda281 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp(a AS vector::Fixed4, b AS vector::Fixed4, t AS Float) AS vector::Fixed4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed4`. Probe `r268o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda282 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp_unclamped(a AS vector::Fixed4, b AS vector::Fixed4, t AS Float) AS vector::Fixed4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed4`. Probe `r269o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda283 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::max(a AS vector::Fixed4, b AS vector::Fixed4) AS vector::Fixed4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed4`. Probe `r270o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda284 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::min(a AS vector::Fixed4, b AS vector::Fixed4) AS vector::Fixed4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed4`. Probe `r271o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda285 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::normalize(v AS vector::Fixed4) AS vector::Fixed4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed4`. Probe `r272o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda286 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::project(a AS vector::Fixed4, b AS vector::Fixed4) AS vector::Fixed4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed4`. Probe `r273o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda287 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reflect(a AS vector::Fixed4, b AS vector::Fixed4) AS vector::Fixed4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed4`. Probe `r274o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda288 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reject(a AS vector::Fixed4, b AS vector::Fixed4) AS vector::Fixed4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed4`. Probe `r275o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda289 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::scale(a AS vector::Fixed4, b AS vector::Fixed4) AS vector::Fixed4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed4`. Probe `r276o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda290 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::slerp(a AS vector::Fixed4, b AS vector::Fixed4, t AS Float) AS vector::Fixed4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Fixed4`. Probe `r277o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda291 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::abs(v AS vector::Integer2) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r278o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda292 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::clamp_length(v AS vector::Integer2, max AS Integer) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r279o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda293 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::cross(a AS vector::Integer2) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r280o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda294 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp(a AS vector::Integer2, b AS vector::Integer2, t AS Float) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r281o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda295 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp_unclamped(a AS vector::Integer2, b AS vector::Integer2, t AS Float) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r282o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda296 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::max(a AS vector::Integer2, b AS vector::Integer2) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r283o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda297 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::min(a AS vector::Integer2, b AS vector::Integer2) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r284o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda298 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::normalize(v AS vector::Integer2) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r285o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda299 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::perpendicular(v AS vector::Integer2) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r286o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda300 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::project(a AS vector::Integer2, b AS vector::Integer2) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r287o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda301 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reflect(a AS vector::Integer2, b AS vector::Integer2) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r288o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda302 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reject(a AS vector::Integer2, b AS vector::Integer2) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r289o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda303 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::rotate_2d(v AS vector::Integer2, angle AS Float) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r290o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda304 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::scale(a AS vector::Integer2, b AS vector::Integer2) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r291o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda305 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::slerp(a AS vector::Integer2, b AS vector::Integer2, t AS Float) AS vector::Integer2` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer2`. Probe `r292o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda306 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::abs(v AS vector::Integer3) AS vector::Integer3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer3`. Probe `r293o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda307 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::clamp_length(v AS vector::Integer3, max AS Integer) AS vector::Integer3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer3`. Probe `r294o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda308 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::cross(a AS vector::Integer3, b AS vector::Integer3) AS vector::Integer3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer3`. Probe `r295o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda309 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp(a AS vector::Integer3, b AS vector::Integer3, t AS Float) AS vector::Integer3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer3`. Probe `r296o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda310 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp_unclamped(a AS vector::Integer3, b AS vector::Integer3, t AS Float) AS vector::Integer3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer3`. Probe `r297o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda311 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::max(a AS vector::Integer3, b AS vector::Integer3) AS vector::Integer3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer3`. Probe `r298o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda312 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::min(a AS vector::Integer3, b AS vector::Integer3) AS vector::Integer3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer3`. Probe `r299o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda313 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::normalize(v AS vector::Integer3) AS vector::Integer3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer3`. Probe `r300o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda314 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::project(a AS vector::Integer3, b AS vector::Integer3) AS vector::Integer3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer3`. Probe `r301o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda315 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reflect(a AS vector::Integer3, b AS vector::Integer3) AS vector::Integer3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer3`. Probe `r302o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda316 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reject(a AS vector::Integer3, b AS vector::Integer3) AS vector::Integer3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer3`. Probe `r303o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda317 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::scale(a AS vector::Integer3, b AS vector::Integer3) AS vector::Integer3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer3`. Probe `r304o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda318 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::slerp(a AS vector::Integer3, b AS vector::Integer3, t AS Float) AS vector::Integer3` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer3`. Probe `r305o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda319 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::abs(v AS vector::Integer4) AS vector::Integer4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer4`. Probe `r306o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda320 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::clamp_length(v AS vector::Integer4, max AS Integer) AS vector::Integer4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer4`. Probe `r307o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda321 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::cross(a AS vector::Integer4, b AS vector::Integer4, c AS vector::Integer4) AS vector::Integer4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer4`. Probe `r308o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda322 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp(a AS vector::Integer4, b AS vector::Integer4, t AS Float) AS vector::Integer4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer4`. Probe `r309o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda323 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::lerp_unclamped(a AS vector::Integer4, b AS vector::Integer4, t AS Float) AS vector::Integer4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer4`. Probe `r310o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda324 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::max(a AS vector::Integer4, b AS vector::Integer4) AS vector::Integer4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer4`. Probe `r311o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda325 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::min(a AS vector::Integer4, b AS vector::Integer4) AS vector::Integer4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer4`. Probe `r312o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda326 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::normalize(v AS vector::Integer4) AS vector::Integer4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer4`. Probe `r313o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda327 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::project(a AS vector::Integer4, b AS vector::Integer4) AS vector::Integer4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer4`. Probe `r314o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda328 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reflect(a AS vector::Integer4, b AS vector::Integer4) AS vector::Integer4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer4`. Probe `r315o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda329 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::reject(a AS vector::Integer4, b AS vector::Integer4) AS vector::Integer4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer4`. Probe `r316o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda330 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::scale(a AS vector::Integer4, b AS vector::Integer4) AS vector::Integer4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer4`. Probe `r317o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda331 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F4 `vector::slerp(a AS vector::Integer4, b AS vector::Integer4, t AS Float) AS vector::Integer4` | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `vector::Integer4`. Probe `r318o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda332 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F5 replacement, scalar field (`f := k`) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Integer`. Probe `r319o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda333 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F5 replacement, `String` field (`f := toString(k)`) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `String`. Probe `r320o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda334 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F5 replacement, `List` field (`f := [k]`) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `List OF Integer`. Probe `r321o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda335 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F5 replacement, `Map` field (`f := mkMap()`) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Map OF Integer TO Integer`. Probe `r322o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda336 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F5 replacement, `Set` field (`f := toSet([k])`) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n (no arm) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Set OF Integer`. Probe `r323o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S7 ARM=- WITH=y, S9 $lambda337 ARM=- WITH=y, S10 ARM=- WITH=y. |
| F5 replacement, record field (`f := Inner[x := k]`) | — | n (no arm) | n (no arm) | n (StoreGlobal) | n (no arm) | n/a (not a `FOR EACH` iterable) | n (no arm) | n (no arm) | Path P1; P2 at S5. Field `Inner`. Probe `r324o0_S*`: S3 ARM=- WITH=y, S4 ARM=- WITH=y, S5 ARM=- GLOBAL=y SUG=- WITH=y, S6 ARM=- WITH=y, S9 $lambda338 ARM=- WITH=y, S10 ARM=- WITH=y. S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`). |
| F5 `r.prop = value` (field assignment) | — | n/a (not expressible) | n/a (not expressible) | n/a (not expressible) | n/a (not expressible) | n/a (not expressible) | n/a (not expressible) | n/a (not expressible) | Parse error `1-102-0013 MFB_PARSE_RECORD_FIELD_ASSIGNMENT` (probe `r6`, Appendix C.4); rule at `src/rules/table.rs:186`. `WITH` is the only record update form. |

### 1b. Re-verification of plan-141 §2 (measured at `b6a10efbc`)

plan-141 §2's 22 record rows (R1–R6, 88 cells) were re-probed by rebuilding plan-141
Appendix C.4's source verbatim with this compiler (`p141c4`, Appendix C.6) and
diffing the markers against the ones plan-141 recorded. **Every record-form marker is
identical** (`r1_*`…`r5_*`, `x_concat_S3`, `x_concat_S4`). No record verdict changed
between `b6a10efbc` and `efdb54bb7`. The one line that differs is not a record cell:

| probe | plan-141 | now | changed by |
|---|---|---|---|
| `x_concat_S2` (global `String` `gStr = gStr & …`) | `ARM=- GLOBAL=y` | `ARM=concat GLOBAL=-` | `02692cd64` plan-142-H (`git log -S is_global_self_update_call`) |

This agrees with the code. `fndiff.py` (Appendix C.7) compares every record-field arm,
`resolve_inplace_record_field`, `inplace_call_args`, `InPlaceGate::admits`,
`record_collection_last_inlined`, `record_field_is_inlined` and `lower_with_update`
between `b6a10efbc` and HEAD, normalizing whitespace, and all of them are `same`. The
record path's only change is the dispatch. The seam call now precedes the record arms
(`bc:1240`), and it declines every `WithUpdate` at `G2`.

The §1 table's R-row equivalents: R1 (scalar/`String` replacement) = the F5 scalar and
`String` rows; R2 = the F5 `List`/`Map`/`Set` rows; R3 = the F1 arm rows (and the F1
no-arm rows); R4 = column S10 (G14); R5 = column S6 (G17); R6 = the F5 `r.prop = value`
row.

## 2. `STATE` sites

Filled by plan-144-B.

## 3. Summary

Written by plan-144-B.

## Appendix A — census

### A.1 `census.py`

Run on 2026-09-21 against the worktree's `target/release/mfb`:
`python3 census.py target/release/mfb hits` → `packages 42 overloads 828` and 323
lines; `… generic` → 18 lines. The family split and per-type counts
(`rows.py`, Appendix C.2) are: coll 59, str 44, other 220 over 20 types (`Integer` 25,
`Float` 16, `Fixed` 16, `vector::Float2`/`Fixed2`/`Integer2` 15 each, `big::Int` 13,
the six other `vector::*` types 13 each, `color::Color` 9, `Money` 6,
`datetime::DateTime` 4, `datetime::Duration` 3, `datetime::Instant` 2, `json::Json` 2,
`http::Response` 1).

```python
"""plan-144 row census: every builtin overload across every `mfb man` package.

  python3 census.py <mfb> hits      -> `packages N overloads M`, then every literal
                                       self-update overload (first param type == return)
  python3 census.py <mfb> generic   -> every overload whose return type differs from
                                       its first parameter's but names a type variable
  python3 census.py <mfb> all       -> every overload, one per line

plan-142-A's Appendix script, with the type test replaced by `ft and ft == ret`.
"""
import re
import subprocess
import sys

M, mode = sys.argv[1], sys.argv[2]
top = subprocess.run([M, "man"], capture_output=True, text=True).stdout
pkgs = re.findall(r"^│ ([a-zA-Z]+) +│", top[top.index("Builtin packages"):], re.M)
pkgs = [p for p in pkgs if p != "Package"]
rows, total = [], 0
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
            rows.append((pkg, f, s, ft, ret))
if mode == "hits":
    print("packages", len(pkgs), "overloads", total)
    for pkg, f, s, ft, ret in rows:
        if ft and ft == ret:
            print(s)
elif mode == "generic":
    for pkg, f, s, ft, ret in rows:
        if ft and ft != ret and re.search(r"\b[A-Z]\b", ret):
            print(s)
else:
    for r in rows:
        print(r[2])
```

### A.2 The `form` column for F2 rows, with citations

Classified from each function's lowering, not its man page (paths under
`src/codegen/`). `shrink`/`grow` on an encoder or decoder means the length only moves
one way. The bytes are transformed, not sliced, so none of them is a substring or
superstring of the argument.

| function | form | lowering | reason |
|---|---|---|---|
| `astrings::addAttribute` | same-len | `builtins/astrings/func_add_attribute.rs:39` | text deep-copied unchanged (`writeSpans`, `gen_astrings.rs:175`); the run list gains one span |
| `astrings::clearAttributes` (1-arg) | same-len | `builtins/astrings/func_clear_attributes.rs:44` | text copied unchanged; the run list becomes empty |
| `astrings::clearAttributes` (3-arg) | same-len | `builtins/astrings/func_clear_attributes.rs:51` | text copied unchanged; runs are split around the range, so the count can go up or down |
| `astrings::removeAttribute` | same-len | `builtins/astrings/func_remove_attribute.rs:42` | text copied unchanged; matching runs are split or dropped |
| `encoding::formUrlDecode` | shrink | `builtins/encoding/helper_percent_decode_bytes.rs:13` | `%XX` → 1 byte, `+` → space, the rest copied: never longer |
| `encoding::formUrlEncode` | grow | `builtins/encoding/func_form_url_encode.rs:39` | alphanumerics kept, space → `+`, others → `%XX`: never shorter |
| `encoding::htmlEscape` | grow | `builtins/encoding/func_html_escape.rs:41` | five `strings::replace` calls, each one character → a longer entity |
| `encoding::htmlUnescape` | shrink | `builtins/encoding/func_html_unescape.rs:52` | each accepted reference (4+ bytes) decodes to fewer UTF-8 bytes |
| `encoding::percentDecode` | shrink | `builtins/encoding/helper_percent_decode_bytes.rs:13` | `%XX` → 1 byte, the rest copied |
| `encoding::percentEncode` | grow | `builtins/encoding/func_percent_encode.rs:36` | unreserved bytes kept, others → `%XX` |
| `encoding::punycodeDecode` | rewrite | `builtins/encoding/func_punycode_decode.rs:49` | each `xn--` label decoded to UTF-8, longer or shorter |
| `encoding::punycodeEncode` | rewrite | `builtins/encoding/func_punycode_encode.rs:40` | a non-ASCII label → `xn--` + encoding, longer or shorter |
| `fs::canonicalPath` | rewrite | `builtins/fs/gen_canonical.rs:112` | the OS `realpath` (touches the filesystem): resolves the cwd, symlinks and `..` |
| `fs::pathBaseName` | shrink | `builtins/fs/gen_path_builder.rs:108` | the substring after the last `/` |
| `fs::pathDirName` | rewrite | `builtins/fs/gen_path_builder.rs:173` | a prefix, but no `/` returns the constant `"."` (`:223`), so `""` → `"."` is longer |
| `fs::pathExtension` | shrink | `builtins/fs/gen_path_builder.rs:257` | the suffix from the last `.` |
| `fs::pathNormalize` | rewrite | `builtins/fs/gen_path_builder.rs:321` | syntactic; never longer except `""` → `"."` (`:371`) |
| `fs::readText` | not-derived | `builtins/fs/gen_atomic_write.rs:802` | the argument is a path; the result is the file's contents |
| `io::input` | not-derived | `builtins/io/func_input.rs:16` | the argument is a prompt; the result is a stdin line |
| `net::percentDecode` | shrink | `builtins/net/helper_percent_decode_impl.rs:18` | `%XX` → 1 byte, the rest copied |
| `os::getEnv` | not-derived | `builtins/os/func_get_env.rs:14` | the argument is a name; the result is the variable's value |
| `os::getEnvOr` | not-derived | `builtins/os/func_get_env_or.rs:13` | the value, or the fallback argument |
| `os::resourcePath` | grow | `builtins/os/func_resource_path.rs:30` | the executable's directory + `/` + the argument verbatim |
| `regex::replace` | rewrite | `builtins/regex/func_replace.rs:124` | splices expanded replacements in |
| `strings::caseFold` | rewrite | `builtins/strings/gen_case_map.rs:191` | ASCII byte-for-byte, but non-ASCII takes the Unicode table, which can change the width (count pass `:204`) |
| `strings::graphemeAt` | shrink | `builtins/strings/func_grapheme_at.rs:172` | one grapheme's byte range |
| `strings::left` | shrink | `builtins/strings/gen_left_right.rs:80` | a prefix of `count` scalars |
| `strings::lower` | rewrite | `builtins/strings/gen_case_map.rs:191` | as `caseFold` |
| `strings::mid` | shrink | `collection/search/builder_search.rs:663` | `Intrinsic`; a byte range from the start offset |
| `strings::normalizeNfc` | rewrite | `builtins/strings/func_normalize_nfc.rs:72` | decompose/reorder/compose: composing shrinks, exclusions expand |
| `strings::padLeft` | grow | `builtins/strings/gen_pad.rs:148` | max(0, width − scalars) pad characters in front |
| `strings::padLeftToWidth` | grow | `builtins/strings/helper_pad_to_width.rs:47` | `repeat(padChar, copies) & value` |
| `strings::padRight` | grow | `builtins/strings/gen_pad.rs:148` | as `padLeft`, after the argument |
| `strings::padRightToWidth` | grow | `builtins/strings/helper_pad_to_width.rs:55` | `value & repeat(padChar, copies)` |
| `strings::repeat` | rewrite | `builtins/strings/func_repeat.rs:120` | len × times; `times = 0` → `""`, so it can shrink: not `grow` |
| `strings::replace` | rewrite | `string/repr/builder_strings.rs:15` | `Intrinsic`; `old`/`new` can differ in length |
| `strings::right` | shrink | `builtins/strings/gen_left_right.rs:113` | a suffix of `count` scalars |
| `strings::stripPrefix` | shrink | `builtins/strings/gen_strip.rs:58` | start moved past a matched prefix, or unchanged |
| `strings::stripSuffix` | shrink | `builtins/strings/gen_strip.rs:58` | length cut by a matched suffix, or unchanged |
| `strings::trim` | shrink | `builtins/strings/gen_trim.rs:104` | the byte range left after trimming whitespace |
| `strings::trimChars` | shrink | `builtins/strings/func_trim_chars.rs:205` | the byte range left after trimming the set |
| `strings::trimEnd` | shrink | `builtins/strings/gen_trim.rs:104` | as `trim`, end side only |
| `strings::trimStart` | shrink | `builtins/strings/gen_trim.rs:104` | as `trim`, start side only |
| `strings::upper` | rewrite | `builtins/strings/gen_case_map.rs:191` | as `caseFold` |

## Appendix B — record paths (plan-144-A Phase 2)

### B.1 The record-field arms

`grep -rhoE 'fn try_inplace_[a-z_]*' src --include='*.rs' | wc -l` → 48; 9 of them are
`try_inplace_record_field_*`. All 9 go through **RF** =
`resolve_inplace_record_field` (`ipd:310`): `G2` `WithUpdate` → `G13` the target is
this local → `G14` exactly one update → `G17` `record_collection_last_inlined`
(`bc:295`: a `List`/`Map`/`Set`, inlined, with no inlined field after it) →
`InPlaceGate` (`ipd:342`) {`G1` `by_ref`, `G15` no live `FOR EACH` over this field,
`G10` layout} → `inplace_call_args` (`ipd:435`) {`G2` `Call`, `G3`
`native_builtin_target` = the arm's builtin, `G4` arity}. Dispatch order: after the
seam (`bc:1240`), `bc:1241` → `:1283` in the order listed.

| recogniser | location | dispatch | recognises (builtin, arity) | ordered gates after RF | vs plan-141 B.3 |
|---|---|---|---|---|---|
| try_inplace_record_field_append | `bia:90` | `bc:1241` | `append`, 2 (element or list) | G9 List → G18 `args[0]` is `r.f` → G11 element or list → G12 `args[1]` is not `r.f` | body identical (`fndiff.py`); was `bia:92`, `bc:1216` |
| try_inplace_record_field_remove_key_assign | `bia:307` | `bc:1247` | `removeKey`, 2 | G9 Map → G18 → G11 key | identical; was `bia:348` |
| try_inplace_record_field_remove_at_assign | `bia:372` | `bc:1253` | `removeAt`, 2 | G9 List → G18 → E1 index | identical; was `bia:413` |
| try_inplace_record_field_set_remove_assign | `bia:431` | `bc:1259` | `remove`, 2 | G9 Set → G18 → G11 element | identical; was `bia:472` |
| try_inplace_record_field_set_add_assign | `bia:497` | `bc:1265` | `add`, 2 | G9 Set → G18 → G11 element → G12 | identical; was `bia:538` |
| try_inplace_record_field_set_assign | `bia:588` | `bc:1271` | `set`, 3 (List or Map) | G18 → List: G26 fixed-width element (`bia:617`), E1, E2 \| Map: E2 key, E2 value | identical; was `bia:629` |
| try_inplace_record_field_insert_assign | `bia:1337` | `bc:1277` | `insert`, 3 | → `…_splice_assign` | identical; was `bia:1378` |
| try_inplace_record_field_prepend_assign | `bia:1348` | `bc:1283` | `prepend`, 2 | → `…_splice_assign` | identical; was `bia:1389` |
| try_inplace_record_field_splice_assign | `bia:1250` | `bia:1344`, `bia:1355` | shared body of `insert`/`prepend` | G9 List → G18 → G12 → G11 element | identical; was `bia:1291` |

**Differences from plan-141 Appendix B.3:** line numbers only, and one dispatch change.
Every arm body and both container functions are unchanged (`fndiff.py b6a10efbc HEAD`
→ `same` for all 9 arms, RF, `inplace_call_args`, `admits`,
`record_collection_last_inlined`). The dispatch change is that `NirOp::Assign` now
calls the plan-142 seam first (`bc:1240`). It declines every `WithUpdate` (below).

### B.2 Seam check: does a record statement reach `try_inplace_self_update`?

**Yes, it reaches it, and it never fires there.** `try_inplace_self_update` (`su:234`)
has exactly two callers (`grep -rn 'try_inplace_self_update(' src | grep -v 'fn '`):
`bc:1102` (inside `NirOp::StoreGlobal`, only when `is_global_self_update_call` or a
global `&` chain matches, `bc:1087–1091`) and `bc:1240` (inside `NirOp::Assign`,
**unconditionally**, before the record arms). `SelfUpdateSite` is built at exactly
those two places (`bc:1092`, `bc:1234`). A local record's `r = WITH r {…}` therefore
enters the seam. Every one of its 26 arms (`SELF_UPDATE_ARMS`, `su:151`) needs a `Call`
whose `args[0]` is the binding (`resolve_self_update`, `ipd:237–250`: `G2`, `G5`/`G6`),
or a `&` chain rooted at it (concat). A `WithUpdate` is neither, so every arm declines
without emitting. A global record's `WITH` does not enter the seam at all, because
`is_global_self_update_call` (`su:262`) is false for it. Dump: no seam-arm marker
appears in any of the 2,100 record probe functions or their 339 lambdas, and `SUG=-`
in all 339 S5 probes (Appendix C.5). This is new since plan-141 (the seam did not
exist at `b6a10efbc`), but it does not change any verdict.

### B.3 F4 guard

The claim a per-type F4 row makes is that no arm names a builtin of that type, so the
verdict cannot depend on which function is called. The guard compares every builtin
name an in-place arm matches with the 220 F4 overloads:

- the arm names: the 26 seam arms (`su:151`; names from the `resolve_self_update` /
  `resolve_shrink` / `resolve_set_op` / `try_inplace_filter_set` string literals, and
  `MATH_SELF_UPDATE`, `builder_inplace_rewrite.rs:21`), the 9 record arms and 8
  `STATE` Layer-2 arms (the `resolve_inplace_record_field`/`…_state_field` literals):
  `append add set removeKey prepend removeAt insert remove filter take drop mid distinct
  replace transform sort sortBy union intersection difference symmetricDifference merge
  mapValues` plus the 16 `math` names `abs acos asin atan atan2 clamp cos exp log log10
  max min pow sin sqrt tan`. The concat arm matches `&`, not a name. `SELF_UPDATE_TABLE`
  (`su:796`) names no builtin outside these.
- command: `grep -cE '::(<the names above joined by |>)\(' f4.txt` over the 220 F4
  overloads → **71, not 0.** The hits are `math::abs/acos/asin/atan/atan2/clamp/cos/exp/
  log/log10/max/min/pow/sin/sqrt/tan` on `Integer`/`Float`/`Fixed`/`Money` (40),
  `vector::abs/max/min` on all 9 vector types (27), `big::abs/add/pow` (3) and
  `datetime::add` on `Instant` (1).

A name hit is not an arm firing. Every hit's arm also gates the binding's type
(`G9`/`G10`: a `List`/`Set`/`Map` layout), and at a record or `STATE` site no seam arm
runs at all (B.2). But the guard as the plan wrote it is a name test, and it failed,
so the plan's rule applies: **the 15 types with a hit (`Integer`, `Float`, `Fixed`,
`Money`, `big::Int`, `datetime::Instant`, and the 9 `vector::*` types) are split into
per-overload rows** (201 rows). The 5 types without a hit keep one row each. Every
overload of those 5 was still probed (the row's evidence lists them), and the markers
are identical across each type's overloads. See Correction A1.

### B.4 Inlined classification of every row field type

`record_field_is_inlined` (`bcl:3152`): `String` → inlined; otherwise inlined iff the
type is a composite (a record in the type model, a union, a collection, or a `Result`)
**and** `type_is_memcpy_copyable` (`bcl:3160`). Measured by probe `inl`
(Appendix C.8): `TYPE L_T { xs AS List OF Integer, f AS T }` with
`r = WITH r { xs := append(r.xs, k) }` fires `record_field_append` iff `T` is **not**
inlined (G17 counts inlined fields after `xs`).

| field type | inlined | branch | probe `inl_<T>` |
|---|---|---|---|
| `String` | yes | `== String` | `WITH=y` |
| `AttributedString` | yes | composite, copyable | `WITH=y` |
| `List OF …`, `Map OF …`, `Set OF …` (F1 rows) | yes | collection, copyable payload | plan-141 `r3_list_then_*`; §1 S3 = `n (G17)` |
| nested record `Inner { x AS Integer }` (F5) | yes | record, copyable | `WITH=y` |
| `big::Int` | yes | record, copyable | `WITH=y` |
| `color::Color` | yes | record, copyable | `WITH=y` |
| `datetime::DateTime`, `datetime::Duration`, `datetime::Instant` | yes | record, copyable | `WITH=y` (each) |
| `http::Response` | yes | record, copyable | `WITH=y` |
| `vector::Float2/3/4`, `vector::Fixed2/3/4`, `vector::Integer2/3/4` | yes | record, copyable | `WITH=y` (each) |
| `Integer`, `Float`, `Fixed`, `Money` | no | scalar: not composite | `ARM=record_field_append` (each) |
| `json::Json` | no | a union (`man json types`: Unions) that is recursive (`JsonArr`/`JsonObj` hold `Json`), so not memcpy-copyable; it is a pointer field (`record_field_is_pointer`, `bcl:2815` → `named_field_is_pointer`: `union_names`) | `ARM=record_field_append` |

Which F4 types are records comes from `mfb man <pkg> types` (the "Records" section):
all nine vectors, `color::Color`, `big::Int`, the `datetime` types and
`http::Response` are records, and `json::Json` is the only union.

### B.5 The copy paths, and the `String` capacity shadow

- **`lower_with_update`** (`bvs:703`): allocates `with_target`, lowers each updated
  field's value (`bvs:730–755`), reads every other field from the old block
  (`bvs:761–783`; an inlined field's pointer is `base + offset`, a kept recursive field
  is deep-copied, plan-134-G), and builds a fresh inlined record
  (`emit_build_inlined_record`, `bvs:784`). Nothing in it can reuse the old block.
  The caller frees the old block (P1, P2 above).
- **`StoreGlobal`** (`bc:1076`): after the seam gate, `lower_value_owned` (which is
  `lower_with_update` for a `WITH`), then the old-block free through
  `store_global_old`/`store_global_new` (`bc:1143`), then the store.
- **The capacity shadow cannot exist for a field.** `string_capacity_slot_for`
  (`bc:2351`) answers from `string_capacity_slots`, which is keyed on a **local name**
  and filled only by `prescan_string_self_appends` (`bc:2374`) for
  `NirOp::Assign { name, value }` whose value is `name & …`
  (`string_self_append_operands(value, name)`). `r = WITH r { s := r.s & t }` is a
  `WithUpdate`, not a `&` chain rooted at a local, so no slot is created. No record arm
  handles `&` either. F3's `&` rows are therefore `n` at every record site, and
  `y` would need both a per-field shadow and a record `&` arm.

## Appendix C — probes

All probes were built with the worktree's `target/release/mfb` using
`mfb build --ncode <project>` at the default optimization level, in
`/tmp/plan-144-probes/`. Each project uses the same `project.json` (`kind: executable`,
`entry: main`, `targets: [native]`). A function's `stackSlots` in the `.ncode` JSON
lists its slots by type name. Every marker slot name occurs exactly once in `src/`
(`grep -rho '"<name>"' src --include='*.rs' | wc -l` → 1 for `with_target`,
`store_global_new`, `su_global_block`, `state_field_inplace`, `state_assign_value`,
`state_assign_replaced`, `inline_state_rhs`, `inplace_recfield_rhs`), so its presence
proves which path ran.

### C.1 Generic self-updates

`gen/src/main.mfb`: one SUB per generic candidate (the 18 `census.py … generic` lines):
`xs = collections::chunks(xs, 2)`, `xss = collections::flatten(xss)`,
`xs = collections::get(xs, 0)`, `m = collections::get(m, 1)`,
`xs = collections::getOr(xs, 0, xs)`, `m = collections::getOr(m, 1, m)`,
`xs = collections::groupBy(xs, ident, ident)`, `m = collections::keys(m)`,
`m = collections::mapValues(m, ident)`, `xs = collections::partition(xs, pos)`,
`xs = collections::reduce(xs, emptyList(), keep)`, `… reduceRight …`,
`s = collections::toList(s)`, `xs = collections::toSet(xs)`,
`xs = collections::transform(xs, ident)`, `m = collections::values(m)`,
`xs = collections::window(xs, 2)`, `xs = collections::zip(xs, xs)`.
`mfb build gen` → exit 1 with 14 errors, one per non-type-checking candidate:

```
gen/src/main.mfb:48 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: function call argument type does not match parameter type
gen/src/main.mfb:53 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: function call argument type does not match parameter type
gen/src/main.mfb:28 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
gen/src/main.mfb:33 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
gen/src/main.mfb:38 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
gen/src/main.mfb:43 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
gen/src/main.mfb:58 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
gen/src/main.mfb:63 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
gen/src/main.mfb:74 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
gen/src/main.mfb:91 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
gen/src/main.mfb:96 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
gen/src/main.mfb:107 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
gen/src/main.mfb:112 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
gen/src/main.mfb:117 error[2-203-0008 TYPE_ASSIGNMENT_MISMATCH]: assignment value type does not match binding type
```

Lines 48/53 are `getOr` (an argument of type `T` can never be the `List OF T` or
`Map OF K TO V` itself), and the other 12 are assignment mismatches (`chunks`,
`flatten`, `get` ×2, `groupBy`, `keys`, `partition`, `toList`, `toSet`, `values`,
`window`, `zip`). `transform`, `mapValues`, `reduce` and `reduceRight` build with no
error: **4 generic rows**, the same 4 as plan-141 C.3.

### C.2 `rows.py` — the shared row census

```python
"""plan-144 shared row census -> rows.json (used by both the record and STATE probes).

  python3 rows.py            -> writes /tmp/plan-144-probes/rows.json, prints family counts

Families (plan-144-A §3):
  F1 collection overloads: the 59 literal ones + the 4 generics that type-check
  F2 String/AttributedString overloads: 44
  F3 operators: 6
  F4 other value types: one row per type, EXCEPT a type one of whose overloads shares
     a builtin name with an in-place arm (the Phase 2 F4 guard); such a type is split
     into one row per overload
  F5 non-self-update forms: 7
Every row carries the field type it updates and an op template, `{X}` = the field.
"""
import json
import re

P = "/tmp/plan-144-probes/"
hits = [l.strip() for l in open(P + "hits.txt").read().splitlines()[1:]]

# The Phase 2 F4 guard: every builtin name an in-place arm matches.
ARM_NAMES = set("""append add set removeKey prepend removeAt insert remove filter take drop
mid distinct replace transform sort sortBy union intersection difference symmetricDifference
merge mapValues abs acos asin atan atan2 clamp cos exp log log10 max min pow sin sqrt tan""".split())

ARG = {  # expression for a non-first argument of this type
    "Integer": "k", "Float": "1.5", "Fixed": "1.5F", "Money": "1.50m", "Boolean": "TRUE",
    "String": '"a"', "big::Int": "big::fromInteger(k)", "color::Color": "color::rgb(1, 2, 3)",
    "datetime::Duration": "datetime::duration(k)", "datetime::Instant": "datetime::fromMillis(k)",
    "datetime::DateTime": "datetime::toUtc(datetime::fromMillis(k))",
    "datetime::Zone": "datetime::utc()", "json::Json": 'json::parse("1")',
    "http::Response": 'http::ok("a")', "List OF String": '["a"]',
    "List OF Byte": "saltBytes()", "List OF Integer": "[k]",
    "List OF Float": "[1.5]", "List OF Fixed": "[1.5F]",
    "crypto::Argon2Profile": "crypto::Argon2Profile.Recommended",
    "astrings::Attribute": "astrings::bold()", "AttributedString": 'astrings::fromString("a")',
}
for d in "234":
    for e in ("Float", "Fixed", "Integer"):
        ARG[f"vector::{e}{d}"] = f"vector::one{e}{d}"

INIT = dict(ARG)  # an initial field value: no `k` in scope at module level
INIT.update({"Integer": "1", "big::Int": "big::fromInteger(1)",
             "datetime::Duration": "datetime::duration(1)",
             "datetime::Instant": "datetime::fromMillis(1)",
             "datetime::DateTime": "datetime::toUtc(datetime::fromMillis(1))",
             "List OF Integer": "[1, 2, 3]", "List OF Float": "[1.5, 2.5]",
             "List OF Fixed": "[1.5F, 2.5F]", "List OF Byte": "abcBytes()",
             "Map OF Integer TO Integer": "mkMap()",
             "Set OF Integer": "collections::toSet([1, 2, 3])", "Inner": "Inner[x := 1]"})


def split_params(params):
    out, depth, cur = [], 0, ""
    for ch in params:
        if ch in "([":
            depth += 1
        elif ch in ")]":
            depth -= 1
        if ch == "," and depth == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(cur.strip())
    return out


def auto_template(sig):
    m = re.match(r"(\w+)::(\w+)\((.*)\) AS (.*)$", sig)
    pkg, fn, params = m.group(1), m.group(2), m.group(3)
    args = []
    for i, p in enumerate(split_params(params)):
        if p.startswith("["):
            continue  # optional parameter: omitted
        t = p.split(" AS ", 1)[1]
        args.append("{X}" if i == 0 else ARG[t])
    return f"{pkg}::{fn}({', '.join(args)})"


COLL = {  # plan-141 C.2's templates, keyed on (function, first parameter type)
    ("add", "Set OF T"): "collections::add({X}, k)",
    ("append", "item AS T"): "collections::append({X}, k)",
    ("append", "item AS List OF T"): "collections::append({X}, [k, k])",
    ("difference", ""): "collections::difference({X}, collections::toSet([k]))",
    ("distinct", ""): "collections::distinct({X})",
    ("drop", ""): "collections::drop({X}, 1)",
    ("filter", ""): "collections::filter({X}, isPositive)",
    ("insert", ""): "collections::insert({X}, 0, k)",
    ("intersection", ""): "collections::intersection({X}, collections::toSet([k]))",
    ("merge", ""): "collections::merge({X}, mkMap(), TRUE)",
    ("mid", ""): "collections::mid({X}, 0, 2)",
    ("prepend", ""): "collections::prepend({X}, k)",
    ("remove", ""): "collections::remove({X}, k)",
    ("removeAt", ""): "collections::removeAt({X}, 0)",
    ("removeKey", ""): "collections::removeKey({X}, k)",
    ("replace", ""): "collections::replace({X}, 1, k)",
    ("set", "List"): "collections::set({X}, 0, k)",
    ("set", "Map"): "collections::set({X}, k, k)",
    ("sort", ""): "collections::sort({X})",
    ("sortBy", ""): "collections::sortBy({X}, negate)",
    ("symmetricDifference", ""): "collections::symmetricDifference({X}, collections::toSet([k]))",
    ("take", ""): "collections::take({X}, 2)",
    ("union", ""): "collections::union({X}, collections::toSet([k]))",
}
GENERIC = [
    ("collections::transform(value AS List OF T, f AS FUNC(T) AS U) AS List OF U",
     "List OF Integer", "collections::transform({X}, negate)"),
    ("collections::mapValues(value AS Map OF K TO V, f AS FUNC(V) AS U) AS Map OF K TO U",
     "Map OF Integer TO Integer", "collections::mapValues({X}, negate)"),
    ("collections::reduce(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS U",
     "List OF Integer", "collections::reduce({X}, emptyList(), keep)"),
    ("collections::reduceRight(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS U",
     "List OF Integer", "collections::reduceRight({X}, emptyList(), keep)"),
]


def coll_row(sig):
    fn = re.match(r"collections::(\w+)\(", sig).group(1)
    first = re.match(r"collections::\w+\(\w+ AS ([^,)]*)", sig).group(1)
    ft = {"List OF T": "List OF Integer", "Set OF T": "Set OF Integer",
          "Map OF K TO V": "Map OF Integer TO Integer"}[first]
    for (name, key), tpl in COLL.items():
        if name == fn and (key == "" or key in sig or first.startswith(key)):
            return ft, tpl
    raise KeyError(sig)


def ret_of(sig):
    return sig.rsplit(" AS ", 1)[1]


rows = []
coll = [s for s in hits if ret_of(s).split()[0] in ("List", "Set", "Map")]
strs = [s for s in hits if ret_of(s) in ("String", "AttributedString")]
other = [s for s in hits if s not in coll and s not in strs]
assert (len(coll), len(strs), len(other)) == (59, 44, 220), (len(coll), len(strs), len(other))

for s in coll:
    if s.startswith("collections::"):
        ft, tpl = coll_row(s)
    else:
        ft, tpl = ret_of(s), auto_template(s)
    rows.append({"fam": "F1", "label": f"`{s}`", "ft": ft, "ops": [tpl]})
for s, ft, tpl in GENERIC:
    rows.append({"fam": "F1", "label": f"`{s}` (generic, U = T)", "ft": ft, "ops": [tpl]})
# plan-143 §4's form column, classified here from each function's lowering because
# plan-143's findings do not exist yet (citations: findings Appendix A.2).
FORM = {
    "astrings::addAttribute": "same-len", "astrings::clearAttributes": "same-len",
    "astrings::removeAttribute": "same-len",
    "encoding::formUrlDecode": "shrink", "encoding::formUrlEncode": "grow",
    "encoding::htmlEscape": "grow", "encoding::htmlUnescape": "shrink",
    "encoding::percentDecode": "shrink", "encoding::percentEncode": "grow",
    "encoding::punycodeDecode": "rewrite", "encoding::punycodeEncode": "rewrite",
    "fs::canonicalPath": "rewrite", "fs::pathBaseName": "shrink", "fs::pathDirName": "rewrite",
    "fs::pathExtension": "shrink", "fs::pathNormalize": "rewrite", "fs::readText": "not-derived",
    "io::input": "not-derived", "net::percentDecode": "shrink", "os::getEnv": "not-derived",
    "os::getEnvOr": "not-derived", "os::resourcePath": "grow", "regex::replace": "rewrite",
    "strings::caseFold": "rewrite", "strings::graphemeAt": "shrink", "strings::left": "shrink",
    "strings::lower": "rewrite", "strings::mid": "shrink", "strings::normalizeNfc": "rewrite",
    "strings::padLeft": "grow", "strings::padLeftToWidth": "grow", "strings::padRight": "grow",
    "strings::padRightToWidth": "grow", "strings::repeat": "rewrite", "strings::replace": "rewrite",
    "strings::right": "shrink", "strings::stripPrefix": "shrink", "strings::stripSuffix": "shrink",
    "strings::trim": "shrink", "strings::trimChars": "shrink", "strings::trimEnd": "shrink",
    "strings::trimStart": "shrink", "strings::upper": "rewrite",
}
for s in strs:
    fn = re.match(r"(\w+::\w+)\(", s).group(1)
    rows.append({"fam": "F2", "label": f"`{s}`", "ft": ret_of(s), "ops": [auto_template(s)],
                 "form": FORM[fn] + " (plan-144)"})
F3 = [("`s & t`", "String", '{X} & toString(k)'),
      ("`s & t & u` (chain)", "String", '{X} & toString(k) & "u"'),
      ("`x + k` (Integer)", "Integer", "{X} + k"),
      ("`x - k` (Integer)", "Integer", "{X} - k"),
      ("`x + k` (Float)", "Float", "{X} + 1.5"),
      ("`x - k` (Float)", "Float", "{X} - 1.5")]
for label, ft, tpl in F3:
    rows.append({"fam": "F3", "label": label, "ft": ft, "ops": [tpl]})
by_type = {}
for s in other:
    by_type.setdefault(ret_of(s), []).append(s)
split = sorted(t for t, ss in by_type.items()
               if any(re.match(r"\w+::(\w+)\(", s).group(1) in ARM_NAMES for s in ss))
for t, ss in by_type.items():
    if t in split:
        for s in ss:
            rows.append({"fam": "F4", "label": f"`{s}`", "ft": t, "ops": [auto_template(s)]})
    else:
        rows.append({"fam": "F4", "label": f"`{t}` (all {len(ss)} overloads: "
                     + ", ".join(re.match(r"(\w+::\w+)\(", s).group(1) for s in ss) + ")",
                     "ft": t, "ops": [auto_template(s) for s in ss]})
F5 = [("replacement, scalar field (`f := k`)", "Integer", "k"),
      ("replacement, `String` field (`f := toString(k)`)", "String", "toString(k)"),
      ("replacement, `List` field (`f := [k]`)", "List OF Integer", "[k]"),
      ("replacement, `Map` field (`f := mkMap()`)", "Map OF Integer TO Integer", "mkMap()"),
      ("replacement, `Set` field (`f := toSet([k])`)", "Set OF Integer", "collections::toSet([k])"),
      ("replacement, record field (`f := Inner[x := k]`)", "Inner", "Inner[x := k]")]
for label, ft, tpl in F5:
    rows.append({"fam": "F5", "label": label, "ft": ft, "ops": [tpl]})
rows.append({"fam": "F5", "label": "`r.prop = value` (field assignment)", "ft": None, "ops": []})

for i, r in enumerate(rows):
    r["id"] = f"r{i:03d}"
json.dump({"rows": rows, "split": split}, open(P + "rows.json", "w"), indent=1)
from collections import Counter
print(Counter(r["fam"] for r in rows), "total", len(rows))
print("split F4 types:", len(split), split)
print("unsplit F4 types:", [t for t in by_type if t not in split])
```

### C.3 `gen_rec.py` — the record probe

2,100 functions (339 ops × 6 sites, plus S7 for the 66 iterable-field ops), 0 excluded,
built without diagnostics.

```python
"""Generate /tmp/plan-144-probes/rec/src/main.mfb: one FUNC per (row, op, record site).

FUNC names are `<row id>o<op index>_<site>`; markers.py then reports, per FUNC, which
arm fired and which copy path ran. A FUNC returns the record so the update is live;
S5 (the global) is a SUB. `rec/exclude.txt` lists FUNC names that do not compile,
with their diagnostic, and are left out (each becomes an `n/a` cell with that
diagnostic, after the diagnostic is checked to be the language's and not the probe's).

Record types, per field type T (plan-144-A §5):
  RecT   { a AS T, b AS T }            a = S3 (not last), b = S4 (last)
  RecNT  { a AS T, b AS T, n AS Integer }   S10: `b` and the scalar `n` in one WITH
  OutT   { n AS Integer, inner AS RecT }    S6
  MUT gRT AS RecT                           S5
"""
import json
import os
import re
import sys

sys.path.insert(0, "/tmp/plan-144-probes")
P = "/tmp/plan-144-probes/"
OUT = sys.argv[1] if len(sys.argv) > 1 else P + "rec"
data = json.load(open(P + "rows.json"))
rows = data["rows"]
ns = {}
exec(open(P + "rows.py").read().split("def split_params")[0], ns)  # the ARG/INIT tables
INIT = ns["INIT"]

exclude = {}
if os.path.exists(OUT + "/exclude.txt"):
    for line in open(OUT + "/exclude.txt"):
        if line.strip():
            name, _, why = line.strip().partition(" ")
            exclude[name] = why

ITERABLE = ("List", "Map", "Set")


def ident(t):
    return re.sub(r"[^A-Za-z0-9]", "_", t)


types = sorted({r["ft"] for r in rows if r["ft"]})
out = ["IMPORT collections", "IMPORT io", "IMPORT math", "IMPORT strings", "IMPORT astrings",
       "IMPORT encoding", "IMPORT fs", "IMPORT os", "IMPORT net", "IMPORT regex", "IMPORT big",
       "IMPORT bits", "IMPORT color", "IMPORT crypto", "IMPORT datetime", "IMPORT http",
       "IMPORT json", "IMPORT money", "IMPORT vector", "IMPORT compress", "",
       "TYPE Inner", "  x AS Integer", "END TYPE", ""]
for t in types:
    i = ident(t)
    out += [f"TYPE Rec_{i}", f"  a AS {t}", f"  b AS {t}", "END TYPE", ""]
    out += [f"TYPE RecN_{i}", f"  a AS {t}", f"  b AS {t}", "  n AS Integer", "END TYPE", ""]
    out += [f"TYPE Out_{i}", "  n AS Integer", f"  inner AS Rec_{i}", "END TYPE", ""]
out += [
    "FUNC mkMap() AS Map OF Integer TO Integer",
    "  MUT m AS Map OF Integer TO Integer",
    "  m = collections::set(m, 1, 1)",
    "  RETURN m",
    "END FUNC", "",
    "FUNC emptyList() AS List OF Integer", "  RETURN []", "END FUNC", "",
    "FUNC saltBytes() AS List OF Byte", '  LET b AS List OF Byte = encoding::utf8Encode("salt-salt")',
    "  RETURN b", "END FUNC", "",
    "FUNC abcBytes() AS List OF Byte", '  LET b AS List OF Byte = encoding::utf8Encode("abc")',
    "  RETURN b", "END FUNC", "",
    "FUNC negate(v AS Integer) AS Integer", "  RETURN 0 - v", "END FUNC", "",
    "FUNC isPositive(v AS Integer) AS Boolean", "  RETURN v > 0", "END FUNC", "",
    "FUNC keep(acc AS List OF Integer, v AS Integer) AS List OF Integer",
    "  RETURN collections::append(acc, v)", "END FUNC", "",
]
for t in types:
    i = ident(t)
    out.append(f"MUT gR_{i} AS Rec_{i} = Rec_{i}[a := {INIT[t]}, b := {INIT[t]}]")
out.append("")

funcs = []


def emit(name, ret, body):
    if name in exclude:
        return
    funcs.append((name, ret))
    head = f"FUNC {name}(k AS Integer) AS {ret}" if ret else f"SUB {name}(k AS Integer)"
    out.extend([head] + ["  " + l for l in body] + ["END FUNC" if ret else "END SUB", ""])


for r in rows:
    t = r["ft"]
    if t is None:
        continue
    i = ident(t)
    init = f"Rec_{i}[a := {INIT[t]}, b := {INIT[t]}]"
    for oi, tpl in enumerate(r["ops"]):
        base = f"{r['id']}o{oi}"
        op = lambda x: tpl.replace("{X}", x)  # noqa: E731
        emit(f"{base}_S3", f"Rec_{i}", [f"MUT r AS Rec_{i} = {init}",
                                        f"r = WITH r {{ a := {op('r.a')} }}", "RETURN r"])
        emit(f"{base}_S4", f"Rec_{i}", [f"MUT r AS Rec_{i} = {init}",
                                        f"r = WITH r {{ b := {op('r.b')} }}", "RETURN r"])
        emit(f"{base}_S5", None, [f"gR_{i} = WITH gR_{i} {{ b := {op(f'gR_{i}.b')} }}"])
        emit(f"{base}_S6", f"Out_{i}", [f"MUT o AS Out_{i} = Out_{i}[n := 1, inner := {init}]",
                                        f"o = WITH o {{ inner := WITH o.inner {{ b := {op('o.inner.b')} }} }}",
                                        "RETURN o"])
        if t.split()[0] in ITERABLE:
            emit(f"{base}_S7", f"Rec_{i}", [f"MUT r AS Rec_{i} = {init}", "FOR EACH v IN r.b",
                                            f"  r = WITH r {{ b := {op('r.b')} }}", "NEXT",
                                            "RETURN r"])
        emit(f"{base}_S9", f"Rec_{i}", [f"MUT r AS Rec_{i} = {init}",
                                        f"collections::forEach([k], LAMBDA(v AS Integer) -> r = WITH r {{ b := {op('r.b')} }})",
                                        "RETURN r"])
        emit(f"{base}_S10", f"RecN_{i}", [f"MUT r AS RecN_{i} = RecN_{i}[a := {INIT[t]}, b := {INIT[t]}, n := 1]",
                                          f"r = WITH r {{ b := {op('r.b')}, n := k }}", "RETURN r"])

out += ["SUB main()"] + [f"  {n}(7)" for n, _ in funcs] + ["END SUB", ""]
os.makedirs(OUT + "/src", exist_ok=True)
open(OUT + "/src/main.mfb", "w").write("\n".join(out))
os.system(f"cp {P}project.json {OUT}/project.json")
print(len(funcs), "functions,", len(exclude), "excluded")
```

### C.4 `markers.py` and `lambdas.py`

`markers.py` is plan-141 C.1's script extended with every plan-142 arm (the
`ArmId::markers()` table, `su:1251`), the record arms, the `STATE` arms, the seam-global
marker and the `STATE` replace markers. `lambdas.py` maps each lifted `$lambdaN` to the
function that references it (by the mangled symbol), so each S9 cell reads its own
lambda's markers. On the record probe it maps 342 lambdas: 339 to the 339 S9 functions,
one each, and 3 to library bodies (`#crypto_hkdf`, `#crypto_pbkdf2`,
`#crypto_hpkeLabeledExpand`).

```python
"""Per-function in-place arm markers from an `mfb build --ncode` dump (plan-144).

Usage: python3 markers.py <file.ncode> [function-name-prefix]

plan-141 Appendix C.1's `markers.py`, extended for plan-144: every arm added by
plan-142 (the `ArmId::markers()` table in `self_update.rs`), the record-field
arms, the STATE arms, and the two copy paths.

Each arm allocates a stack slot whose type name occurs exactly once in `src/`, so
the slot's presence in a function's `stackSlots` proves the arm fired there.

Prints per function:
  ARM     - the arm(s) whose marker slot is present, or `-`;
  GLOBAL  - `store_global_new` (the `NirOp::StoreGlobal` copy-and-free path ran);
  SUG     - `su_global_block` (StoreGlobal reached the seam, plan-142-H);
  WITH    - `with_target` (`lower_with_update`, the whole-record rebuild);
  L1      - `state_field_inplace` (STATE Layer 1, the in-place scalar store);
  REPL    - `state_assign_value` (the STATE whole-record replace);
  FREE    - `state_assign_replaced` (bug-644: the replace frees the old block).
"""
import json
import sys

ARMS = {
    # plan-142 seam arms (ArmId::markers)
    "inplace_append_item": "append",
    "inplace_bulk_append_rhs": "bulk_append",
    "inplace_set_add_item": "set_add",
    "inplace_set_index": "set(List)",
    "inplace_set_key": "set(Map)",
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
    # record-field arms
    "inplace_recfield_rhs": "record_field_append",
    "inplace_recfield_remove_key": "record_field_remove_key",
    "inplace_recfield_remove_at_index": "record_field_remove_at",
    "inplace_recfield_set_remove": "record_field_set_remove",
    "inplace_recfield_add_item": "record_field_set_add",
    "inplace_recfield_set_index": "record_field_set(List)",
    "inplace_recfield_set_key": "record_field_set(Map)",
    "inplace_recfield_splice_item": "record_field_splice(insert/prepend)",
    # STATE Layer 2 arms
    "inline_state_rhs": "state_append",
    "inplace_state_remove_key": "state_remove_key",
    "inplace_state_remove_at_index": "state_remove_at",
    "inplace_state_set_remove": "state_set_remove",
    "inplace_state_add_item": "state_set_add",
    "inplace_state_set_index": "state_set(List)",
    "inplace_state_set_key": "state_set(Map)",
    "inplace_state_splice_item": "state_splice(insert/prepend)",
}

path = sys.argv[1]
prefix = sys.argv[2] if len(sys.argv) > 2 else ""
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


def flag(types, slot):
    return "y" if slot in types else "-"


functions = []
walk(data, functions)
for fn in functions:
    name = fn["name"]
    if not name.startswith(prefix):
        continue
    types = {s["type"] for s in fn.get("stackSlots", [])}
    arms = sorted({ARMS[t] for t in types if t in ARMS})
    print(f"{name}: ARM={','.join(arms) or '-'} GLOBAL={flag(types, 'store_global_new')} "
          f"SUG={flag(types, 'su_global_block')} WITH={flag(types, 'with_target')} "
          f"L1={flag(types, 'state_field_inplace')} REPL={flag(types, 'state_assign_value')} "
          f"FREE={flag(types, 'state_assign_replaced')}")
```

```python
"""Map every `$lambdaN` in an --ncode dump to the probe function that references it.

Usage: python3 lambdas.py <file.ncode>  ->  `<probe fn> <lambda>` per line
"""
import json
import re
import sys

text = open(sys.argv[1]).read()
data = json.loads(text[text.index("{"):])
out = []


def walk(node):
    if isinstance(node, dict):
        if "stackSlots" in node and "name" in node:
            out.append(node)
        for v in node.values():
            walk(v)
    elif isinstance(node, list):
        for v in node:
            walk(v)


walk(data)
for fn in out:
    if fn["name"].startswith("$lambda"):
        continue
    # A lambda is referenced by its mangled symbol (`…24lambda<N>`: `$` is `_24`).
    for n in sorted(set(re.findall(r"lambda(\d+)", json.dumps(fn))), key=int):
        print(fn["name"], f"$lambda{n}")
```

`r6` (the `r.prop = value` row): `MUT r AS P = P[xs := [1], n := 1]` then `r.n = 5`:

```
/tmp/plan-144-probes/r6/src/main.mfb:10 error[1-102-0013 MFB_PARSE_RECORD_FIELD_ASSIGNMENT]: record field assignment is not supported
```

### C.5 Record probe result, by site

`python3 markers.py rec/p144_probe.ncode r` (2,100 probe functions), with each
function's name reduced to its site and the arm names folded into `recfield`, then
`sort | uniq -c`:

```
 339 S10 ARM=- GLOBAL=- SUG=- WITH=y L1=- REPL=- FREE=-
 339 S3 ARM=- GLOBAL=- SUG=- WITH=y L1=- REPL=- FREE=-
 329 S4 ARM=- GLOBAL=- SUG=- WITH=y L1=- REPL=- FREE=-
  10 S4 ARM=recfield GLOBAL=- SUG=- WITH=- L1=- REPL=- FREE=-
 339 S5 ARM=- GLOBAL=y SUG=- WITH=y L1=- REPL=- FREE=-
 339 S6 ARM=- GLOBAL=- SUG=- WITH=y L1=- REPL=- FREE=-
  66 S7 ARM=- GLOBAL=- SUG=- WITH=y L1=- REPL=- FREE=-
 339 S9 ARM=- GLOBAL=- SUG=- WITH=- L1=- REPL=- FREE=-
```

The 10 `S4 ARM=recfield` functions are the 10 arm-backed F1 rows
(`r000` `add`, `r001`/`r002` `append`, `r007` `insert`, `r011` `prepend`, `r012`
`remove`, `r013` `removeAt`, `r014` `removeKey`, `r016`/`r017` `set`). Each fires the
arm its row predicts (`fill_rec.py` checks the arm name). The 339 S9 functions' lambdas
(`markers.py … '$lambda'`) are all `ARM=- WITH=y`: every captured update rebuilds.
The 66 S7 functions are the iterable-field ops (63 F1 + the F5 `List`/`Map`/`Set`
rows). For the other 23 field types, `FOR EACH` over the field does not compile
(`s7na`, C.6).

`fill_rec.py` (C.9) predicts each of the 2,282 cells from the code reading and checks
it against these markers: **0 disagreements** (`rows 326 cells 2282 disagreements 0`).

### C.6 `s7na`, `inl`, `p141c4`

`gen_s7na.py` writes one SUB per non-iterable row field type (23), each
`FOR EACH v IN r.b` over a field of that type. `mfb build s7na` gives 23 errors, all

```
error[2-203-0050 TYPE_FOR_EACH_REQUIRES_COLLECTION]: FOR EACH source must be a List or Map
```

(one per SUB, at the `FOR EACH` line). This is the evidence for every `n/a` S7 cell.

`gen_inl.py` (the B.4 probe):

```python
"""Inlined classification probe (plan-144-A Phase 2).

For every non-collection row field type T: `TYPE L_T { xs AS List OF Integer, f AS T }`
and `r = WITH r { xs := collections::append(r.xs, k) }`. The record arm passes G17
(and fires) iff no inlined field follows `xs`, i.e. iff `record_field_is_inlined(T)`
is false. markers.py: ARM=record_field_append -> T not inlined; WITH=y -> T inlined.
"""
import json
import os
import re

P = "/tmp/plan-144-probes/"
ns = {}
exec(open(P + "rows.py").read().split("def split_params")[0], ns)
INIT = ns["INIT"]
rows = json.load(open(P + "rows.json"))["rows"]
types = sorted({r["ft"] for r in rows if r["ft"] and r["ft"].split()[0] not in ("List", "Map", "Set")})
src = open(P + "rec/src/main.mfb").read()
head = src[:src.index("MUT gR_")]
out = [head]
for t in types:
    i = re.sub(r"[^A-Za-z0-9]", "_", t)
    out += [f"TYPE L_{i}", "  xs AS List OF Integer", f"  f AS {t}", "END TYPE", "",
            f"FUNC inl_{i}(k AS Integer) AS L_{i}",
            f"  MUT r AS L_{i} = L_{i}[xs := [1], f := {INIT[t]}]",
            "  r = WITH r { xs := collections::append(r.xs, k) }", "  RETURN r", "END FUNC", ""]
out += ["SUB main()"] + [f"  inl_{re.sub(r'[^A-Za-z0-9]', '_', t)}(7)" for t in types] + ["END SUB", ""]
os.makedirs(P + "inl/src", exist_ok=True)
open(P + "inl/src/main.mfb", "w").write("\n".join(out))
os.system(f"cp {P}project.json {P}inl/project.json")
print(len(types), "types")
```

`p141c4`: plan-141 Appendix C.4's source, extracted verbatim from
`planning/plan-141-findings/inplace-audit.md` and rebuilt. The markers diff against the
recorded ones is shown in §1b.

### C.7 `fndiff.py`

```python
"""Diff named Rust function bodies between two commits.

Usage: python3 fndiff.py <old-rev> <new-rev> <file>... -- <fn-name-regex>
Prints `same`/`CHANGED`/`missing` per function matched in either revision.
"""
import re
import subprocess
import sys

sep = sys.argv.index("--")
old, new, files, pat = sys.argv[1], sys.argv[2], sys.argv[3:sep], re.compile(sys.argv[sep + 1])


def bodies(rev):
    out = {}
    for f in files:
        try:
            src = subprocess.run(["git", "show", f"{rev}:{f}"], capture_output=True, text=True,
                                 check=True).stdout
        except subprocess.CalledProcessError:
            continue
        for m in re.finditer(r"\n[ \t]*(?:pub\(crate\) )?fn ([a-z_0-9]+)", src):
            name = m.group(1)
            if not pat.search(name):
                continue
            i = src.index("{", m.end())
            depth, j = 0, i
            while True:
                if src[j] == "{":
                    depth += 1
                elif src[j] == "}":
                    depth -= 1
                    if depth == 0:
                        break
                j += 1
            out[name] = re.sub(r"\s+", " ", src[m.start():j + 1])
    return out


a, b = bodies(old), bodies(new)
for name in sorted(set(a) | set(b)):
    if name not in a or name not in b:
        print("missing", name, "old" if name not in a else "new")
    else:
        print("same   " if a[name] == b[name] else "CHANGED", name)
```

`python3 fndiff.py b6a10efbc HEAD bia ipd bc bvs bcl -- '<record/STATE arm and container
names>'` → `same` for every function except `inplace_state_operands_reach_a_state_assign`
(`G25`, a `STATE` gate: plan-144-B).

### C.8 `gen_s7na.py`

```python
"""S7 n/a evidence: `FOR EACH v IN r.b` over a field of every non-iterable row type.

Writes /tmp/plan-144-probes/s7na/src/main.mfb, one SUB per field type; the build's
diagnostics (one per SUB) are the n/a evidence for that type's S7 cells.
"""
import json
import os
import re

P = "/tmp/plan-144-probes/"
ns = {}
exec(open(P + "rows.py").read().split("def split_params")[0], ns)
INIT = ns["INIT"]
rows = json.load(open(P + "rows.json"))["rows"]
types = sorted({r["ft"] for r in rows if r["ft"] and r["ft"].split()[0] not in ("List", "Map", "Set")})
src = open(P + "rec/src/main.mfb").read()
head = src[:src.index("MUT gR_")]  # imports, types, helpers
out = [head]
for t in types:
    i = re.sub(r"[^A-Za-z0-9]", "_", t)
    out += [f"SUB s7_{i}()", f"  MUT r AS Rec_{i} = Rec_{i}[a := {INIT[t]}, b := {INIT[t]}]",
            "  FOR EACH v IN r.b", "    io::print(\"x\")", "  NEXT", "END SUB", ""]
out += ["SUB main()", "  io::print(\"x\")", "END SUB", ""]
os.makedirs(P + "s7na/src", exist_ok=True)
open(P + "s7na/src/main.mfb", "w").write("\n".join(out))
os.system(f"cp {P}project.json {P}s7na/project.json")
print(len(types), "types:", types)
```

### C.9 `fill_rec.py` — the §1 fill and the reading-vs-dump check

```python
"""Fill plan-144 §1 (record sites) from the code reading, checked against the dump.

  python3 fill_rec.py  -> writes rec/table.md (the §1 rows); prints the reading-vs-dump
                          disagreements (must be 0) and the cell count

The verdict of each cell is PREDICTED from the code reading (the rules below, cited in
the findings file's path legend), then CHECKED against the probe's markers:
  y            <=> an ARM=record_field_* marker and no WITH (`with_target`) marker;
  n at S3/S4/S6/S7/S10 <=> ARM=- and WITH=y (the `lower_with_update` rebuild);
  n at S9      <=> the lambda the S9 FUNC lifts has ARM=- and WITH=y;
  n at S5      <=> GLOBAL=y (`store_global_new`), SUG=- (the seam was not reached).
When the reading and the dump disagree, the dump wins and the cell says so.
"""
import json
import re

P = "/tmp/plan-144-probes/"
data = json.load(open(P + "rows.json"))
rows = data["rows"]


def load_markers(path):
    m = {}
    for line in open(path):
        name, _, rest = line.strip().partition(": ")
        m[name] = dict(kv.split("=", 1) for kv in rest.split())
    return m


mk = load_markers(P + "rec/markers.txt")
lam_of = dict(l.split() for l in open(P + "rec/lambdas.txt"))
lam = {}
for line in open(P + "rec/lambda_markers.txt"):
    name, _, rest = line.strip().partition(": ")
    lam[name] = dict(kv.split("=", 1) for kv in rest.split())

# The 10 overloads a record-field arm serves (Appendix B), keyed on the row's op.
ARM_OF = {
    "collections::add({X}, k)": "record_field_set_add",
    "collections::append({X}, k)": "record_field_append",
    "collections::append({X}, [k, k])": "record_field_append",
    "collections::insert({X}, 0, k)": "record_field_splice(insert/prepend)",
    "collections::prepend({X}, k)": "record_field_splice(insert/prepend)",
    "collections::remove({X}, k)": "record_field_set_remove",
    "collections::removeAt({X}, 0)": "record_field_remove_at",
    "collections::removeKey({X}, k)": "record_field_remove_key",
    "collections::set({X}, 0, k)": "record_field_set(List)",
    "collections::set({X}, k, k)": "record_field_set(Map)",
}
SITES = ["S3", "S4", "S5", "S6", "S7", "S9", "S10"]
ITERABLE = ("List", "Map", "Set")


def predict(row, site):
    """The code reading: (verdict, path letter) for one cell."""
    ft = row["ft"]
    iterable = ft.split()[0] in ITERABLE
    arm = ARM_OF.get(row["ops"][0]) if row["fam"] == "F1" else None
    if site == "S7" and not iterable:
        return "n/a (not iterable)", None
    if site == "S5":
        return "n (StoreGlobal)", "P2"
    if arm is None:
        return "n (no arm)", "P1"
    return {"S3": ("n (G17)", "P1"), "S4": ("y", "A"), "S6": ("n (G17)", "P1"),
            "S7": ("n (G15)", "P1"), "S9": ("n (G1)", "P1"), "S10": ("n (G14)", "P1")}[site]


def observed(fn, site):
    if site == "S9":
        m = lam[lam_of[fn]]
        return m, f"{lam_of[fn]} ARM={m['ARM']} WITH={m['WITH']}"
    m = mk[fn]
    if site == "S5":
        return m, f"ARM={m['ARM']} GLOBAL={m['GLOBAL']} SUG={m['SUG']} WITH={m['WITH']}"
    return m, f"ARM={m['ARM']} WITH={m['WITH']}"


def agrees(verdict, site, m, arm):
    if verdict.startswith("n/a"):
        return m is None
    if verdict == "y":
        return m["ARM"] == arm and m["WITH"] == "-"
    if site == "S5":
        return m["ARM"] == "-" and m["GLOBAL"] == "y" and m["SUG"] == "-"
    return m["ARM"] == "-" and m["WITH"] == "y"


out, disagree, cells = [], [], 0
for row in rows:
    label = row["label"].replace("|", "\\|")
    if row["ft"] is None:  # `r.prop = value`
        na = "n/a (not expressible)"
        out.append(f"| {row['fam']} {label} | — | " + " | ".join([na] * 7)
                   + " | Parse error `1-102-0013 MFB_PARSE_RECORD_FIELD_ASSIGNMENT` (probe `r6`, "
                   "Appendix C.4); rule at `src/rules/table.rs:186`. `WITH` is the only record "
                   "update form. |")
        cells += 7
        continue
    arm = ARM_OF.get(row["ops"][0]) if row["fam"] == "F1" else None
    verdicts, ev = [], []
    for site in SITES:
        v, path = predict(row, site)
        cells += 1
        per_op = []
        for oi in range(len(row["ops"])):
            fn = f"{row['id']}o{oi}_{site}"
            if v.startswith("n/a"):
                if fn in mk:
                    disagree.append((fn, v, "compiled"))
                continue
            m, text = observed(fn, site)
            if not agrees(v, site, m, arm):
                disagree.append((fn, v, text))
            per_op.append(text)
        if v.startswith("n/a"):
            v = "n/a (not a `FOR EACH` iterable)"
        verdicts.append(v)
        if per_op and len(set(per_op)) == 1:
            ev.append(f"{site} {per_op[0]}")
        elif per_op:
            ev.append(f"{site} " + "; ".join(sorted(set(per_op))))
    nops = len(row["ops"])
    probe = f"`{row['id']}o0_S*`" if nops == 1 else f"`{row['id']}o0…o{nops - 1}_S*` ({nops} overloads, identical markers)"
    s7 = "" if row["ft"].split()[0] in ITERABLE else " S7: `TYPE_FOR_EACH_REQUIRES_COLLECTION` (probe `s7na`)."
    path = "A at S4; P1 elsewhere; P2 at S5" if arm else "P1; P2 at S5"
    form = row.get("form", "—")
    out.append(f"| {row['fam']} {label} | {form} | " + " | ".join(verdicts)
               + f" | Path {path}. Field `{row['ft']}`. Probe {probe}: " + ", ".join(ev) + "." + s7 + " |")

open(P + "rec/table.md", "w").write("\n".join(out) + "\n")
print("rows", len(out), "cells", cells, "disagreements", len(disagree))
for d in disagree[:20]:
    print(d)
```
