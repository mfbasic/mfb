# bug-337: man pages that contradict the implementation — a documented function that does not exist, a wrong error code, and 12 more

Last updated: 2026-07-18
Effort: medium (1h–2h)
Severity: LOW
Class: Other (documentation)

Status: Open
Regression Test: per-item; the `mfb build` repros in D3/D8/D9 belong in
`tests/acceptance/` as compile-success/compile-failure fixtures

A cluster of *content* defects in the `mfb man` corpus: places where a page states
something the compiler does not do. These are worse than a missing page. A reader who
consults `mfb man` and writes code from it gets a compile error (D3, D8), catches the
wrong error code (D2), or never learns that the overload they need exists (D4, D10).
Several pages carry a `[[provenance]]` citation pointing at the very symbol that
contradicts them, which means the citation was added without being read.

The single correct behavior a fix produces: **every claim on a man page is true of the
compiler at the cited symbol, and every function `mfb man` lists is callable.**

Items are ordered by user impact, not by discovery order. D1–D3 are things a reader can
act on and be wrong; D4–D9 are things a reader will be misled by; D10–D14 are cosmetic
or single-line.

References:

- `.ai/man_template.md`, `.ai/man_package_template.md`; `AGENTS.md:53-60`
- `src/docs/spec/stdlib/13_money.md:112` — the spec already documents the `Money`
  behavior D1's man pages deny.
- `src/docs/spec/diagnostics/02_error-codes.md:64,94` — the canonical error registry
  D2 contradicts.
- Structural causes (no citation checker, no `.md` guard coverage, three packages
  invisible to the generator) are filed separately as bug-336. Neither blocks the other.
- Found during the cleanup review (Agent 20 man sweep; Agent 17 builtins; Agent 07
  tls/crypto/link), base `25c38ba1`.

## Current State

Every item below was confirmed on both sides: the page line and the implementation line
were each read, and where a repro is shown it was executed against
`target/debug/mfb` in this worktree.

## Items

### D1 — `math::` accepts `Money` on eight functions; not one math page mentions it, and three assert the negation

The `Money` overloads landed with plan-29-G §4.7 and the spec documents them. The man
pages were never updated, and three of them state the opposite.

| The page says | The code does |
| --- | --- |
| `src/docs/man/builtins/math/round.txt:71-73` — "value must be a single Float or Fixed. Passing any other type, such as an Integer, String, Boolean, Byte, record, union, resource, thread, function, or lambda, is a compile-time type error." | `src/builtins/math.rs:172-174` — `FLOOR \| CEIL \| ROUND if one_money(arg_types) => Cow::Borrowed("Integer")`. A `Money` argument compiles and returns the dimensionless whole-unit `Integer`. |
| `src/docs/man/builtins/math/package.md:31-33` — "`abs` accepts `Integer`, `Float`, or `Fixed`; the rounding and transcendental functions accept `Float` or `Fixed`; `rand` and `seed` work on `Integer`." `[[src/builtins/math.rs:expected_arguments]]` | `src/builtins/math.rs:193-194,199` (`expected_arguments`, the cited symbol) — `ABS => "Integer \| Float \| Fixed \| Money"`, `FLOOR \| CEIL \| ROUND => "Float \| Fixed \| Money"`, `RAND => "Integer min, Integer max (or Money, Money)"`. |
| `src/docs/man/builtins/math/min.txt` PARAMETERS — "`a AS Integer \| Float \| Fixed`" | `src/builtins/math.rs:145` → `all_same_numeric` → `is_numeric` at `:256`, which is `matches!(type_name, "Integer" \| "Float" \| "Fixed" \| "Money")`. |

- Full implementation set (`src/builtins/math.rs`): `abs` `:144`, `min`/`max` `:145`,
  `clamp` `:169` — all accept `Money` and *return* `Money`; `floor`/`ceil`/`round`
  `:172-174` accept `Money` and return `Integer` (a deliberate dimension exit);
  `rand(Money, Money)` `:182-184` returns `Money`.
- `grep -c Money src/docs/man/builtins/math/*` is **0** across the entire directory —
  all 35 legacy pages and `package.md`.
- `floor.txt:71`, `ceil.txt:75`, `round.txt:72` each carry the "must be a single Float or
  Fixed" TYPE CHECKING paragraph.
- Man and spec actively disagree: `src/docs/spec/stdlib/13_money.md:110-112` describes
  `math::round(Money)` as the documented way to exit the currency dimension.
- Related but already filed: bug-300 E6/E7 cover the `call_return_type_name` table
  inconsistency and the same `round`/`floor`/`ceil` man omission. This item is the full
  eight-function set including the negating prose, which E7 does not cover.
- Fix: add the `Money` rows/overloads to `abs`, `min`, `max`, `clamp`, `floor`, `ceil`,
  `round`, `rand`, and to `package.md:31-33`; delete the three negating TYPE CHECKING
  paragraphs.

### D2 — `tls` pages print `77060001` for `ErrTimeout`; that code is `ErrWrapped`, and the real timeout code is `77050008`

Highest-impact single defect in the corpus: a reader writing
`TRAP e WHERE e.code = 77060001` to catch an accept timeout catches nothing, and would
instead catch a generic wrapped error from anywhere.

| The page says | The code does |
| --- | --- |
| `src/docs/man/builtins/tls/accept.md:80` — `\| 77060001 \| ErrTimeout \| timeoutMs is positive and no connection arrived …` | `src/target/shared/code/error_constants.rs:143` — `pub(crate) const ERR_TIMEOUT_CODE: &str = "77050008";` |
| `src/docs/man/builtins/tls/package.md:94` — `\| 77060001 \| ErrTimeout \| raised by accept when a positive timeoutMs elapses … [[src/target/shared/code/error_constants.rs:ERR_TIMEOUT_CODE]]` | `src/docs/spec/diagnostics/02_error-codes.md:94` — `\| 7-706-0001 \| 77060001 \| ErrWrapped \| Generic wrapper code for adding context while preserving the underlying message.` |

- `package.md:94` is the sharp case: the invisible provenance citation names
  `ERR_TIMEOUT_CODE` — the correct constant — and the visible number beside it is a
  different error entirely. The citation resolves (the symbol exists), so bug-336's
  proposed symbol-existence checker would *not* catch this; a value-comparing check
  would. Worth noting when designing that gate.
- `77060001` appears in exactly these two man pages and nowhere else in `src/docs/man/`.
- Fix: `77060001` → `77050008` in both pages.

### D3 — five `encoding` example programs do not compile

Every `encoding` page with a length-printing example calls `collections::len`, which is
not a `collections` member.

| The page says | The code does |
| --- | --- |
| `src/docs/man/builtins/encoding/utf32Encode.md:69` (and `utf8Encode.md:90`, `utf8EncodeBytes.md:69`, `utf8EncodeInts.md:70`, `utf16Encode.md:70`) — `io::print(toString(collections::len(points)))` | `src/builtins/collections.rs:20-39` (`FUNCTIONS`, 19 names) + `:47-68` (`NATIVE_MEMBERS`, 20 names) — 39 members, none of them `len`. `len` is a `general` builtin, `src/builtins/general.rs:4`. |

Repro (the example verbatim, plus the `IMPORT io`/`IMPORT collections` lines the example
also omits):

```
$ cat src/main.mfb
IMPORT encoding
IMPORT io
IMPORT collections

SUB main
  LET points AS List OF Integer = encoding::utf32Encode("hello")
  io::print(toString(collections::len(points)))
END SUB

$ mfb build .
./src/main.mfb:7 error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: identifier could not be resolved
               Built-in package `collections` does not export `collections.len`.
```

- Observed: two errors before the imports are added (`SYMBOL_UNKNOWN_IMPORT` for both
  `io` and `collections`), then `SYMBOL_UNKNOWN_IDENTIFIER`.
- Expected: `len(points)` — the bare `general` call, no import needed.
- Contrast: a scan of all 485 pages found no other invalid call. This is a single
  copy-pasted mistake replicated five times, not a systemic example problem.
- Fix: `collections::len(x)` → `len(x)` in all five; drop the now-unneeded
  `IMPORT collections`.

### D4 — `net::toAddress` has a full man page, is listed as a real function, and does not exist

| The page says | The code does |
| --- | --- |
| `src/docs/man/builtins/net/toAddress.txt:1-19` — `NAME / toAddress - project a Url onto a connectable Address`, `SYNOPSIS / net::toAddress(url AS Url) AS Address`, a nine-line DESCRIPTION, an ERRORS section, and an EXAMPLES block | `grep -rn toAddress src/ --include='*.rs' --include='*.mfb'` returns **zero hits**. There is no implementation anywhere in the compiler. |

- The page itself admits it at `:21-23` — "Not yet provided in this version: connect
  directly with `net::connectTcp(url.host, url.port)` … A follow-up adds the native shim"
  — buried after nineteen lines that describe it as real, and its own EXAMPLES block at
  `:32-33` then works around it in a comment.
- `mfb man net` lists it as a function regardless:

  ```
  $ mfb man net | sed -n '152p'
    toAddress          project a Url onto a connectable Address
  ```

- Three other pages assert it flatly with no caveat:
  `src/docs/man/builtins/net/package.md:45` ("`net::toAddress` projects a `Url` onto a
  connectable `Address`, filling the scheme's default port"), `:72` and `:73` (two rows
  of the package Errors table attribute `ErrInvalidFormat` and `ErrUnsupported` to
  "`toUrl` and `toAddress`"), and `src/docs/man/builtins/net/toUrl.txt:30,52`.
- Fix: `git rm` the page and strike the four cross-references, or implement it. Deleting
  the page is the correct default — a man page is not a roadmap. If it is kept as a
  planned-feature note, it must not be a function page in `net/` (see bug-336 S8: there
  is no topic-page mechanism, which is why it ended up here).

### D5 — `tls` pages document `timeoutMs` as advisory and ignored; both backends implement it and raise the `ErrTimeout` the pages omit

| The page says | The code does |
| --- | --- |
| `src/docs/man/builtins/tls/connect.md:43` — "The optional `timeoutMs` is accepted for forward compatibility but is advisory" | `src/target/shared/code/tls/openssl.rs:153-155` — "Connect the socket, bounded by `timeoutMs` when > 0 (non-blocking connect + poll)"; `:217-241` polls with the stored timeout; `:709-737` is the `connect_timeout` label, which calls `emit_fail(… ERR_TIMEOUT_CODE, ERR_TIMEOUT_SYMBOL …)`. |
| `connect.md:79` — "`timeoutMs` … Advisory on the current backend and **currently ignored**; defaults to `0` when omitted." | `src/target/shared/code/tls/macos.rs:855-876` computes a `dispatch_time` deadline from the stored `timeoutMs`; `:968-990` is `conn_timeout`, which emits `ERR_TIMEOUT_CODE`. The handshake path additionally bounds itself (`openssl.rs:312-336` via `SO_RCVTIMEO`/`SO_SNDTIMEO`; `macos.rs:3220,3279-3308`). |
| `src/docs/man/builtins/tls/package.md:66` — "`timeoutMs` argument to `tls::connect` is `Integer` milliseconds but is advisory" | Both backends. |
| `connect.md` Errors table lists `ErrOutOfMemory`, `ErrAddressNotFound`, `ErrNetworkFailed`, `ErrTlsFailed` — **no `ErrTimeout` row** | `ErrTimeout` is reachable on both platforms. |

- The page is doubly wrong: it tells the reader the parameter does nothing, and then
  omits from the Errors table the one error that only exists because the parameter does
  something. A `TRAP` written from this page has no arm for the timeout.
- Fix: rewrite `connect.md:43,79` and `package.md:66` to describe the implemented
  behavior (DNS resolution is explicitly *not* bounded — `openssl.rs:155`), and add the
  `77050008 / ErrTimeout` row to `connect.md`'s Errors table.

### D6 — macOS TLS cannot produce the `ErrAddressNotFound` / `ErrAddressInvalid` the pages promise

| The page says | The code does |
| --- | --- |
| `src/docs/man/builtins/tls/connect.md:93` — `\| 77070002 \| ErrAddressNotFound \| host could not be resolved, including when it is malformed or has no address record.` | `ERR_ADDRESS_NOT_FOUND_CODE` appears only in `src/target/shared/code/tls/openssl.rs:745`. Zero occurrences in `tls/macos.rs`. |
| `src/docs/man/builtins/tls/listen.md:92` — `\| 77070001 \| ErrAddressInvalid \| host/port could not be resolved to a bindable local endpoint.` | `ERR_ADDRESS_INVALID_CODE` appears only in `openssl.rs:1295`. Zero occurrences in `tls/macos.rs`. |
| `tls/package.md:95-96` — both rows repeated, each with a resolving `[[error_constants.rs:ERR_ADDRESS_*_CODE]]` citation | `src/target/shared/code/tls/macos.rs:942-966` — the `conn_fail` label cancels the connection and emits `ERR_TLS_FAILED_CODE`. Every connection-establishment failure on macOS, including an unresolvable host, collapses into `ErrTlsFailed`. |

- A cross-platform program that branches on `ErrAddressNotFound` works on Linux and
  silently takes the wrong branch on macOS. The pages state the codes unconditionally,
  with no platform note.
- The underlying divergence is a real (if minor) behavioral inconsistency; the
  documentation fix is to state the matrix. Whether macOS should distinguish the cases
  is out of scope here.
- Fix: add a per-platform note to the two Errors tables and to `package.md`.

### D7 — `link/package.md`: ~83 lines of prose render as a code block, a 30-line section is duplicated verbatim, and 11 diagnostics are missing

Three independent defects in one page, all visible in `mfb man link`.

**Broken fence.** Fences open/close at lines 7/9, 26/42, then 60 and 144. Lines 61–143
are therefore inside a code block. Observed:

```
$ mfb man link | sed -n '59,64p'
  package and wins before import lookup for that root name.
  [[src/resolver/mod.rs:collect_top_level_symbols]]
  [[src/resolver/resolution.rs:resolve_package_qualified_name]]
  
  ## Binding packages
  
```

Raw `##` headings, raw backticks, and — because the renderer only strips `[[ ]]` outside
code blocks — the invisible provenance citations all leak into user-visible output. This
is the only page in the corpus where citations are displayed.

**Duplicated section pair.** `## Binding packages` appears at `:71` and `:101`;
`## Native functions` at `:87` and `:117`. `diff <(sed -n 71,100p) <(sed -n 101,130p)`
differs only in the line-wrapping of one sentence — the block is otherwise byte-identical.
Both copies are inside the runaway fence, which is presumably how it went unnoticed.

**Missing diagnostics.** `src/rules/table.rs` defines 25 `NATIVE_*` codes;
`link/package.md`'s Diagnostics section (`:191-214`) names 16, of which 14 are real. The
11 absent: `NATIVE_ABI_UNKNOWN_CTYPE`, `NATIVE_BIND_IN_INVALID`,
`NATIVE_BIND_STATE_INVALID`, `NATIVE_CONST_OUT`, `NATIVE_CONST_UNKNOWN_SLOT`,
`NATIVE_CSTRUCT_ESCAPE`, `NATIVE_CSTRUCT_INVALID`, `NATIVE_CSTRUCT_TOO_LARGE`,
`NATIVE_FREE_INVALID`, `NATIVE_LIBRARY_VENDOR_COLLISION`,
`NATIVE_STRUCT_FIELD_MISMATCH`. Two documented names — `NATIVE_ABI` and `NATIVE_SYMBOL` —
match no entry in the table and are presumably prefixes of real codes.

- Fix: close the fence at `:60`, delete lines `:101-130`, reconcile the diagnostics
  table against `src/rules/table.rs`.
- Note: Agent 07 #19 additionally reports that `link/package.md` and
  `spec/language/17_native-libraries.md` still document the removed `RESULT` clause and
  magic `return` slot, which would make the flagship SQLite example a
  `NATIVE_ABI_NO_RESULT` error. That is a larger spec-plus-man item spanning
  `src/docs/spec/**`; **not verified here** and deliberately left out of this document —
  it should get its own bug rather than be folded into a man-only cluster.

### D8 — `collections::window` and `findLastIndex` document parameter names the compiler rejects — and both are reserved keywords

| The page says | The code does |
| --- | --- |
| `src/docs/man/builtins/collections/window.txt:5` — `collections::window OF T(value AS List OF T, size AS Integer, step AS Integer = 1)` | `src/builtins/collections_package.mfb:301` — `FUNC __collections_window OF T(value AS List OF T, size AS Integer, stride AS Integer = 1)` |
| `src/docs/man/builtins/collections/findLastIndex.txt:5` — `… predicate AS FUNC(T) AS Boolean, end AS Integer = -1)` | `src/builtins/collections_package.mfb:209` — `FUNC __collections_findLastIndex OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean, endIndex AS Integer = -1)` |

Both documented names are reserved words (`STEP` from `FOR`, `END` from block terminators),
so a named call written from the page does not merely fail to bind — it fails to *parse*:

```
$ # collections::window(v, 2, step := 2)
./src/main.mfb:6 error[1-102-0001 MFB_PARSE_EXPECTED_EXPRESSION]: parser expected an expression
   6 |   LET w = collections::window(v, 2, step := 2)
     |                                     ^^^^

$ # collections::findLastIndex(v, isBig, end := 2)
./src/main.mfb:10 error[1-102-0001 MFB_PARSE_EXPECTED_EXPRESSION]: parser expected an expression
  10 |   LET i = collections::findLastIndex(v, isBig, end := 2)
     |                                                ^^^
```

Both compile cleanly with the implementation names (`stride := 2`, `endIndex := 2`).

- Fix: `step` → `stride`, `end` → `endIndex` in the two pages. Renaming the
  implementation instead is not an option — `step` and `end` are unspellable.

### D9 — `crypto::generateP256Raw` / `P384Raw` / `P521Raw` are user-callable with no man page

| The page says | The code does |
| --- | --- |
| No page exists. `scripts/list_functions.py:24-29` (`INTERNAL_CALLS`) deliberately excludes all three from the documented surface: "The `crypto::generateP*Raw` entries are the internal raw-key generators backing the public `generateP*` wrappers." | `src/builtins/crypto.rs:57-59` declares them and `:123-125` routes them through `is_crypto_call`. `src/builtins/mod.rs:381` reaches `crypto::is_crypto_call` with no internal-call filter. |

Repro:

```
$ cat src/main.mfb
IMPORT io
IMPORT crypto

SUB main
  LET k = crypto::generateP256Raw()
  io::print(toString(len(k)))
END SUB

$ mfb build .
Building mfb_project (executable) for macos-aarch64
Wrote executable to ./build/mfb_project.out
```

- The doc tooling and the resolver disagree about whether these are public. This is
  exactly the shape of bug-213, which was resolved by adding
  `src/builtins/audio.rs:239` (`is_audio_internal_call`) and the guard at
  `src/builtins/mod.rs:374` that runs *before* the package dispatch.
- Fix: mirror bug-213 — add `is_crypto_internal_call` and reject the three raw names in
  `is_builtin_call`. Do **not** fix this by writing three man pages; the correct state is
  that they are not callable. (Recorded here because the drift surfaced as a man gap;
  the fix is in `src/builtins/`.)

### D10 — `general/package.md` omits `Money` on five conversions and drops `toMoney` entirely

`grep -c Money src/docs/man/builtins/general/package.md` is **0**.

| The page says | The code does |
| --- | --- |
| `:37-40` — "`toInt`, `toFloat`, `toFixed`, `toByte`, and `toScalar` produce the named type; `toString` renders `Integer`, `Float`, `Fixed`, `Boolean`, `String`, `Byte`, `Scalar`, and `List OF Byte` values as text." `[[src/builtins/general.rs:expected_arguments]]` at `:35` | `src/builtins/general.rs:355-358` (the cited symbol) — `TO_INT => "String[, Integer], Byte, Float, Fixed, Money, or Scalar"`, `TO_FLOAT => "String, Integer, Fixed, or Money"`, `TO_FIXED => "String, Integer, Float, or Money"`, `TO_BYTE => "Integer, Money, or Scalar"`. Resolution at `:243,254,263,272`. |
| The page never mentions `toMoney` | `src/builtins/general.rs:11` (`const TO_MONEY`), `:294-297` — "Explicit crossing into `Money` from every scalar type (plan-29-G §4.2)", returning `Money`. |
| `toString` accepted-type list omits `Money` | `src/builtins/general.rs:210` (two-arg precision form) and `:218-222` (one-arg form) both accept `Money`. |

- Same root as D1: the `Money` work landed and the docs did not follow.
- Fix: add `Money` to the four conversion lists and to `toString`'s accepted types; add
  a sentence for `toMoney`.

### D11 — undocumented named-argument aliases across six packages, several on pages that cite the alias table as their source

`call_param_names` declares alternate spellings that bind today and appear on no page.
Confirmed working:

```
$ # both lines compile and run
io::print(toString(math::min(a := 3, b := 5)))
io::print(toString(math::min(left := 3, right := 5)))
```

| Package | Declared aliases | Page |
| --- | --- | --- |
| `math` | `src/builtins/math.rs:74` `min`/`max` → `["a","left"]`, `["b","right"]`; `:75` `clamp` → `["low","minimum"]`, `["high","maximum"]`; `:77` `pow` → `["base","value"]`, `["exponent","power"]`; `:79` `seed` → `["value","seed"]` | `math/min.txt` PARAMETERS lists only `a`, `b`; `max.txt`, `clamp.txt` likewise — zero occurrences of `left`/`right`/`minimum`/`maximum` |
| `strings` | `src/builtins/strings.rs:123` `replace` → `["old","needle"]`, `["new","replacement"]` | `strings/replace.txt` |
| `encoding` | `src/builtins/encoding.rs:100-102` — nine encoders share `["value","text"]` | `encoding/percentEncode.md:52` documents `text` only, and cites `[[src/builtins/encoding.rs:call_param_names]]` |
| `fs` | `src/builtins/fs.rs:128` `writeBytes`/`writeBytesAtomic`/`appendBytes` → `["bytes","value"]` | `fs/writeBytes.md:68` documents `bytes` only, and cites `[[src/builtins/fs.rs:call_param_names]]` |
| `http` | `src/builtins/http.rs:115,120,123` | per-package pages |
| `net`, `tls` | `src/builtins/net.rs:115,123-125`; `src/builtins/tls.rs:80` | per-package pages |

- The two `.md` cases are the notable ones: the page cites the exact table it is
  half-transcribing. A reader has no way to discover the alias, and a reader who *is*
  using the alias has no way to confirm it is supported.
- Fix: document each alias in the page's Parameters table ("also spelled `left`"), the
  convention `general/package.md:44-45` already uses for `toString`'s
  `precision`/`decimals`. Alternative — deleting the aliases — is rejected: they are a
  shipped surface and removing them is a breaking change.

### D12 — the dangling provenance citation (1 of 530)

| The page says | The code does |
| --- | --- |
| `src/docs/man/builtins/net/package.md:40` — `… pairing each payload with its sender. [[src/builtins/net.rs:record_fields_for_type]]` | `src/builtins/net.rs:88` — the function is `builtin_type_fields`. `record_fields_for_type` appears nowhere in the tree. |

- Verified: a resolver run over all 530 unique citations finds this as the **only**
  dangling symbol. (Six others are malformed file-only citations with no `:Symbol`;
  those name files that exist and are tracked as bug-336 S12.)
- The claim it backs is correct — `net.rs:88-95` does define `Address`, `Datagram`, and
  `DatagramText` fields — so this is a traceability defect, not a content defect.
- Fix: `record_fields_for_type` → `builtin_type_fields`. Land bug-336's
  `check_man_citations.py` in the same change so it cannot recur.

### D13 — two dead `mfb man` invocations

| The page says | Observed |
| --- | --- |
| `src/docs/man/builtins/money/package.md:18` and `:52` — "(see `mfb man types money`)" and a See Also entry `mfb man types money` | `$ mfb man types money` → `error: unknown function` money` in package `types``. `src/docs/man/types/` contains `comparisons`, `list`, `logical`, `map`, `numeric`, `package`, `pair`, `partition`, `string` — no `money`. `Money` is documented inside `types/numeric.md`. |
| `src/docs/man/types/string.md:87` — See Also entry `mfb man builtins general toScalar` | `$ mfb man builtins general toScalar` → `error: mfb man accepts at most two arguments`. `builtins` is an on-disk directory, not a package; this is the only place the repository layout leaks into a user-facing command. |

- `String`, `Scalar`, `List`, and `Map` each got their own `types/` page; `Money` did
  not, which is what the two dead links are reaching for.
- Fix (money): either give `Money` its own `types/money.md` (recommended — it matches
  every sibling scalar type and makes the two existing links correct) or retarget both
  to `mfb man types numeric`. Fix (string): `mfb man general toScalar` — though note
  that page does not exist either (bug-336 S5).

### D14 — the language tour omits `Scalar` and `Money` from its primitive list

| The page says | The code does |
| --- | --- |
| `src/docs/man/tour/package.md:56` — "Scalars are `Integer`, `Float`, `Fixed`, `Boolean`, `String`, and `Byte`." | `src/docs/man/types/package.md:33-59` and `types/string.md:3,20-23` document `Scalar` (32-bit Unicode, plan-41) and `Money` (i64 scaled 10⁻⁵, plan-29) as primitives. |

- `grep -c 'Money' src/docs/man/tour/*.md` is 0 across all six tour pages; `Scalar` as a
  *type* likewise appears zero times (the single lexical hit on `:56` is the word
  "Scalars" introducing the sentence above).
- The tour is the first thing a new user reads, and this is its only content drift —
  the other five tour pages check out.
- Fix: add `Scalar` and `Money` to the sentence.

## Goal

- No man page states something the compiler does not do.
- No `mfb man <pkg>` FUNCTIONS list names an uncallable function.
- Every error code printed on a man page equals the value of the constant cited beside
  it.
- Every example program on a man page compiles.

### Non-goals (must NOT change)

- Any compiled-program behavior, with one deliberate exception: D9's fix is a resolver
  guard in `src/builtins/`, which makes three currently-callable names uncallable. That
  is the intended correction and matches bug-213's precedent.
- The named-argument aliases in D11 — they are a shipped surface. Document them; do not
  delete them.
- The `.txt` → `.md` migration (bug-336 S1). Fix these claims in place. A migration that
  silently rewrites a wrong claim into prettier Markdown makes the wrong claim harder to
  find, not easier.
- The macOS/Linux TLS error-code divergence itself (D6) — document the matrix; changing
  macOS to distinguish unresolvable hosts is separate work.

## Blast Radius

Searched, with a verdict per site:

- `src/docs/man/builtins/math/*` (35 legacy pages + `package.md`) — D1, all in scope.
- `src/docs/man/builtins/tls/{accept,connect,listen,package}.md` — D2, D5, D6, in scope.
- `src/docs/man/builtins/encoding/{utf8Encode,utf8EncodeBytes,utf8EncodeInts,utf16Encode,utf32Encode}.md`
  — D3, in scope. No other page in the corpus contains an invalid call (all 485 scanned).
- `src/docs/man/builtins/net/{toAddress.txt,toUrl.txt,package.md}` — D4, D12, in scope.
- `src/docs/man/link/package.md` — D7, in scope.
- `src/docs/spec/language/17_native-libraries.md` — the `RESULT`-clause drift Agent 07
  reports alongside D7. **Not verified in this document; out of scope** — it spans the
  spec corpus and needs its own bug.
- `src/docs/man/builtins/collections/{window,findLastIndex}.txt` — D8. The other 36
  `collections` pages were not individually re-checked against
  `collections_package.mfb`; they are latent, same hazard, and are covered by bug-336
  S1's regeneration of that package.
- `src/builtins/crypto.rs`, `src/builtins/mod.rs` — D9, the only source change here.
- `src/docs/man/builtins/general/package.md` — D10, in scope.
- Six packages' `call_param_names` tables and their pages — D11, in scope.
- `src/docs/man/{tour,types}/*` — D13, D14, in scope.
- Unaffected: `src/docs/man/{flow,lambda,errors,unicode}/*` — reviewed, no drift found.
- Latent, out of scope: the cleanup review's Agent 18 notes its man cross-check covered
  only `http`/`json`/`csv`/`audio`; `crypto`, `datetime`, `encoding`, `collections`,
  `net`, `vector`, `strings`, `regex`, and `money` were not individually verified against
  their `.mfb` sources and may hold further drift of the D8 shape.

## Fix Design

Land in impact order — D1–D3 first, since those are the items a reader can act on and be
wrong. Every item is a localized edit; the only ordering constraint is that D9's resolver
guard should land with a test before or alongside the doc items, since it is the one
change with a runtime surface.

Two items are worth resisting the tempting wrong fix:

- **D4** — do not "fix" `toAddress` by rewording the page to be clearer that it is
  unimplemented. `mfb man net` will still list it under FUNCTIONS, because `build.rs`
  lists every page in a package directory (bug-336 S8). Delete the page or implement the
  function.
- **D9** — do not write three man pages for `generateP*Raw`. `list_functions.py:24-29`
  already records the intent that they are internal; the defect is that the resolver
  never enforced it.

D2 is a one-character-class edit but has an ordering implication for bug-336's citation
checker: the citation at `tls/package.md:94` *resolves* (the symbol
`ERR_TIMEOUT_CODE` exists) while the number beside it is wrong. A symbol-existence check
does not catch this. If the checker is later extended to compare a cited constant's value
against a nearby literal, D2 is the motivating case — worth recording in that script's
header comment.

Expected output shift: none to compiled programs. `mfb man link` output changes
substantially (D7 un-fences ~83 lines and deletes 30), and any golden capturing it churns
wholesale; that churn is expected and is the intended extent.

## Phases

### Phase 1 — repros and the one source change

- [ ] Add `tests/acceptance/` fixtures for D3 (the corrected `encoding` example
      compiles), D8 (`stride :=` / `endIndex :=` bind), and D9 (`crypto::generateP256Raw`
      is rejected). Confirm D9's fails against current behavior — it compiles today.
- [ ] D9: add `crypto::is_crypto_internal_call` and gate it in
      `src/builtins/mod.rs:is_builtin_call`, mirroring `audio.rs:239` /
      `mod.rs:374`.

Acceptance: the D9 test flips from compiles-successfully to rejected; D3/D8 fixtures pass
with the corrected spellings.
Commit: —

### Phase 2 — high-impact content corrections

- [ ] D1: `Money` rows on eight `math` pages + `package.md:31-33`; delete the three
      negating TYPE CHECKING paragraphs (`floor.txt:71`, `ceil.txt:75`, `round.txt:72`).
- [ ] D2: `77060001` → `77050008` in `tls/accept.md:80` and `tls/package.md:94`.
- [ ] D3: `collections::len` → `len` in the five `encoding` pages; drop the stray
      `IMPORT collections`.
- [ ] D4: delete `net/toAddress.txt`; strike `net/package.md:45,72,73` and
      `net/toUrl.txt:30,52`.
- [ ] D5: rewrite `tls/connect.md:43,79` and `tls/package.md:66`; add the `ErrTimeout`
      row to `connect.md`'s Errors table.
- [ ] D6: per-platform note on `tls/connect.md:93`, `tls/listen.md:92`,
      `tls/package.md:95-96`.
- [ ] D7: close the fence at `link/package.md:60`; delete `:101-130`; add the 11 missing
      `NATIVE_*` codes and reconcile `NATIVE_ABI`/`NATIVE_SYMBOL`.
- [ ] D8: `step` → `stride`, `end` → `endIndex`.

Acceptance: `mfb man net` no longer lists `toAddress`; `mfb man link` renders no code
block after the ABI example; the Phase 1 fixtures still pass.
Commit: —

### Phase 3 — remaining items and validation

- [ ] D10: `Money` + `toMoney` in `general/package.md`.
- [ ] D11: document the aliases across the six packages.
- [ ] D12: `record_fields_for_type` → `builtin_type_fields` in `net/package.md:40`.
- [ ] D13: create `types/money.md` (or retarget both links); fix `types/string.md:87`.
- [ ] D14: add `Scalar` and `Money` to `tour/package.md:56`.
- [ ] Re-run bug-336's `check_man_citations.py`; it should now be clean of dangling
      symbols.
- [ ] Regenerate any `mfb man` goldens; confirm the delta is only D7's `link` page and
      the edited pages.

Acceptance: full acceptance suite green; golden deltas are exactly the edited pages.
Commit: —

## Validation Plan

- Regression tests: the three Phase 1 acceptance fixtures (D3, D8, D9).
- Runtime proof: `mfb man net | grep toAddress` returns nothing; `mfb man link` shows no
  runaway code block and no `[[…]]` in output; `mfb man tls accept | grep 77050008`
  matches; `mfb man types money` resolves (D13); the corrected `encoding` example builds
  and runs.
- Doc sync: none outward — this document *is* the doc sync. The `spec/` side of the
  `RESULT`-clause drift noted under D7 is explicitly deferred to its own bug.
- Full suite: `scripts/test-accept.sh` plus the man/spec render checks.

## Open Decisions

- **D4** — delete `net/toAddress.txt` (recommended: a man page is not a roadmap) vs.
  implement `net::toAddress`, which is a small `Url` → `Address` projection over
  `src/builtins/net.rs:88-95` and would make four existing cross-references correct.
- **D13 money** — add `types/money.md` (recommended: matches `string`/`numeric`/`list`/
  `map` and repairs both dead links) vs. retarget the two links to `types numeric`.
- **D6** — document the platform matrix (recommended, in scope) vs. make macOS
  distinguish resolution failures from handshake failures (behavior change, separate
  bug).

## Summary

The engineering risk is almost entirely in the audit, not the edits: each item is a
one-to-thirty-line change, but establishing that a page contradicts the code required
reading both sides, and the same discipline is needed for the packages the cleanup review
did not reach (`crypto`, `datetime`, `collections`, `strings`, `regex`, `money`,
`vector`). Exactly one item — D9 — touches compiled behavior, and it does so to *remove*
an accidentally-public surface, following bug-213's precedent. Everything else leaves the
compiler untouched and makes the documentation stop lying about it.
