# bug-596: a named call cannot omit an overloaded builtin's trailing default — `tls::connect`'s documented form does not build

Last updated: 2026-09-12
Effort: small
Severity: MEDIUM — the documented `tls::connect(host, port, timeoutMs := …, serverName := …)` form is rejected; every named `tls::connect` call that leaves out `allowSelfSigned` fails
Class: Correctness / regression (bug-477)

Status: Open
Regression Test: `src/codegen/builtins/mod.rs` — `a_named_call_may_omit_an_overloaded_builtins_trailing_optionals`, `a_named_call_that_skips_a_middle_parameter_selects_no_overload`

## How it was found

The bug-472 man-example gate, run over the whole corpus, failed `tls::connect`
examples 2 and 3 at BUILD. plan-108-F had built every `tls` example on 2026-08-31.

## Reproduction (round-3 binary `d2e923967`, macOS; one-file projects)

| call | result |
|---|---|
| `tls::connect("h", 443, serverName := "x")` | `TYPE_CALL_ARITY_MISMATCH`: omits parameter `allowSelfSigned` before a later supplied argument |
| `tls::connect("h", 443, 5000, serverName := "x")` | same |
| `tls::connect("h", 443, timeoutMs := 5000)` | omits parameter `serverName` … |
| `tls::connect("h", 443, allowSelfSigned := TRUE)` | omits parameter `serverName` … |
| `tls::connect("h", 443, 5000, "x")` | builds |
| `tls::connect("h", 443, timeoutMs := 5000, serverName := "x", allowSelfSigned := FALSE)` | builds |

A user FUNC with two defaults (`f(1, c := 7)`) and a single-overload builtin
(`tcp::connect(…, timeoutMs := 100)`) both build. The fault is confined to builtins
whose overloads disagree on layout, which are normalized through a per-overload name
table.

## Root cause

`select_param_name_overload` (`src/codegen/builtins/mod.rs`) is the one selector that
both the type checker (`ir::shape`) and IR lowering (`normalize_overloaded_builtin_call_arguments`)
use. It accepted only an overload taking **exactly** `positionals + names` arguments.
`tcp::connect` never noticed, because its optional `timeoutMs` is a separate
overload of each arity. bug-477 (`511bdf31c`) gave `tls::connect` a trailing
defaulted `allowSelfSigned` **inside** each of its two implementations. After that,
no overload had the arity of a named call that left it out. Selection failed, and the
checker's fallback reported the first unsupplied slot, even though that slot trails
the supplied ones and has a default.

The spec sentence (`mfb spec language functions`, "Named args") says both "after omitted
default parameters are filled" and "the one taking exactly this many arguments". That
arity clause predates any overloaded builtin with a default past the named set, and it
is the part that is wrong.

## Fix

The selector now requires the supplied arguments to fill a **contiguous prefix** of the
overload. An exact-arity overload is preferred, so tcp, datetime and crypto select
exactly as before. Failing one, it takes the shortest longer overload whose
unsupplied slots all trail the supplied ones, and leaves them to the same trailing
`Fill` padding a positional call receives. A gap before a later supplied name is still
never selected, so the located "omits parameter" diagnostic stays for that shape
(`func_tcp_invalid`'s `tcp::connect(host := "h", timeoutMs := 5000)` is unchanged).

Selection is by names and positions only; it never sees types. `tls::connect("h", 443,
serverName := "x")` therefore now selects the `Address` form, whose second positional IS
`timeoutMs`, and the type check rejects it with `TYPE_CALL_ARGUMENT_MISMATCH` listing
both signatures. Before the fix it was `TYPE_CALL_ARITY_MISMATCH`, which named
`allowSelfSigned` wrongly. The same structural rule is what makes
`tls::connect(address, 5000, serverName := "x")` build, and it did not before. Measured
with the fixed binary:

| call | before | after |
|---|---|---|
| `tls::connect("h", 443, timeoutMs := 5000, serverName := "x")` (man example 2) | rejected | builds |
| `tls::connect("h", 443, 5000, serverName := "x")` | rejected | builds |
| `tls::connect("h", 443, timeoutMs := 5000)` | rejected | builds |
| `tls::connect(a, 5000, serverName := "x")` (`a AS net::Address`) | rejected | builds |
| `tls::connect(a, timeoutMs := 5000)` | rejected | builds |
| `tls::connect("h", 443, serverName := "x")` (skips `timeoutMs`) | rejected | rejected |
| `tls::connect("h", 443, allowSelfSigned := TRUE)` | rejected | rejected |
| every positional form, and every all-named form | builds | builds |

## The first cut re-opened bug-349, and what stops that now

The first version of the fallback (`1d0fa73b9`) accepted any longer overload the supplied
names prefix-filled, without asking whether the slots it left out had defaults.
`datetime::instant`'s overloads drop REQUIRED components off the front
(`instant(seconds)` … `instant(days, hours, mins, seconds, nanos)`). So
`datetime::instant(days := 5)` prefix-filled the 5-arg form, and its one argument then
type-checked against the 1-arg `seconds` form: 5 days would be read as 5 seconds. That
is bug-349's silent misbinding, back. The full unit suite caught it.
`the_syntax_corpus_reproduces_its_goldens_in_process` reported that
`bug349_instant_named_arg_arity_invalid`'s three `TYPE_CALL_ARITY_MISMATCH` errors had
become an accepted program.

The fallback now fails CLOSED. `registry::call_param_name_overload_required` gives, per
overload, the count of leading parameters up to the last `DefaultValue::None`. A longer
overload is selected only when the call supplies at least that many, so every omitted
slot is `Fill` or `Optional`. With no counts there is no fallback. Both callers pass the
counts: `ir::shape`'s checker and `ir::lower`'s normalization. The fixture reproduces its
three errors again, identical to its golden, and `the_fallback_never_leaves_out_a_required_parameter`
pins `instant(days)`, `instant(days, hours)` and `duration(hours)` to no selection.

## Not changed, recorded

A builtin still rejects a named call that skips a MIDDLE defaulted parameter:
`tls::connect("h", 443, serverName := "x")` omits `timeoutMs`. The single-overload path
(`ir::shape`, "omits parameter … before a later supplied argument") rejects the same
shape for every builtin, while a user FUNC accepts it. That was true before bug-477 and
is not what regressed. `tls::connect` example 3 relied on it, and it is corrected on the
bug-472 branch to name `timeoutMs`.
