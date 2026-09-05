# bug-526: `mfb man tls poll` prints a list-overload signature that does not compile

Last updated: 2026-09-04
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: `tests/` — a renderer pin asserting the `RES` marker survives into every rendered signature

`mfb man tls poll` renders its multiplex overload as:

```
tls::poll(socks AS List OF tls::Socket, [timeoutMs AS Integer]) AS tls::Socket
```

and repeats `List OF tls::Socket` in the Parameters table. That declaration is
not legal MFBASIC. Writing `List OF tls::Socket` produces:

```
error[2-203-0082 TYPE_RESOURCE_REQUIRES_RES]: resource must be bound with RES
    Collection element type `tls.Socket` is a resource; mark it `RES`
    (e.g. `List OF RES File`), not a bare resource type.
```

The same page's Description gets it right — "Given a `List OF RES tls::Socket`
… The elements must be marked `RES`" — and so does its own example, which
declares `MUT socks AS List OF RES tls::Socket`. So the page contains both the
correct form and an uncompilable one, and the uncompilable one is in the two
places a reader looks first.

`mfb man tcp poll` renders the identical overload correctly, as
`tcp::poll(socks AS List OF RES tcp::Socket, …)`. The two members differ only
in how their parameter type is constructed in Rust.

The single correct behavior a fix produces: `mfb man tls poll`'s rendered
signature is a declaration that compiles, matching `mfb man tcp poll`.

References:

- `src/codegen/builtins/tls/func_poll.rs:151-165` — the parameter that drops `RES`
- `src/codegen/builtins/tcp/func_poll.rs:146-150` — the same parameter, correct
- Spike: `spikes/api-review/bug-526-tls-poll-res/`

## Failing Reproduction

```
./target/release/mfb man tls poll | head -20
./target/release/mfb man tcp poll | head -20
```

- Observed:

```
tls::poll(socks AS List OF tls::Socket, [timeoutMs AS Integer]) AS tls::Socket
tcp::poll(socks AS List OF RES tcp::Socket, [timeoutMs AS Integer]) AS tcp::Socket
```

  Then, compiling what the `tls` page prints:

```
MUT bad AS List OF tls::Socket = []
```

```
error[2-203-0082 TYPE_RESOURCE_REQUIRES_RES]: resource must be bound with RES
```

- Expected: `tls::poll(socks AS List OF RES tls::Socket, …) AS tls::Socket`.

Contrast case: `tcp::poll` is correct, and it is the proof that the renderer
*can* emit `RES` in a list element — so this is not a renderer limitation, as
the comment in the tls source asserts.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ (descriptor defect; target-independent) |

## Root Cause

The two packages build the same parameter type two different ways.

`src/codegen/builtins/tcp/func_poll.rs:149`:

```rust
ParameterType::list_of(ParameterType::Res(Box::new(super::socket())))
```

`src/codegen/builtins/tls/func_poll.rs:159-161`:

```rust
ty: ParameterType::ListOf(Box::new(ParameterType::named(
    super::TLS_SOCKET_TYPE_ID,
))),
```

The `tls` form never constructs a `ParameterType::Res` node, so the renderer has
nothing to print. The comment above it explains the omission:

> The element is the bare resource id: `ParameterType::parse` strips the `RES `
> ownership marker off a list element, so the concrete `List OF RES tls.Socket`
> argument unifies as `ListOf(Named("tls.Socket"))`. (The `RES` requirement
> itself is enforced separately by the resource/type checker.)

The first half is a true statement about `ParameterType::parse` — parsing the
string drops the marker. The conclusion drawn from it is wrong: `tcp::poll`
constructs the `Res` node *directly* rather than by parsing, keeps it, and
still unifies against the same concrete argument. So the descriptor could
always have carried the marker; the comment rationalized an omission into a
constraint.

That makes this a small instance of a general hazard: a `ParameterType` built by
parsing a string and one built by construction are not interchangeable, and the
difference is invisible until something renders it.

## Goal

- `mfb man tls poll` prints `List OF RES tls::Socket` in both the Overloads
  block and the Parameters table.
- The list overload still resolves for the same arguments it resolves for
  today — no overload-resolution change.
- A test fails if a resource-typed list parameter loses its `RES` marker again,
  in any package.

### Non-goals (must NOT change)

- `tls::poll`'s behavior, overload set, or resolution. The scalar
  `poll(sock, timeoutMs) AS Boolean` and the multiplex form must select exactly
  as they do now.
- `tcp::poll`, which is already correct.
- `ParameterType::parse`'s marker-stripping, which is a separate and
  deliberate rule — the one type grammar stays canonical.
- The Description and example on the tls page, which are already right.
- **Tempting wrong fix, forbidden:** editing the Description to say
  `List OF tls::Socket` so the page is internally consistent. The Description
  is the half that is true; the signature is the defect.

## Blast Radius

Every resource-typed collection parameter in a registry descriptor. Find them
with `grep -rn "ListOf(Box::new(ParameterType::named" src/codegen/builtins/`
and cross-check against `grep -rn "list_of(ParameterType::Res"`:

- `src/codegen/builtins/tls/func_poll.rs` — fixed by this bug.
- `src/codegen/builtins/tcp/func_poll.rs` — unaffected; already correct, and the
  reference implementation.
- **Every other member taking a `List OF RES T`** — enumerated in Phase 1.
  `udp`, `fs` and `thread` are the likely candidates; any that used the
  `ListOf(named(...))` form has the same rendering defect and is fixed here.
- `src/cli/man.rs` — unaffected; it renders what the descriptor holds, and
  `tcp::poll` proves it handles `Res` correctly.
- Overload resolution — **must be verified, not assumed**. The registry's
  strict matcher gates on whether a parameter is a resource
  (`.ai/resources-packages.md`), so adding a `Res` node changes what the matcher
  sees. `tcp::poll` works with the node present, which is strong evidence, but
  the tls overload set differs (its scalar form takes a bare `Socket`) and must
  be re-tested.

## Fix Design

Change `func_poll.rs:159-161` to use the `tcp` construction:

```rust
ty: ParameterType::list_of(ParameterType::Res(Box::new(
    ParameterType::named(super::TLS_SOCKET_TYPE_ID),
))),
```

and replace the rationalizing comment with a pointer to `tcp::poll` as the
canonical form.

Then add the pin. The general shape of the bug — a descriptor whose rendered
signature is not a legal declaration — is worth catching mechanically, not just
for this parameter. A test that walks every registry function, renders each
overload signature, and asserts that any parameter whose type contains a
built-in resource carries the `RES` marker would have caught this and will
catch the next one.

Rejected: making the renderer infer `RES` for any resource-typed list element.
It would paper over the descriptor being wrong, and the descriptor is what
overload resolution reads — so the signature would be right while the matcher
still saw a different type.

Rejected: fixing only the Parameters table. The Overloads block is generated
from the same `ty`; there is one defect, in one place.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Land `spikes/api-review/bug-526-tls-poll-res/` (done).
- [ ] Add a renderer test asserting `mfb man tls poll`'s signature contains
      `List OF RES tls::Socket`. Confirm it fails today.
- [ ] `grep -rn "ListOf(Box::new(ParameterType::named" src/codegen/builtins/` —
      list every descriptor building a collection of a named type, and mark
      which of those named types are built-in resources. Those are the siblings.

Acceptance: the renderer test fails; the sibling list is complete with a
verdict per entry.
Commit: —

### Phase 2 — the fix

- [ ] Switch `tls/func_poll.rs`'s `socks` parameter to the `Res`-carrying form.
- [ ] Replace the comment with the correct explanation.
- [ ] Apply the same fix to every sibling found in Phase 1.

Acceptance: the Phase 1 test passes; `mfb man tls poll` matches
`mfb man tcp poll` in shape.
Commit: —

### Phase 3 — prove resolution is unchanged + validation

- [ ] Run the existing `tls::poll` overload-resolution tests; add one asserting
      the scalar form still selects for a bare `Socket` argument.
- [ ] Regenerate any `.ncodesum` golden the descriptor change shifts. A
      descriptor `ty` feeds resolution, so a golden diff here is **not**
      automatically drift — objdump one fixture before accepting it.
- [ ] `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh tls --run`.
- [ ] Add the general pin: every rendered signature is a legal declaration.

Acceptance: full suite green; overload resolution demonstrably unchanged; the
general pin is in place.
Commit: —

## Validation Plan

- Regression test: the renderer assertion on `tls::poll`, plus the general
  "rendered signature carries `RES`" pin.
- Runtime proof: `spikes/api-review/bug-526-tls-poll-res/` — the page's own
  signature, pasted into a program, compiles.
- Doc sync: `tls/func_poll.rs` comment; no prose change needed, the Description
  is already correct.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

None. The correct construction is already in the tree, in `tcp::poll`.

## Summary

A one-expression fix whose only real risk is overload resolution: the `Res` node
is visible to the registry's strict matcher, so the change must be proven not to
shift which overload selects. `tcp::poll` running with the node present is
strong prior evidence. The lasting value is the general pin — a rendered
signature that does not compile is a defect class, not an incident.
