# plan-122-A: The `color` builtin package — core value type and constructors

Last updated: 2026-09-02
Overall Effort: huge (> 3d)
Effort: large (3h–1d)
Depends on: nothing

MFBASIC has three unrelated notions of a colour today: `canvas::Color` (a 4-`Byte`
RGBA record), `term::TermColor` (a 3-`Byte` RGB record the runtime allocates), and
an `astrings` foreground/background attribute carrying a packed `0xRRGGBB`
`Integer` with no type at all. Nothing converts between them. plan-122 replaces all
three with one `color::Color`.

This sub-plan builds the **package** and nothing else: registration on the
clean-room registry, the `Color` record, the clamping constructors, the packed-
integer bridge, hex parsing and rendering, and the `toString` override. **No
consumer changes** — `canvas::Color`, `term::TermColor` and `astrings::foreground`
are untouched and still work exactly as they do today, so this letter lands with a
green `cargo test` and zero golden churn outside the new fixtures.

Behavioral outcome: a program can `IMPORT color`, build a `color::Color` from
components / a packed integer / a hex string, render it back to hex, and read its
four `Byte` channels — proved by an rt-behavior fixture and `mfb man color`.

References:

- `.ai/resources-packages.md` §"New builtin-package registration seams" — the
  authoritative seam checklist. **Two of its bullets are stale** (§7 task): there is
  no `src/builtins/<pkg>.rs` and no `descriptor.rs` (`ls src/builtins` →
  `No such file or directory`); those seams moved into `src/codegen/builtins/` and
  `src/codegen/registry/`.
- `.ai/resources-packages.md` §".mfb source-language authoring gotchas" — record
  literals are positional `Type[...]`, record fields are not assignable, reserved
  words cannot be field or parameter names. Every one of these bites when writing a
  pure-source package.
- `.ai/man-content.md` — the man-page content standard every new member must meet.
- `.ai/testing-gates.md` — artifact gate, acceptance golden harness, byte-identity.
- `mfb spec` §18 (builtin packages), §14 (value/copy semantics), §4.2 (records).
- `src/codegen/builtins/encoding/` — the closest existing model: a **pure-source**
  package, every member `Body::mfb`, one `func_*.rs` per member plus `helper_*.rs`
  chunks.
- `src/codegen/builtins/vector/mod.rs` — the `add_constant` record-constant
  precedent (`vector::zeroFloat3`) that plan-122-C's named colours reuse.

## Prerequisites

These are a precondition on the whole plan-122 feature, not a dependency to
negotiate. Sub-plans B–F point here.

| Must be true | Command | Status |
|---|---|---|
| A release binary matching `main` exists (tests run the RELEASE `mfb`) | `cargo build --release && ls -l target/release/mfb` | MET — re-measured 2026-09-03 in the `P-122` worktree at `0a1f0f6b3`: 32,997,248 bytes |
| Working tree is clean enough to attribute golden churn | `git status --porcelain` → only the 5 known `examples/browser` edits | MET — re-measured 2026-09-03: `git status --porcelain` on `main` is **empty**. The 5 `examples/browser` edits were landed between authoring and execution, so golden attribution in D/E/F is clean |
| The registry qualifies cross-package value-type leaves automatically | `grep -n "fn qualify_value_type_references" src/codegen/registry/mod.rs` → 1665 | MET — re-measured 2026-09-03: still `src/codegen/registry/mod.rs:1665` |
| Rule code / error code for a malformed hex string already exists | `grep -rn "ErrInvalidFormat" src/codegen/builtins/errorcode/mod.rs` → `77050003` | MET — re-measured 2026-09-03: `errorcode/mod.rs:78`. **No new `data_objects.rs` row is needed** (see `new-error-in-a-package-needs-a-data-object-row`) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* prerequisites.

Everything below is written against the world where these hold.

## 1. Goal

- A `color` builtin package exists and is importable from any build (not
  `--app`-gated), exporting the record `color::Color` with `Byte` fields
  `red`/`green`/`blue`/`alpha`.
- `color::rgb`, `color::rgba`, `color::gray`, `color::withAlpha`, `color::invert`,
  `color::toPacked`, `color::fromPacked`, `color::fromHex`, `color::toHex`,
  `color::toHexAlpha` all work, are documented on `mfb man color`, and every man
  example compiles and runs.
- `toString(c)` on a `color::Color` renders `#rrggbbaa`.
- `color::fromHex` accepts `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` and the same
  four without the leading `#`, case-insensitively, and raises `ErrInvalidFormat`
  on anything else.

### Non-goals (explicit constraints)

- **No consumer changes in this letter.** `canvas::Color`, `canvas::rgb`,
  `canvas::rgba`, `term::TermColor`, `astrings::foreground`/`background` keep
  their current signatures and behavior. Their removal is D, E and F.
- **No colour-space maths.** `brighten`/`darken`/`luminance`/HSL are B. Nothing in
  A converts between sRGB and linear light, and A does **not** move canvas's sRGB
  table.
- **No named colours.** `fromName`/`nameOf` and the CSS table are C.
- **No new error code.** `fromHex` raises the existing `ErrInvalidFormat`
  (`77050003`).
- **No transcendentals anywhere in `color`.** canvas's software rasteriser is the
  oracle its GPU backends are compared against and must produce identical bytes on
  every target (`src/codegen/builtins/canvas/helper_color.rs:1-11`); from B onward
  canvas calls into `color`, so the whole package inherits that rule. IEEE `+ - * /
  sqrt` only — no `pow`, no trig.
- **`Color` is a plain value record, not a resource and not read-only.** Unlike
  `term::TermColor` (which the runtime allocates, so `is_read_only_record` refuses
  construction — `src/codegen/builtins/term/mod.rs:100-108`), a program may build a
  `color::Color` with a record literal and `WITH`-update one.

## 2. Current State

### The three colour representations

| Where | Shape | Built by | Cited |
|---|---|---|---|
| `canvas` | record `Color { red, green, blue, alpha AS Byte }` | `canvas::rgb`/`rgba`, clamping through `__canvas_clampByte` | `src/codegen/builtins/canvas/mod.rs:183`, `func_rgb.rs:36`, `func_rgba.rs:52`, `helper_clamp_byte.rs:13` |
| `term` | record `TermColor { r, g, b AS Byte }`, **runtime-allocated, read-only** | never by a program; only read back from `term::getForeground`/`getBackground` | `src/codegen/builtins/term/mod.rs:89-95`, `:190`; hand-emitted at `src/codegen/term/core/term.rs:2069` |
| `astrings` | no type — a packed `0xRRGGBB` `Integer` in `AttrNumber.value` | `astrings::foreground(r,g,b)` / `background` via `__astrings_packColor` | `src/codegen/builtins/astrings/func_foreground.rs:38`, `helper_pack_color.rs:16`, `mod.rs:170` |

The `term`↔`astrings` bridge is where the absence of a shared type is most
visible: it re-implements channel extraction by hand
(`__term_colorR`/`G`/`B`, `src/codegen/builtins/term/helper_astrings_bridge.rs:97-107`)
and uses `-1` as an "unset" sentinel that is only sound because a packed colour is
always in `0..0xFFFFFF`.

### The seams a new package must touch

Measured by reading each site, not from the doc:

| Seam | File:line | What |
|---|---|---|
| module decl | `src/codegen/builtins/mod.rs:5-35` | `pub(crate) mod color;` in the sorted list |
| import gate | `src/codegen/builtins/mod.rs:50` | `is_builtin_import` match arm |
| argument checking | `src/codegen/builtins/mod.rs:451` | `ARGUMENT_CHECKED_PACKAGES` — **without a row here an arity/type mistake in a `color::` call degrades to a bare `TYPE_UNKNOWN_VALUE` with no diagnostic** |
| test census | `src/codegen/builtins/mod.rs:935` | `ALL_BUILTIN_PACKAGES` + the second list in `is_builtin_import_cases` |
| registry build | `src/codegen/registry/mod.rs:1937` | `crate::codegen::builtins::color::register(&mut r);` in `build()` |
| resolver type seed | `src/resolver/mod.rs:20-58` | `BUILTIN_TYPES` needs the **package-qualified** `color.Color` id, never the bare leaf (bug-484 comment at `:33-40`) |
| §18 package list | `src/docs/spec/language/18_builtin-functions.md:46` | pinned by `spec_section_18_package_list_matches_is_builtin_import` (`src/codegen/builtins/mod.rs:967`) — the test fails if the sentence and `is_builtin_import` disagree |

`mfb man` needs **no** list update: the package index is derived from the registry
(`src/cli/man.rs` resolves by name; there is no hand-written package table).

### Measured populations

| What | Count | Command |
|---|---|---|
| `canvas::rgb(` occurrences, whole tree | 141 | `grep -rn 'canvas::rgb(' --include='*.mfb' --include='*.rs' --include='*.md' . \| wc -l` |
| — of those, in `src/` / `tests/` / `examples/` | 19 / 110 / 8 | `for d in src tests examples; do grep -rn 'canvas::rgb(' $d \| wc -l; done` |
| `canvas::rgba(` occurrences | 14 (2 src / 11 tests / 0 examples) | same, `canvas::rgba(` |
| `canvas::Color` occurrences | 57 (24 src / 27 tests / 5 examples) | same, `canvas::Color` |
| `TermColor` occurrences (all spellings incl. `TERM_COLOR_*`) | 109 (38 src / 63 tests / 6 planning / 2 bugs) | `for d in src tests planning bugs; do grep -rn 'term::TermColor\|term\.TermColor\|TERM_COLOR' $d \| wc -l; done` |
| `astrings::foreground(`/`background(` occurrences | 24 | `grep -rn 'astrings::foreground(\|astrings::background(' --include='*.mfb' --include='*.rs' --include='*.md' . \| wc -l` |
| `.mfb` fixtures importing `term` | 45 | `grep -rl '^IMPORT term' --include='*.mfb' tests/ examples/ \| wc -l` |
| `.mfb` fixtures importing `astrings` | 15 | `grep -rl '^IMPORT astrings' --include='*.mfb' tests/ examples/ \| wc -l` |
| golden fixture dirs under `tests/rt-behavior/term` + `tests/syntax/term` | 14 + 23 | `ls -d tests/rt-behavior/term/*/ \| wc -l; ls -d tests/syntax/term/*/ \| wc -l` |
| golden fixture dirs under `tests/rt-behavior/astrings` + `tests/syntax/astrings` | 9 + 3 | same for `astrings` |
| `.ncodesum` files under `tests/byte-identity/term` | 5 | `find tests/byte-identity/term -name '*.ncodesum' \| wc -l` |
| `.ncodesum` files, whole byte-identity tree | 133 | `find tests/byte-identity -name '*.ncodesum' \| wc -l` |
| Rust test files naming the colour surface | 10 | `grep -rln 'canvas::rgb\|canvas::Color\|term::TermColor\|astrings::foreground\|astrings::background' tests/*.rs \| wc -l` |
| example `.mfb` files naming it | 3 | same over `examples/` |
| spec/man docs naming it | 5 | same over `src/docs/` |

### Verified properties

- **Cross-package value types work, and imports are NOT transitive.** Verified by
  building a probe: a project with `IMPORT udp` (no `IMPORT net`) doing
  `LET addr = udp::localAddress(sock)` then `addr.host` fails with
  `2-203-0043 TYPE_UNKNOWN_VALUE`: *"Field `host` belongs to `net::net.Address`,
  whose fields are not visible in this file. Imports are not transitive and a
  package cannot re-export another's types, so add `IMPORT net`."*
  **Consequence for plan-122: every program that reads a channel off a
  `color::Color` must `IMPORT color`, no matter which package handed it over.**
  This is why "keep `canvas::rgb` as an alias" would not have spared callers
  anything, and why removing the old spellings outright is the right call.
- **That diagnostic renders a doubled qualifier** — `net::net.Address`, because
  `src/ir/shape.rs:1868` interpolates `{owner}::{type_name}` while `type_name` is
  already package-qualified. It is a live cosmetic bug that plan-122 will make far
  more visible (every colour consumer will hit it during D/E/F). Pinned by 5
  goldens under `tests/syntax/tcp/`. **Fixed out-of-band before this plan starts —
  see §7.**
- **A pure-source package's entire companion is compiled into every importing
  binary, used or not.** Measured by building `IMPORT io` + `IMPORT <pkg>` with no
  call to `<pkg>` and comparing `build/*.out` sizes against the 66,596-byte
  `IMPORT io` baseline:

  | package | bytes | delta | companion |
  |---|---|---|---|
  | `bits`, `math`, `collections`, `strings`, `os`, `money`, `term`, `tcp`, `tls` | 66,596 | **0** | empty or type-declarations-only |
  | `net`, `udp` | 182,180 | +115,584 | pure-source |
  | `datetime` | 281,412 | +214,816 | pure-source |
  | `vector` | 396,836 | +330,240 | pure-source |
  | `encoding` | 495,908 | +429,312 | pure-source |
  | `csv` | 545,444 | +478,848 | pure-source |
  | `json` | 644,516 | +577,920 | pure-source |
  | `regex` | 1,007,780 | +941,184 | pure-source |
  | `astrings` | 1,139,876 | +1,073,280 | pure-source |
  | `crypto` | 1,404,180 | +1,337,584 | pure-source |

  Command (per package): `mfb init /tmp/szp; printf 'IMPORT io\nIMPORT <pkg>\n\nFUNC main AS Integer\n  io::print("hi")\n  RETURN 0\nEND FUNC\n' > /tmp/szp/src/main.mfb; (cd /tmp/szp && mfb build); stat -f%z /tmp/szp/build/szp.out`
- **The trigger is a non-empty companion, not `add_imports`.** `tcp` declares
  `add_imports(vec!["net"])` (`src/codegen/builtins/tcp/mod.rs:172`) yet costs 0
  bytes, because it declares no records/enums/helpers and so renders an empty
  companion. `udp` declares the same import **plus** one record
  (`src/codegen/builtins/udp/mod.rs:143,147`) and therefore pays net's full
  115,584 bytes. `term` declares records and enums but **no** `add_imports`, and
  costs 0.
  **Consequence for plan-122-F: `term` must NOT gain `add_imports(["color"])`.** A
  native (`Body::abi_function`) member can name `color.Color` in its signature
  through a qualified type-id constant — exactly how `tcp::localAddress` returns
  `net.Address` (`src/codegen/builtins/tcp/mod.rs:103-112`, `func_local_address.rs:71`)
  — without dragging the companion in. That keeps every TUI binary at its current
  size.
- **A member may mix a native and a source overload.** `canvas::getSize` has a
  `Body::abi_function` overload taking an `Image` and a `Body::mfb` overload taking
  nothing (`src/codegen/builtins/canvas/func_get_size.rs:125-148`). This is the
  precedent F uses for `term::setForeground(color::Color)` alongside the existing
  three-`Byte` native form.
- **The whole companion lands in an importer's `.ir`.** A small `astrings`
  fixture's golden is 5,450 lines and contains every `#astrings_*` member including
  ones the fixture never calls
  (`grep -o '"name": *"[^"]*"' tests/rt-behavior/astrings/attribute-model-rt/golden/attribute_model_rt.ir | sort -u`).
  So D/E/F churn `.ir`/`.ast` goldens for **every** importer, not just the fixtures
  they edit.
- **UNVERIFIED — the byte cost of `color` itself.** It cannot be measured before
  the package exists. Phase 6 measures it and records the number; it is an input to
  C's go/no-go on the CSS table, not a blocker for A.

## 3. Design Overview

`color` is a **pure-source** package modelled on `encoding`: one `func_*.rs` per
public member carrying `INTRO`/`DESC`/`EX` prose and a `Body::mfb` body, plus
`helper_*.rs` chunks for the private `__color_*` FUNCs, assembled by
`RegistryPackage::get_mfb`.

Three pieces, layered:

1. **The record and the clamp.** `Color` plus `__color_clampByte` — a byte-for-byte
   move of `__canvas_clampByte` (`src/codegen/builtins/canvas/helper_clamp_byte.rs:13`)
   under the new name. Clamping rather than raising is a contract inherited from
   canvas and documented there: colours are computed, and a value one past an end is
   a rounding artefact, not a bug.
2. **The packed-integer bridge.** `toPacked`/`fromPacked` over `0xAARRGGBB`. This is
   not a convenience: `astrings` stores fg/bg as a packed `Integer` today, so E is
   written against these two functions rather than re-deriving shifts, and the term
   bridge's hand-rolled `__term_colorR/G/B` collapse into `color::fromPacked`.
3. **Text form.** `fromHex`/`toHex`/`toHexAlpha` and the `toString` override.

**Where correctness risk concentrates:** `fromHex`, because it is the only member
with a parse and therefore the only one with an error path. Its risk is contained —
it touches nothing outside the package — so it is scheduled early rather than last.

**Where design uncertainty concentrates:** the package-registration seams. A missed
seam does not fail loudly; `ARGUMENT_CHECKED_PACKAGES` in particular degrades
silently to "no diagnostic at all". Phase 1 therefore lands a package with **one**
trivial member and proves every seam works before any real content is written —
the cheapest experiment that could falsify the approach.

**Byte-identity is NOT this plan's gate.** plan-122 changes behavior and API by
design. A is *expected* to be byte-identical everywhere (it adds a package nobody
imports yet), and a diff in A is a bug to root-cause. From B onward the expected
diffs are named per letter. Byte-identity is a verification method here, never a
premise: a failing `.ncodesum` means *objdump one fixture, find the cause, fix it*
— never "the design is dead".

### Rejected alternatives

- **Keep `canvas::rgb`/`rgba` as forwarding aliases.** Rejected: the measured
  non-transitive-import probe shows a caller must `IMPORT color` to touch a channel
  anyway, so an alias buys no source compatibility while leaving two names for one
  concept. (User decision, 2026-09-02.)
- **Make `color` a native package.** Rejected: every member is pure arithmetic on a
  4-`Byte` record, and a pure-source package is a fraction of the work and reviewable
  as MFBASIC. The binary-size cost is real (see the table above) but it is a
  pre-existing compiler gap that affects nine other packages equally.
- **Put the colour maths in `math`.** Rejected: `math` has an empty companion and
  costs importers 0 bytes today; adding pure-source members to it would put that
  cost on every numeric program.
- **Two packages (`color` core + `color.names`).** Rejected: the user asked for one
  colour system, and MFBASIC has no sub-package mechanism.

## 4. The `Color` record

```
TYPE Color
  red AS Byte
  green AS Byte
  blue AS Byte
  alpha AS Byte
END TYPE
```

Identical in field names, order and types to today's `canvas::Color`
(`src/codegen/builtins/canvas/mod.rs:183-210`), which is what lets D be a rename
rather than a reshape. `alpha` is **straight, not premultiplied**: `0` fully
transparent, `255` fully opaque, and red/green/blue are unaffected by it. The
all-zero value is fully transparent — the property canvas's `Paint` relies on for
"an unset channel is a no-op" (`src/codegen/builtins/canvas/helper_paint_defaults.rs:15`).

## 5. Member surface (this letter)

| Member | Signature | Notes |
|---|---|---|
| `rgb` | `(red, green, blue AS Integer) AS Color` | alpha 255; components clamped |
| `rgba` | `(red, green, blue, alpha AS Integer) AS Color` | components clamped |
| `gray` | `(level AS Integer) AS Color` | `rgb(level, level, level)` |
| `withAlpha` | `(base AS Color, alpha AS Integer) AS Color` | rgb preserved, alpha clamped |
| `invert` | `(base AS Color) AS Color` | `255 - channel`; alpha preserved |
| `toPacked` | `(base AS Color) AS Integer` | `0xAARRGGBB`, always `0..0xFFFFFFFF` |
| `fromPacked` | `(value AS Integer) AS Color` | inverse; low 32 bits only |
| `fromHex` | `(text AS String) AS Color` | see §6; raises `ErrInvalidFormat` |
| `toHex` | `(base AS Color) AS String` | always `#rrggbb`, alpha dropped |
| `toHexAlpha` | `(base AS Color) AS String` | always `#rrggbbaa` |

Parameters are `Integer`, not `Byte`, for the same reason `canvas::rgba`'s are
(`src/codegen/builtins/canvas/func_rgba.rs:81-85`): declaring them `Byte` pushes an
out-of-range value into a conversion error at the call site, which is the opposite
of the clamping contract.

`toHex` and `toHexAlpha` are two members rather than one alpha-sensitive member so
the output width is a property of the call, not of the data — a caller writing a
fixed-width field never has to branch.

`toString(color::Color)` is registered with `add_override`
(`src/codegen/registry/mod.rs:1285`) and renders `#rrggbbaa` — the lossless form,
since `toString` is what a debugging `io::print` reaches for. The `net::Url`
renderer is the precedent (`registry::general_override_target`).

## 6. `fromHex` grammar

Accepted, case-insensitively, with an optional leading `#`:

| Form | Length after `#` | Expansion |
|---|---|---|
| `rgb` | 3 | each digit doubled: `f0a` → `ff00aa`, alpha 255 |
| `rgba` | 4 | each digit doubled, 4th is alpha |
| `rrggbb` | 6 | alpha 255 |
| `rrggbbaa` | 8 | as written |

Anything else — an unsupported length, a non-hex character, an empty string, a
second `#` — raises `ErrInvalidFormat` (`77050003`) with a message naming the
offending input.

Implementation: strip the optional `#` with `strings::stripPrefix`, upper-case
with `strings::upper`, walk `strings::toBytes` and map each byte through a private
`__color_hexValue` returning `-1` for a non-digit. **`encoding::hexDecode` is
deliberately not used**: it would make `color` import `encoding`, whose companion
costs an importer 429,312 bytes (measured, §2), and it does not handle the 3/4-digit
short forms or the `#`.

`toHex`/`toHexAlpha` emit **lowercase** digits, matching `encoding::hexEncode`'s
convention, so `fromHex(toHex(c))` round-trips and `toHex` output compares equal
across two programs.

## Compatibility / Format Impact

Nothing observable changes in this letter. `color` is a new package no shipped
program imports.

What A does establish, and D–F depend on:

- **`color.Color` is the package-qualified type id** — the spelling that goes in
  `resolver::BUILTIN_TYPES` and in every cross-package descriptor reference. The
  bare leaf `Color` must never appear in either place (bug-484:
  `src/resolver/mod.rs:33-40`).
- **`0xAARRGGBB` is the canonical packed order**, alpha in the high byte. E widens
  the astrings attribute payload to this layout.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work it describes; use `- [~]` for partially done and say what remains;
> mark a task moot with `- [x] ~~text~~ — moot: <evidence>`; fill each `Commit:`
> line the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — Registration skeleton (falsify the seams first)

Land a `color` package with exactly one member, and prove every registration seam
works before writing any real content. Nothing here is throwaway — the member
survives; only its siblings are missing.

- [x] Create `src/codegen/builtins/color/mod.rs`: `RegistryPackage::new("color", MODULE_INTRO, MODULE_DESC)`,
      the `Color` record via `add_record` (§4), `add_imports(vec!["color", "strings"])`
      (a package reaches its own internal members through the qualified spelling,
      as `astrings` does — `src/codegen/builtins/astrings/mod.rs:135`).
      **Correction:** `add_imports(vec!["color"])` only — Phase 1 has no `strings`
      caller, and the import set grows per phase as members need it (`bits` in
      Phase 3; `strings`/`collections` in Phase 4).
- [x] Add `pub(crate) const COLOR_TYPE: &str = "Color";` and
      `pub(crate) const COLOR_TYPE_ID: &str = "color.Color";` to that `mod.rs`,
      mirroring `term::TERM_COLOR_TYPE`/`TERM_COLOR_TYPE_ID`
      (`src/codegen/builtins/term/mod.rs:89-95`). D/E/F reference these, never a
      string literal.
- [x] Add `src/codegen/builtins/color/func_rgba.rs` and `helper_clamp_byte.rs`
      (`__color_clampByte`, the body of `canvas/helper_clamp_byte.rs:13` renamed).
- [x] Wire the seams, in this order: `pub(crate) mod color;`
      (`src/codegen/builtins/mod.rs:5-35`); `"color"` in `is_builtin_import`
      (`:50`); `"color"` in `ARGUMENT_CHECKED_PACKAGES` (`:451`); `"color"` in
      `ALL_BUILTIN_PACKAGES` (`:935`) and in the `is_builtin_import_cases` list;
      `color::register(&mut r)` in `registry::build()`
      (`src/codegen/registry/mod.rs:1937`); `builtins::color::COLOR_TYPE_ID` in
      `resolver::BUILTIN_TYPES` (`src/resolver/mod.rs:20-58`).
- [x] Add `` `color` `` to the §18 package sentence
      (`src/docs/spec/language/18_builtin-functions.md:46`) — the
      `spec_section_18_package_list_matches_is_builtin_import` test fails until it
      matches `is_builtin_import` exactly.
- [x] **Found and fixed while wiring the seam above:** `ALL_BUILTIN_PACKAGES` was a
      hand-written *copy* of the `is_builtin_import` arm and had silently omitted
      `tcp` and `udp`, so the §18 package sentence omitted them too while §18's own
      transport paragraph (`:69`) documented `IMPORT tcp`/`IMPORT udp`. A `matches!`
      cannot be enumerated, so no test could see it. Hoisted the arm to
      `builtins::BUILTIN_IMPORTS`, made `is_builtin_import` a `.contains()` over it,
      and pointed `ALL_BUILTIN_PACKAGES` at the same slice — one list, no third
      spelling to drift. See Corrections.
- [x] Tests: new `tests/rt-behavior/color/func_color_rgba_valid/` fixture with all
      **four** goldens (`build.log`, `.ast`, `.ir`, `.run` — a missing one prints
      `unexpected actual` with no `mismatch:` line and only a FULL `test-accept.sh`
      run sees it).
- [x] Prove the argument checker is live: a `tests/syntax/color/` fixture calling
      `color::rgba(1, 2)` must produce `TYPE_CALL_ARITY_MISMATCH`, **not** a bare
      `TYPE_UNKNOWN_VALUE`. If it produces the latter, the
      `ARGUMENT_CHECKED_PACKAGES` row did not take.
      Landed as TWO fixtures — see Corrections: the bug-484 bare-leaf assertion had
      to move to its own fixture (`color_type_bare_leaf_invalid`) because a resolver
      error short-circuits the type checker and would have made the arity assertions
      vacuous.

Acceptance: **MET.** `cargo test --no-fail-fast` green (exit 0, 100 test binaries,
0 failures — `/tmp/p122-ct.log`); `mfb man color` and `mfb man color rgba` both
render; `tests/syntax/color/func_color_rgba_invalid` reports
`TYPE_CALL_ARITY_MISMATCH` (x2) and `TYPE_CALL_ARGUMENT_MISMATCH`, not a bare
`TYPE_UNKNOWN_VALUE`; `./scripts/test-accept.sh` full run green at **1377 ran**
with all three new fixtures counted (`[105] rt-behavior/color/func_color_rgba_valid`,
`[791] syntax/color/color_type_bare_leaf_invalid`,
`[792] syntax/color/func_color_rgba_invalid`);
`artifact-gate.sh all` 1876 goldens, **0 diffs** — the byte-identity outcome A
predicts, since nothing imports `color` yet.
Commit: b771d7f33

### Phase 2 — Constructors

- [x] `func_rgb.rs`, `func_gray.rs`, `func_with_alpha.rs`, `func_invert.rs` — each
      a `Body::mfb` member with full `INTRO`/`DESC`/`EX` prose per `.ai/man-content.md`.
- [x] Tests: extend the rt-behavior fixture; assert clamping at both ends
      (`rgba(300, -20, 128, 255)` → `255, 0, 128, 255`) and that `invert` preserves
      alpha.
      Landed as a NEW sibling fixture `tests/rt-behavior/color/color_constructors_rt`
      rather than by growing `func_color_rgba_valid`, whose name would then have
      described a quarter of its content. Both clamp ends are asserted on every
      argument position of every constructor, so a clamp wired to the wrong
      parameter cannot pass; `invert` is asserted to preserve alpha at both colour
      extremes.

Acceptance: **MET.** `scripts/man-run-examples.sh color --run` →
`examples: 10   built: 10   ran: 10   failed: 0`; `scripts/man-census.sh --fill color`
→ 5 pages, 5/5 intro, 5/5 desc, 5/5 example, 11/11 param-desc, 4/4 types (no empty
cells); `--memory-scope color` and `--scope color` both 0.
Commit: 2bfb00d6e

### Phase 3 — Packed-integer bridge

- [x] `func_to_packed.rs`, `func_from_packed.rs` over `0xAARRGGBB`, using
      `bits::sl`/`sr`/`band`/`bor` (add `"bits"` to `add_imports`).
- [x] Tests: round-trip `fromPacked(toPacked(c)) = c` across the corners
      (all-zero, all-255, one channel at a time) and pin the byte order explicitly —
      `toPacked(rgba(0x12, 0x34, 0x56, 0x78)) = 0x78123456`.
      Landed as `tests/rt-behavior/color/color_packed_rt`. Beyond the plan's list it
      also pins the two range facts the man pages promise — `toPacked` never
      negative (`full=4294967295`), and `fromPacked` reading the **low 32 bits
      only** (`fromPacked(-1)` is white; `fromPacked(0x7FFFFFFF00000000)` is
      all-zero) — because without the mask on the alpha shift `bits::sr` is
      zero-filling over 64 bits and a high bit would leak into alpha.
      It also pins the documented `fromPacked` trap: a 24-bit `0x3366CC` unpacks
      **fully transparent** (`rgb24=51,102,204,0`).

Acceptance: **MET.** `color_packed_rt` prints `order=2014458966` and
`orderExpected=2014458966` — the expected value is the literal `0x78123456`
written into the fixture, not recomputed from the channels, so a swapped
`sl`/`sr` or a transposed channel pair cannot pass. Single-channel packs land on
the documented powers (`redOnly=16711680`, `greenOnly=65280`, `blueOnly=255`,
`alphaOnly=4278190080`), and all seven corner round-trips return the input
channel-for-channel.
Commit: 9395f9439

### Phase 4 — Hex parse and render

The one member with an error path.

- [x] `helper_hex_value.rs` (`__color_hexValue(byte) AS Integer`, `-1` on a
      non-digit) and `helper_hex_pair.rs` if the digit-doubling is worth its own FUNC.
      Landed as `helper_hex_value.rs` (parse side) and `helper_hex_byte.rs`
      (`__color_hexByte` + `__color_hexDigit`, render side). The digit-doubling did
      **not** earn its own FUNC — it is `d * 17`, written inline at the two
      `fromHex` arms that need it.
- [x] `func_from_hex.rs` implementing §6, raising
      `FAIL error(77050003, "…")` on every rejected input.
- [x] `func_to_hex.rs`, `func_to_hex_alpha.rs` — lowercase output.
- [x] Tests: a `tests/rt-behavior/color/` fixture covering **all four accepted
      lengths, both with and without `#`, in mixed case** (8 accepting cases), plus
      a TRAP-ing fixture for each rejection class: bad length (5), non-hex char,
      empty string, doubled `#`. Round-trip `fromHex(toHexAlpha(c)) = c` over the
      corner colours.
      Landed as `tests/rt-behavior/color/color_hex_rt` — one fixture, since the
      rejection cases TRAP and return rather than ending the program, so they
      compose with the accepting ones. **10** rejection cases, not the plan's 4:
      four bad lengths (2, 5, 7, 9 — bracketing every accepted length on both
      sides, not just one), empty, bare `#`, doubled `#`, and three non-hex
      characters including an embedded space.

Acceptance: **MET.** All 8 accepted forms yield the documented channels
(`h3`/`b3` = `255,0,170,255` — digit doubling; `h4` alpha `136` = `0x88`;
mixed case agrees with lower). A missing alpha is **opaque**
(`shortOpaque`/`longOpaque` = `...,255`), the documented contrast with
`fromPacked`. All 10 rejections print `invalidFormat TRUE` — the raised code
compared against `errorCode::ErrInvalidFormat`, not the absence of output, so a
crash could not pass as a rejection. Round trips exact over all four corners
including transparent (`rtBlack=#00000000`) and low channels
(`rtLowChannels=#01020304`, proving the zero-padding). Render is lowercase and
fixed-width (`case=#abcdef`, `pad=#01020304`).
Commit: 3605035ce

### Phase 5 — `toString` override

- [x] Register a `toString` override for `color.Color` via `add_override`
      (`src/codegen/registry/mod.rs:1285`), rendering `#rrggbbaa`, modelled on the
      `net::Url` renderer reached through `registry::general_override_target`.
      Target is the private `__color_toString` helper (`helper_to_string.rs`),
      named by `COLOR_TO_STRING` — the same private-helper-plus-`add_override`
      shape `net` uses for `__net_urlToString`. It delegates to
      `__color_hexByte` rather than restating the digit rendering, so `toString`
      and `toHexAlpha` are the same bytes by construction.
- [x] Tests: `io::print(toString(color::rgba(255, 0, 0, 128)))` prints `#ff000080`.

Acceptance: **MET.** `tests/rt-behavior/color/color_to_string_rt`'s golden shows
`#ff000080` as its first line, plus the corners
(`#00000000`/`#ffffffff`/`#01020304`), `agreesWithToHexAlpha=TRUE` and
`differsFromToHex=TRUE` — so the override is pinned to the lossless form, not
merely to "some string".

Partner assertion landed as `tests/syntax/color/color_to_string_unrelated_record_invalid`
(a compile error, so it cannot live in the rt fixture): `toString` still refuses
an unrelated record with `TYPE_CALL_ARGUMENT_MISMATCH`, and the rendered
"expected" list still names only the scalars plus `List OF Byte` — the override
did not widen the builtin. It declares **two** records, one of them (`Shade`)
field-for-field identical in shape to `color::Color`, so the override cannot be
passing by matching on record shape rather than on the type's identity. The
rt fixture also re-pins `toString` over `Integer`/`Boolean`/`String`.
Commit: 3eebf406d

### Phase 6 — Documentation, census and the cost measurement

- [x] `MODULE_INTRO`/`MODULE_DESC` on the package written to the `.ai/man-content.md`
      standard: what a colour is here, the straight-alpha rule, the clamping rule,
      the `0xAARRGGBB` packed order, and that `Color` is an ordinary value record a
      program may build and `WITH`-update. (Landed in Phase 1; all five points are
      in `MODULE_DESC`.)
- [x] `scripts/man-census.sh --fill color` → 100% fill on every column.
- [x] `scripts/man-census.sh --memory-scope color` and `--scope color` → **0**
      unclassified hits. No `copy`/`free`/`heap`/`borrow` vocabulary; the permitted
      words are copy, mutate, value, alias.
- [x] Add a `color` row to `src/docs/spec/stdlib/` as `18_color.md`, and its entry
      in `src/docs/spec/stdlib/spec.md`.
- [x] **Measure and record the package's unused-import byte cost** with the
      Verified-properties command from §2, and write the number into this file's
      Corrections section. This is C's input.
- [x] **Added task — the Validation Plan's doc-sync of `.ai/resources-packages.md`.**
      Corrected the two stale bullets in §"New builtin-package registration seams"
      (`src/builtins/<pkg>.rs`, `descriptor.rs` — both verified gone) and added the
      measured companion-cost table with its command and the
      "trigger is a non-empty companion, not `add_imports`" finding. Also recorded
      the `BUILTIN_IMPORTS` single-source-of-truth rule so the `tcp`/`udp` drift
      cannot be reintroduced.
- [x] **Added task — coverage denominator check.** `scripts/coverage-common.sh`'s
      `IGNORE` is
      `(^|/)(target|tests)/|_runtime_tables\.rs$|/code/private/unicode\.rs$|/src/testutil\.rs$`,
      which does not match `src/codegen/builtins/color/`, so the new module is in
      the denominator rather than silently unmeasured
      (`codegen-cover-fixture-may-not-cover-your-member`).

Acceptance: **MET.** `man-census.sh --fill color` → 10 pages, 10/10 intro,
10/10 desc, 10/10 example, **16/16** param-desc, 4/4 types, `pages with neither
Description nor Examples: 0`; `--memory-scope color` → `unclassified
memory-vocabulary hits: 0`; `--scope color` → `internals-vocabulary hits: 0`;
`man-run-examples.sh color --run` → 22 examples, 22 built, 22 ran, 0 failed. The
measured byte cost is in Corrections.
Commit: b1526d005

### Phase 7 — Out-of-band: the `{owner}::{type_name}` doubled qualifier

Not colour work, but plan-122 makes this diagnostic fire at every consumer during
D–F, so it is fixed before those letters make its goldens harder to read.

- [x] ~~`src/ir/shape.rs:1868` — render the **leaf**, not the qualified name, so the
      message reads ``belongs to `net::Address` `` instead of
      ``belongs to `net::net.Address` ``.~~ — **moot: already fixed out-of-band**,
      as §2 anticipated ("Fixed out-of-band before this plan starts"). The strip is
      in `check_foreign_record_field` at `src/ir/shape.rs` (`grep -n "Imports are
      not transitive" src/ir/shape.rs` → 1894), carrying a comment naming this
      exact defect: *"The rendered name is already package-qualified (`net.Address`),
      so interpolating it after `{owner}::` spelled the type `net::net.Address` — a
      doubled qualifier no program can write."*
- [x] ~~Regenerate the 5 goldens that pin the old text.~~ — **moot: already
      correct.** `grep -h 'Imports are not transitive' tests/syntax/tcp/*/golden/build.log`
      renders exactly one distinct message, and it reads ``belongs to `net::Address` ``.
      Nothing to regenerate.

Acceptance: **MET (by prior work, verified not assumed).** Every golden pinning
the text shows `net::Address`; the diagnostic still fires
(`local-address-field-without-net-import/golden/build.log` ends `[exit 1]`) and
its rule code is unchanged (`2-203-0043 TYPE_UNKNOWN_VALUE`).

**Count correction:** the plan says 5 goldens; there are **4**
(`grep -rln 'Imports are not transitive' tests/ | wc -l` → 4, all under
`tests/syntax/tcp/`).
Commit: — (no code change; nothing to commit for this phase)

## Validation Plan

- **Tests:** `tests/rt-behavior/color/**` (positive behavior, four goldens each),
  `tests/syntax/color/**` (arity/type diagnostics and every `fromHex` rejection
  class). Negative cases assert the raised error **code**, never merely absence of
  output.
- **Coverage check:** `scripts/coverage.sh --bin mfb` — confirm
  `src/codegen/builtins/color/` is in the denominator. A green suite that never
  reached the new module proves nothing (`codegen-cover-fixture-may-not-cover-your-member`).
- **Runtime proof:** a scratch project that does
  `io::print(color::toHexAlpha(color::fromHex("#f0a")))` → `#ff00aaff`, built and
  run, not just compiled.
- **Doc sync:** §18 package sentence; new `src/docs/spec/stdlib/18_color.md` +
  `spec.md` index; `.ai/resources-packages.md` §"New builtin-package registration
  seams" — correct the two stale bullets (`src/builtins/<pkg>.rs`, `descriptor.rs`)
  found in §2, and add the measured "a pure-source companion is compiled into every
  importer" finding with its numbers.
- **Acceptance:** `cargo test --no-fail-fast` (never fail-fast — a failing
  `golden.rs` skips every later `rt_*`), `./scripts/test-accept.sh` **full run**
  (watch the `N ran` count), `scripts/artifact-gate.sh`, and
  `cargo check --all-targets` at the **end** as well as the start (only
  `--all-targets` sees test-target warnings).
- **Formatting:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **`color::gray` vs `color::grey`.** Recommend `gray` only, matching the CSS
  keyword `gray` that C's table will carry as canonical. (§5)
- **Whether `toPacked` should have an RGB-only sibling.** Recommend not: E needs
  `0xAARRGGBB` and a caller wanting 24 bits writes `bits::band(toPacked(c), 16777215)`.
  Revisit if E's astrings body ends up needing it twice. (§5)

## Corrections

**Prerequisites row 2 was stale in the passing direction.** The plan recorded
"NOT MET — 5 modified `examples/browser/**/*.mfb` files". At execution time
`git status --porcelain` on `main` is empty; those edits landed between authoring
and execution. All four rows re-measured MET (2026-09-03) before Phase 1 started.

**Phase 1 — `add_imports` narrowed to `["color"]`.** The plan specified
`vec!["color", "strings"]`. Phase 1 has no `strings` caller, so the import would
have been dead until Phase 4. The set now grows per phase with the members that
need it: `bits` in Phase 3, `strings` + `collections` in Phase 4. No behavioral
difference — `strings` has an empty companion and costs an importer 0 bytes
(plan §2 measurement table) — but an injected `IMPORT` nothing calls is noise in
every importer's `.ir`.

**Phase 1 — the argument-checker fixture had to be split in two.** The plan asks
for one `tests/syntax/color/` fixture proving `TYPE_CALL_ARITY_MISMATCH` fires.
Written as one file that *also* carried the bug-484 bare-leaf `AS Color` line, the
generated golden contained **only** `SYMBOL_UNKNOWN_TYPE`: a resolver error
short-circuits the type checker, so none of the call diagnostics were ever
reached. That fixture would have passed while proving nothing about
`ARGUMENT_CHECKED_PACKAGES` — precisely the silent-seam failure Phase 1 exists to
catch. Split into:

- `tests/syntax/color/func_color_rgba_invalid` — arity and argument-type
  diagnostics (`TYPE_CALL_ARITY_MISMATCH` x2, `TYPE_CALL_ARGUMENT_MISMATCH`).
- `tests/syntax/color/color_type_bare_leaf_invalid` — the bug-484 pin, alone.

Its partner assertion (the qualified `AS color::Color` *does* resolve) is
`rt-behavior/color/func_color_rgba_valid`, so the pair pins that exactly one of
the two spellings works.

**Bug found and fixed: the §18 package sentence omitted `tcp` and `udp`.**
Not colour work; found while wiring the §18 seam, and fixed here rather than
deferred. `ALL_BUILTIN_PACKAGES` (the test-side list `spec_section_18_package_
list_matches_is_builtin_import` compares §18 against) was a hand-written **copy**
of the `is_builtin_import` `matches!` arm, and had drifted: it omitted `tcp` and
`udp` for the whole of plan-110's lifetime. Because a `matches!` cannot be
enumerated, the guard could only compare the spec against the copy — so the
omission propagated into the spec and no test could see it. Meanwhile §18:69
documented `IMPORT tcp` and `IMPORT udp` two dozen lines further down, so the
document contradicted itself.

Measured: `is_builtin_import` accepted 28 names; `ALL_BUILTIN_PACKAGES` listed 26;
the §18 sentence listed the same 26.

Fix: hoisted the arm to `pub(crate) const BUILTIN_IMPORTS` in
`src/codegen/builtins/mod.rs`, made `is_builtin_import` a `.contains()` over it,
and redefined `ALL_BUILTIN_PACKAGES` as that same slice. There is now one list, so
the copy that drifted cannot exist. §18's sentence now carries all 29 names
(the 28 plus `color`). This is a corrected line backed by §18's own `:69`
paragraph and by `is_builtin_import`, not a re-baseline.

**§2's seam table line numbers moved.** The table cites
`ARGUMENT_CHECKED_PACKAGES` at `src/codegen/builtins/mod.rs:451` and
`ALL_BUILTIN_PACKAGES` at `:935`; measured at execution they were `:511` and
`:1072`, and after the `BUILTIN_IMPORTS` hoist above they are `:530` and `:1086`
(`grep -n "ARGUMENT_CHECKED_PACKAGES\|ALL_BUILTIN_PACKAGES" src/codegen/builtins/mod.rs`).
Cite the symbol and its grep, not the line — see
`plan-line-citations-decay-silently`.

**Phase 2 — a man example cannot use a member from a later phase.** Four Phase-2
examples were first written with `io::print(toString(c))`, which does not compile
until the Phase-5 `toString` override exists (`TYPE_CALL_ARGUMENT_MISMATCH`:
`toString` has argument type(s) (color.Color)`). Phase 2's acceptance criterion
requires `man-run-examples.sh color --run` green, so the criterion cannot be met
by a page that forward-references its own package. Rewritten to print channels
explicitly, which is also the better teaching example for a constructor page.
**General rule for the rest of this plan: an `EX` block may only use members that
already exist at the phase that lands it.** Weakening the criterion to "green
except forward references" was rejected — the whole value of the example gate is
that every published example runs.

> **RESOLVED, and the doc withdrawn (2026-09-03, after merging main).** The flake
> below was a symptom of the **real** bug-498 — `thread::send`/`emit`/`transfer`
> repointing the arena register at the *destination* thread's arena and allocating
> there unlocked, corrupting the free list — fixed on main in `f2bf55a86` and
> archived as `bugs/completed/bug-498-thread-send-cross-thread-arena-race.md`. That
> doc's own proof covers bidirectional transfers, which is exactly this fixture.
>
> Two independent reasons to withdraw my write-up rather than renumber it: the
> number **collided** (a peer session filed the real bug-498 concurrently — bug
> numbers race between sessions exactly as plan and rule-code numbers do, and `ls
> bugs/` under-reported because theirs was already archived), and after merging main
> the symptom is gone — **96 concurrent runs of the fixture, 0 failures**, against a
> defect that previously needed load to appear. A duplicate doc describing a fixed
> defect under a taken number is worse than no doc.
>
> The transferable lesson is kept here rather than in a bug file: *a `Resource
> handle is already closed` under load, in a fixture that is 10/10 green alone, can
> be free-list corruption in the arena rather than a lifetime error in the code that
> reports it.*

**Bug found and filed while running Phase 2's gates: bug-498.**
`rt-behavior/threads/thread-transfer-bidirectional-rt` fails intermittently under
a **full** acceptance run with `7-703-0004 Resource handle is already closed`
(exit 255) instead of printing its two sizes. Not attributable to plan-122:
`artifact-gate.sh all` reports `1878 golden(s) checked, 0 diff(s)`, so this change
is byte-neutral for every program that does not import `color`, and this fixture
does not; and the same fixture was green in the preceding full run. Measured:
10/10 pass when run alone, 1 failure in 3 full parallel runs — so the trigger is
load, and a filtered re-run can never show it. Written up with the evidence and
the cheapest lines of enquiry in
`bugs/bug-498-thread-transfer-bidirectional-flaky-under-load.md` rather than left
as an unexplained red in a log.

**Phase 4 — §6's `strings::upper` pass is unnecessary.** §6 specifies "strip the
optional `#` with `strings::stripPrefix`, **upper-case with `strings::upper`**,
walk `strings::toBytes`". `__color_hexValue` already accepts both cases (it is
`encoding`'s three-range shape: `0-9`, `a-f`, `A-F`), so the `upper` pass would be
a Unicode full-case mapping over the input for no effect. Dropped. Case
insensitivity is proved by the fixture's `b3`/`b4`/`b6`/`b8` mixed-case rows
agreeing with their lowercase equivalents.

**Phase 4 — `__color_hexDigit` renders from a table, not from arithmetic.** The
first draft was `strings::fromScalars([48 + value])`, which does not compile:
`fromScalars` takes a `List OF Scalar` and the addition yields `Integer`
(`2-203-0051 TYPE_LIST_ELEMENT_MISMATCH: List element has type Integer, expected
Scalar`). Rewritten as `strings::mid("0123456789abcdef", value, 1)` — same answer,
no per-digit conversion, and the table itself states that the digits are
lowercase.

**Phase 4 — a `Body`/`EX` containing `"#` needs `r##"…"##`.** `color` is the first
package whose MFBASIC bodies contain the literal `"#"` (every hex render starts
with it) and whose examples contain `"#f0a"`. In a `r#"…"#` raw string that
two-character sequence closes the literal mid-expression, and the resulting
rustc error points at the *following* prose rather than at the `"#`. Affected
constants now use `r##"…"##`. Worth knowing for plan-122-C, whose named-colour
examples will hit the same thing.

**Phase 6 — MEASURED byte cost of the `color` companion. This is plan-122-C's
go/no-go input; read it before starting C.**

Measured 2026-09-03 on macos-aarch64 with the §2 command (`mfb init /tmp/szp`,
`IMPORT io` + `IMPORT <pkg>` with **no call** to `<pkg>`, `stat -f%z` the built
`.out`):

| build | bytes | delta over baseline |
|---|---|---|
| `IMPORT io` alone (baseline) | 66,596 | — |
| `IMPORT io` + `IMPORT color` | 99,620 | **+33,024** |
| `IMPORT io` + `IMPORT strings` | 66,596 | 0 (control — empty companion, as §2 predicts) |
| `IMPORT io` + `IMPORT encoding` | 528,932 | +462,336 |
| `IMPORT io` + `IMPORT astrings` | 1,156,388 | +1,089,792 |

**`color`'s whole companion — record, 4 helpers, 10 members — costs an importer
33,024 bytes.** That is an order of magnitude below every other pure-source
package measured, and it is the denominator C's CSS name table is judged against:
C's Phase 2 go/no-go asks whether the table is "a minority of the package's cost",
so the table must be compared against **33,024**, not against a whole-binary
figure.

> **PRECISION CORRECTION (added while running plan-122-C).** `stat -f%z` on a
> built `.out` is **quantised to 16,512-byte blocks** — proved by adding 20 extra
> `io::print` statements to the probe and watching the size not move at all, then
> sweeping to 4000. Every delta in the table above, and every one in plan-122-A §2,
> is a multiple of 16,512. So `+33,024` is *2 blocks*, true to ±16,512, not to the
> byte. The conclusion is unaffected — 2 blocks against `encoding`'s 28 is still an
> order of magnitude — but do not quote these as exact figures. See plan-122-C's
> Corrections for the evidence.

Two notes for whoever runs C:

- The baseline reproduced the plan's recorded 66,596 exactly, so the measurement
  is comparable to §2's table.
- §2's `encoding` and `astrings` rows have **drifted** since the plan was
  authored (recorded +429,312 and +1,073,280; measured +462,336 and +1,089,792).
  Those packages grew; nothing here caused it. Re-measure rather than quoting
  §2's numbers.

## Final acceptance (2026-09-03)

Every phase is landed and every box resolved. Whole-plan gates, run on the
`worktree-P-122` branch:

| Gate | Result |
|---|---|
| `cargo test --no-fail-fast` | **exit 0** — 100 test binaries, 0 failures |
| `./scripts/test-accept.sh` (full) | **1382 ran, passed** — all 8 `color` fixtures counted (5 `rt-behavior/color/*`, 3 `syntax/color/*`) |
| `scripts/artifact-gate.sh <mfb> all` | **1360 tests, 1523 builds, 1884 goldens, 0 diffs** |
| `cargo check --all-targets` | clean, no warnings |
| `scripts/man-run-examples.sh color --run` | 22 examples, 22 built, 22 ran, **0 failed** |
| `scripts/man-census.sh --fill color` | 10 pages, 100% every column, 16/16 param-desc, 4/4 types |
| `scripts/man-census.sh --memory-scope color` / `--scope color` | **0** / **0** |
| `rustup run 1.96.0 cargo fmt --all` | applied |

The **0 diffs** is the outcome this letter predicts: A adds a package nothing
imports yet, so it must be byte-identical everywhere. The only golden churn in the
whole letter is the `.ir` of `color`'s own fixtures, growing as the companion grew
— the "whole companion lands in an importer's `.ir`" property from §2.

### Whole-plan gates after merging main (2026-09-03)

`main` advanced **50 commits** while A/B/C were being written, so it was merged
into `worktree-P-122` and every gate re-run, per the follow-plan procedure. The
merge was **clean** (no conflicts).

| Gate | Result |
|---|---|
| `./scripts/test-accept.sh` (full, post-merge) | **1386 ran, passed, 0 mismatches** — all 11 `color` fixtures counted |
| `scripts/artifact-gate.sh <mfb> all` (post-merge) | 1364 tests, 1527 builds, **1890 goldens, 0 diffs** |
| `cargo test --no-fail-fast` (post-merge) | **108 test binaries green, 1 failed** — `rt_tls_connect_allow_self_signed`, the known `bug-488` flake; 4/4 green in isolation, twice |
| `cargo check --all-targets` (post-merge) | clean |
| `cargo fmt --all` + `repository/` pass | no churn |
| `mfb man color` | all **28** members render |

Two things the merge turned up, both handled rather than absorbed:

- The duplicate **bug-498** (see the withdrawal note above).
- Two failures in `rt_tls_connect_allow_self_signed`, which is **already-filed
  `bug-488`** — a known test-isolation flake where the file releases an ephemeral
  port for `openssl s_server` to take and loses the race when another `cargo test`
  shares the machine. A peer worktree session was verifiably running its own suite
  both times, and the file passes **4/4 in isolation, twice**. Not mine: nothing in
  plan-122 touches TLS, and `artifact-gate.sh all` reported 0 diffs on the same
  tree.

  The pre-merge one matched bug-488's report exactly. The post-merge one did
  **not** — `still_rejects_an_expired_certificate` failed with *"the certificate
  meant to be expired is still valid"*, which is the file's own setup guard
  refusing to assert vacuously, not a verdict flip. That is a symptom the port
  race does not explain: the cases also share the generated certificate identity
  on disk. Appended to `bug-488` as a third sighting, with the consequence that
  making `port_gate()` cross-process is necessary but **not sufficient**.

## Summary

The engineering risk in A is not the arithmetic — it is the seven registration
seams, of which `ARGUMENT_CHECKED_PACKAGES` fails silently and `BUILTIN_TYPES`
fails only for a bare-leaf spelling. Phase 1 exists to make all seven fail loudly
before any content is written.

That framing held. The arithmetic cost nothing; the seams produced the letter's
one real finding — that the census list guarding §18 was a *copy* of the predicate
rather than the predicate, and had silently lost `tcp` and `udp`.

Untouched by this letter: canvas, term, astrings, every existing golden, and the
whole question of colour-space maths.
