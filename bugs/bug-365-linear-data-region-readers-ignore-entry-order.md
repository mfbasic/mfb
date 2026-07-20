# bug-365: every consumer that reads a list's data region linearly — the `math::` SIMD overloads, `fs` byte writes, and by inspection `audio`/`net`/`tls`/`crypto` — silently reorders any list built with `prepend`, `insert`, or a value-path `set`

Last updated: 2026-07-19
Effort: medium (1h–2h)
Severity: HIGH
Class: Miscompile (silent wrong answer / data corruption)

Status: Open
Regression Test: needed — `tests/rt-behavior/math/math-array-entry-order-rt` and `tests/rt-behavior/fs/fs-write-bytes-entry-order-rt` (new; see Validation Plan)

A `List`'s payloads are densely packed but **not necessarily in index order**.
`prepend`, mid-list `insert`, and the value-semantic `set` deliberately leave the
data region permuted relative to entry order — that is the documented
"offset-stable scheme" (`plan-01 §4.1`), whose whole point is to splice the
lookup table instead of moving payload bytes.

Most of the language honors this: `FOR EACH`, `collections::get`, `sum`,
`contains`, `find`, `transform` all read through `entry[i].valueOffset` and are
correct. But an entire class of consumer takes the data-region base and then
walks it linearly, assuming element `i` lives at `dataBase + i * width`. Every
one of those silently returns the elements in the wrong order.

Two families are **confirmed by reproduction**, in two different packages:

- `math::abs`, `math::min` and the rest of the vectorized array overloads —
  wrong numbers, wrong order.
- `fs::writeBytesAtomic` — **writes the wrong bytes to disk.** A `List OF Byte`
  holding `Z A B C` produces a file containing `ABCZ`.

A third set is unverified but reads identically by inspection and is called out
in §Scope: `audio::write`, the `net`/`tls` socket writes, and the `crypto`
byte-list paths. `audio::write` is the alarming one — a permuted PCM buffer
plays back scrambled.

The result is not a crash and not a diagnostic. It is wrong data, on valid
programs, from functions whose documentation promises the opposite: `mfb man fs
writeBytesAtomic` says the bytes are written "in order, taken verbatim from the
list's data region" — those two clauses are not the same thing, and where they
disagree the data region wins.

## Reproduction

```basic
IMPORT io
IMPORT math
IMPORT collections

FUNC show(label AS String, xs AS List OF Integer) AS Nothing
  MUT s AS String = ""
  FOR EACH v IN xs
    s = s & toString(v) & " "
  NEXT
  io::print(label & s)
END FUNC

FUNC main() AS Integer
  MUT a AS List OF Integer = [-1, -2, -3]
  a = collections::prepend(a, -9)
  show("prepend  logical : ", a)
  show("prepend  abs     : ", math::abs(a))
  show("prepend  min     : ", math::min(a, [0, 0, 0, 0]))
  io::print("prepend  sum     : " & toString(collections::sum(a)))

  MUT b AS List OF Integer = [-1, -2, -3]
  b = collections::insert(b, 1, -9)
  show("insert   logical : ", b)
  show("insert   abs     : ", math::abs(b))

  LET c = a
  show("copied   logical : ", c)
  show("copied   abs     : ", math::abs(c))
  RETURN 0
END FUNC
```

Observed on macos-aarch64, release build, 2026-07-19:

```
prepend  logical : -9 -1 -2 -3
prepend  abs     : 1 2 3 9          <-- WRONG, expected: 9 1 2 3
prepend  min     : -1 -2 -3 -9      <-- WRONG, expected: -9 -1 -2 -3
prepend  sum     : -15              <-- correct (sum walks entries)

insert   logical : -1 -9 -2 -3
insert   abs     : 1 2 3 9          <-- WRONG, expected: 1 9 2 3

copied   logical : -9 -1 -2 -3
copied   abs     : 1 2 3 9          <-- WRONG; a value copy does NOT normalize
```

### The `fs` case — data corruption on disk

```basic
IMPORT io
IMPORT fs
IMPORT collections

FUNC main() AS Integer
  MUT b AS List OF Byte = [65, 66, 67]     ' A B C
  LET z AS Byte = 90                       ' Z
  b = collections::prepend(b, z)           ' logical: Z A B C
  MUT s AS String = ""
  FOR EACH v IN b
    s = s & toString(v) & " "
  NEXT
  io::print("logical bytes : " & s)
  fs::writeBytesAtomic("/tmp/out.bin", b)
  RETURN 0
END FUNC
```

```
logical bytes : 90 65 66 67
expected file : ZABC
actual file   : ABCZ          <-- WRONG
```

This one is worse than the `math::` case: the wrong bytes are **persisted**. A
program that builds a buffer with `prepend` and writes it out produces a
corrupt file, with no error at any layer.

### What the reproductions pin down

1. **`FOR EACH` and `math::abs` disagree about the same list.** One of them is
   wrong, and it is not `FOR EACH`.
2. **`collections::sum` is correct** on the identical value, because it reads
   `entry[i].valueOffset` (`builder_collection_queries.rs:1377`). So this is not a
   property of the list — it is a property of the consumer.
3. **A value-semantic copy does not launder it.** `copy_collection_tight` copies
   the entry table and the data region as two verbatim block copies
   (`builder_collection_layout.rs:392-442`), so the permutation survives
   assignment, argument passing, record embedding, and thread transfer.
4. **It is not confined to codegen'd arithmetic.** The `fs` case shows the same
   defect reaching persistent storage through an ordinary built-in, which is why
   the fix has to be a stated contract plus an audit, not a patch to the SIMD
   kernels alone.

## Root cause

Two shapes of the same mistake.

The SIMD kernels take the data base and stride it:

- `builder_simd_math.rs:175, 177` (`lower_simd_unary`), `:668, 670, 672`
  (`lower_simd_binary`), `:887, 889` (`lower_simd_clamp`) — each calls
  `emit_collection_data_pointer` and then indexes by lane width.
- Same shape in `builder_simd_float_math.rs:370, 372, 2186-2190`,
  `builder_simd_fixed_math.rs:66, 68, 318, 321`, and `builder_pow.rs:456-462`.

The byte-list writers do the same thing with a `memcpy`-shaped loop: take the
data base and copy `dataLength` bytes straight out
(`fs_helpers_atomic.rs:491, 1440`).

`emit_collection_data_pointer` (`builder_collection_layout.rs:1725-1746`) is
correct — it derives the base from `capacity`, not `count`. The defect is the
assumption that follows it: that element `i` lives at `dataBase + i*width`, and
therefore that the data region read front-to-back yields the elements in order.

Note how easy the mistake is to make: the capacity-vs-count trap is documented in
comments at both audio backends and in the spec, so everyone remembered *that*
one. The ordering assumption sits one inch away and is written down nowhere,
which is why it was made independently in at least two packages.

That assumption is contradicted in the tree, in writing:

- `builder_collection_mutate.rs:1616-1619` (`lower_list_prepend_in_place`):
  *"shifts the live lookup entries right by one … and appends the element's
  payload to the spare data tail — **entry offsets are independent of position, so
  no data move is needed**."*
- `builder_collection_mutate.rs:468-472` (`lower_list_insert_collection`): the
  "offset-stable scheme (plan-01 §4.1): copy `A`'s and `B`'s data regions verbatim
  (**no per-entry repack**), then splice the lookup table with three block moves."
- `builder_collection_mutate.rs:3267-3268` (`lower_list_remove_at`): *"Testing
  each entry's own offset keeps it correct for **a list whose data is out of entry
  order after an insert/prepend/set**."*
- `collections::set`'s value path is `removeAt(i)` + `insert(i, singleton)`
  (`builder_collection_mutate.rs:330-346`), so it permutes for any `i < count-1`.

Two consumers already handle disorder correctly and are the model for the fix:

- `collections::mid` carries a **runtime order/tightness probe**
  (`builder_search.rs:876-908`) that compares each `valueOffset` against the
  running expected offset and falls back to a per-entry repack when they differ.
  Its comment names the real-world case: a sorted `fs::listDirectory` result.
- `lower_list_slice_range` (`builder_collection_queries.rs:1275-1310`) rewrites
  `valueOffset` to a running counter, normalizing unconditionally.

## Scope

**Confirmed by reproduction:**

- Every `math::` list overload routed through a SIMD kernel — `abs`, `sqrt`,
  `min`, `max`, `clamp`, `floor`, `ceil`, `round`, `exp`, `log`, the trig family,
  `pow`, `atan2` (`builder_math.rs:85-649`), over `List OF Integer`,
  `List OF Float`, `List OF Fixed`.
- `fs::writeBytesAtomic` (`fs_helpers_atomic.rs:491, 1440`).

**Unverified but reads linearly by inspection — check each before closing:**

| consumer | site |
|---|---|
| `audio::write` | `audio/alsa.rs:1140-1145`, `audio/macos.rs:872-877` |
| `fs::writeBytes`, `readAllBytes`, `writeAllBytes` | `fs_helpers_io.rs:1871, 2084` |
| `fs::pathJoin`, `listDirectory` | `fs_helpers_paths.rs:1008, 1671-1693` |
| socket send/recv | `net/io.rs:643, 799, 1036, 1573, 1781` |
| TLS read/write | `tls/openssl.rs:1969, 2111`, `tls/macos.rs:1410, 1605` |
| `crypto` byte lists | `crypto.rs:142`, `crypto_ec.rs:155, 221` |

`audio::write` is the priority — it is the one that turns a permuted list into
scrambled audio, and it is on the critical path for
`planning/plan-57-E-libsnd-loadsound.md`.

**Not affected**: everything that reads through the entry table — `FOR EACH`,
`collections::get`/`getOr`/`sum`/`contains`/`find`/`transform`/`filter`/`reduce`,
`mid`, `slice`. Maps are unaffected.

**Why it has stayed latent**: a list built only by literal, `append`,
`transform` or `filter` is ordered by construction, and that covers almost all
in-tree usage. It takes one `prepend`, one mid-list `insert`, or one value-path
`set` anywhere in the value's history to arm it.

Note the bug is **latent for lists built only by literal, `append`, or
`transform`/`filter`**, which is why it has not been seen: those paths preserve
index order by construction. It requires one `prepend`, one mid-list `insert`, or
one value-path `set` anywhere in the value's history.

## Fix

Three options, in increasing order of cost and value:

1. **Probe and repack at each linear consumer** — mirror `lower_list_mid`'s order
   probe (`builder_search.rs:876-908`): scan the entries, and if any `valueOffset`
   deviates from the running expected offset, take a normalizing path. Cheapest
   correct fix, keeps the ordered fast path fast, and is **already proven in
   production** — see the empirical table below, where `mid` restores order on a
   permuted input. Recommended. Extract the probe into one shared helper rather
   than copying it per consumer; the count of affected sites is the whole problem.
2. **Normalize at the source** — have `prepend`/`insert`/value-`set` restore
   index order. This abandons the offset-stable scheme and pays an O(n) data
   memmove per operation. It would make the ordering invariant global and let
   *every* consumer drop its entry loads. See §Related.
3. **Assert in `ir::verify`** — not possible; ordering is a runtime property.

Whichever is taken, the ordering contract must be **written down** (§The ordering
contract). Today `src/docs/spec/memory/05_collections.md` describes the layout but
never states whether a reader may assume index order — which is precisely how a
kernel came to assume it.

## The ordering contract

To be added to `src/docs/spec/memory/05_collections.md`, as a subsection of
*Capacity Headroom and Growth* (`:173-198`) — it is the same class of rule as the
existing "always derive the data base from `capacity`, never from `count`" warning
at `:187-190`, and belongs beside it.

> ### Payload Order
>
> A `List`'s payloads are **densely packed but not necessarily in index order**.
> The lookup table, not the data region, defines the sequence.
>
> Element `i`'s payload is located **only** by `entry[i].valueOffset` and
> `entry[i].valueLength`, relative to the capacity-derived data base. A reader
> **must not** assume element `i` begins at `dataBase + i * payloadSize`, and
> must not assume that walking the data region linearly visits elements in index
> order. This holds for **every** element type, including fixed-width scalars
> with no inter-element padding: `list_element_padding_alignment` returning 1
> guarantees there are no *gaps*, not that the payloads are in *order*.
>
> The permutation is a deliberate consequence of the offset-stable scheme
> (plan-01 §4.1): `prepend` and `insert` splice the lookup table and append the
> new payload to the data tail rather than moving `n` payload bytes, and the
> value-semantic `set` is `removeAt` + `insert`.
>
> Order is a property of the **value**, not of a moment: it survives every copy.
> `copy_collection_tight` copies the entry table and the data region as two
> verbatim block copies, so a permuted list stays permuted across assignment,
> argument passing, record embedding, and thread transfer. Nothing in the
> value-copy path normalizes it.
>
> A consumer that requires a densely-ordered buffer — a vectorized kernel, a
> `memcpy` to a native API — must **establish** that order rather than assume it,
> by one of the two idioms already in the tree:
>
> - **probe and repack**: scan the entries against a running expected offset and
>   take a normalizing fallback when they diverge (`collections::mid`,
>   `builder_search.rs:876-908`);
> - **rebuild**: construct a fresh list by appending in index order, which
>   normalizes as a side effect (`transform`, `filter`, `lower_list_slice_range`).

Per-operation behavior, verified empirically on macos-aarch64 (2026-07-19) by
comparing `FOR EACH` order against `math::abs` order — `FOR EACH` reads the entry
table, `math::abs` reads the data region, so agreement means ordered and
divergence means permuted:

| operation | effect on index order |
|---|---|
| list literal | ordered by construction |
| `append` (value and in-place) | **preserves** — ordered stays ordered, permuted stays permuted |
| `prepend` (value and in-place) | **breaks** — new payload goes to the data tail |
| `insert` (mid-list) | **breaks** — same |
| `set` (in-place fast path) | preserves — writes through the stored offset |
| `set` (value path, `i < count-1`) | **breaks** — is `removeAt` + `insert` |
| `removeAt` | preserves; never restores |
| grow / realloc / `copy_collection_tight` | neutral — verbatim block copies |
| `mid` | **restores** (probe + per-entry repack) |
| `slice` (range index) | **restores** (rewrites offsets to a running counter) |
| `transform` / `filter` | **restores** (rebuilds by append) |
| `sort` | preserves — MFB source using the in-place `set` fast path, which permutes *data*, leaving entries as the identity |

Observed output for the append/`mid`/`transform`/`removeAt` rows, on a list
permuted by a preceding `prepend`:

```
prepend       logical  : -9 -1 -2 -3      physical : 9 1 2 3   (permuted)
then append   logical  : -9 -1 -2 -3 -4   physical : 1 2 3 9 4 (still permuted)
then mid      logical  : -9 -1 -2 -3      physical : 9 1 2 3   (restored)
then transfrm logical  : -9 -1 -2 -3      physical : 9 1 2 3   (restored)
then removeAt logical  : -9 -1 -2         physical : 1 2 9     (still permuted)
```

That `mid` restores order on a permuted input is the direct evidence that fix
option 1 is workable: the probe-and-repack idiom is already in production and
already correct.

## Validation Plan

- **Regression test**: new `tests/rt-behavior/math/math-array-entry-order-rt/`.
  It must permute a list (`prepend`, and separately a mid-list `insert`) and then
  assert every affected `math::` array overload against the **logical** order, for
  `List OF Integer`, `List OF Float` and `List OF Fixed`. Assert `FOR EACH` and
  the kernel agree — that equivalence is the invariant, and it fails today.
- **Cover the copy path**: bind the permuted list to a new `LET` and re-run the
  kernel. `copy_collection_tight` does not normalize, so a fix that only handles
  freshly-permuted lists would pass a naive test and still be wrong.
- **Cover the ordered fast path**: a list built only by literal/`append` must
  produce byte-identical codegen and identical results, so the probe does not
  regress the common case. `scripts/artifact-gate.sh`.
- **Spec sync** (`.ai/specifications.md`): add the *Payload Order* subsection
  above to `src/docs/spec/memory/05_collections.md`. This is a required part of
  the fix, not a follow-up — the missing contract is the root cause, and leaving
  it unwritten invites the next kernel to make the same assumption.
- **Audit every consumer in §Scope's unverified table.** Each is a
  `emit_collection_data_pointer` call followed by a computed stride rather than a
  loaded `valueOffset`. Two of these were found by reproduction after being
  predicted by inspection, so the inspection is reliable — treat the table as a
  worklist, not a guess, and add a runtime case per entry. **`audio::write`
  first**: it is the one that plays scrambled audio, and it is on the critical
  path for `planning/plan-57-E-libsnd-loadsound.md`.
- **Acceptance**: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Related

Found while investigating whether the 40-byte `LookupEntry` could be dropped for
fixed-width element lists (a `List OF Byte` costs 41 bytes per byte; see
`planning/plan-57-C-cbuffer-marshaling.md` §Open Decisions). The audit that
found this bug also established:

- For a fixed-width list, `flags`, `keyOffset`, `keyLength` and `valueLength` are
  all dead (32 of 40 bytes) — `flags` is written everywhere and read only by one
  guard in `builder_arena_transfer.rs:727` that nothing can ever trip, since no
  code path clears the USED bit. There are **no tombstones anywhere** in the
  codebase; `removeAt` compacts.
- `valueOffset` is the one live field, purely because of the offset-stable scheme.

So fix option 2 above would also unlock removing the entry table for fixed-width
lists entirely (41× → 1×). Worth noting that for fixed-width elements the
offset-stable scheme is a **pessimization**: prepending to a `List OF Byte` shifts
40 bytes of entry per element to avoid moving 1 byte of payload. That is a
separate investigation, and it should not be bundled into this fix — this bug is
a correctness defect and should land on its own, with its own test.
