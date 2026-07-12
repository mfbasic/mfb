# bug-79: follow-up LOW cluster from the bug-40..73 campaign

Last updated: 2026-07-10
Effort: small each (<1h)
Severity: LOW

Five independent LOW findings surfaced while fixing bugs 40–73. Each was outside
the fixing bug's blast radius and was deliberately left alone. They are grouped
here because each is a few lines; split them if any turns out to be deeper.

---

## (1) macOS TLS leaks the SNI `sec_protocol_options` copy

`nw_tls_copy_sec_protocol_options` returns a +1 reference that the connect path
never releases — roughly one leaked object per TLS connection.

- Found while fixing bug-55 (commit 185e0f8c), which owned the error paths; this is
  on the **success** path and was out of scope.
- Fix: `nw_release` the copied options once the parameters are configured. Prove it
  is not still referenced by the parameters block before releasing.
- Validate with `leaks` across N connections; the count must not scale with N.

---

## (2) `lower_link_thunk` emits duplicate labels

`lower_link_thunk` re-initialises `emit_link_expr`'s label `counter` to `0` for both
the `SUCCESS_ON` and the `RESULT` expression, so a thunk carrying comparisons or
`NOT`s in **both** emits two `..._cmp0_end` labels.

- Found while fixing bug-66 (commit 60117c63); its test was shaped around the
  collision (a bare-`Var` `RESULT`, which emits no labels) rather than through it.
- Whether the assembler silently takes the first or the emitted code is wrong has
  not been established — establish it first, since that decides the severity.
- Fix: thread one counter through both expressions, as bug-56 (cd2d44c9) did for the
  vreg counter.

---

## (3) `pathNormalize("a/..")` returns `"a"`, not `"."`

`pop_scan` leaves `out_len` unchanged when the cancelled component has no preceding
`/`, so the `..` fails to pop the only component.

- Found while fixing bug-65 (commit 000bbcac), which owned the allocation size and
  the `pathBaseName` root case.
- Check the spec and `src/docs/man/builtins/fs/pathNormalize.txt` for the intended
  result of a fully-cancelling path, and of `"a/../.."`.
- Fix in `pop_scan`; add the fully-cancelling and over-cancelling cases to the
  `func_fs_pathNormalize_valid` fixture.

---

## (4) linux-x86_64 imports `write` but never references it

`io.print` on x86-64 issues a raw `svc` (syscall), so the imported `write` symbol is
never a relocation target — a dead import, the same class bug-71 (093d6035) pruned.

- bug-71 pruned `_exit`, `getentropy` and the `io.flush` `fsync`/errno imports, but
  this one sits in the `io.print` arm and was outside the reported list.
- Also still pinned and unreferenced: x86's `math.rand`/`seed` `getentropy` import,
  which `src/docs/spec/linker/08_*.md` documents. Reconcile the spec if it goes.
- Fix: prune, and regenerate the affected `.nplan` goldens.

---

## (5) `pick()(4)` — a returned function value cannot be called directly

The parser rejects calling the result of a call: "only identifiers can be called".
A returned function value must be bound to a local first.

- Found while fixing bug-72 (commit 28c9769e); the object-plan fix made every other
  call shape work, and this is a parser limitation, not a linker one.
- Decide whether this is intended. If a function value is first-class (bug-73
  established that it is, with reference semantics), then `pick()(4)` should parse.
- Fix in the call-expression grammar: allow a call whose callee is any expression of
  function type, not only an identifier. Then confirm the indirect-call lowering
  handles a callee that is a temporary rather than a binding.

---

## Phases

- [ ] (1) SNI options release + `leaks` proof.
- [ ] (2) Establish the duplicate-label consequence, then thread one counter.
- [ ] (3) `pop_scan` fully-cancelling path + spec check.
- [ ] (4) Prune the x86 `write` import; reconcile the `getentropy` spec pin.
- [ ] (5) Decide, then extend the call grammar.
- [ ] `scripts/test-accept.sh` after each.

---
## Resolution (2026-07-11)
- 79.1 (macOS TLS sec_protocol_options leak) — FIXED as bug-116.
- 79.2 (link_thunk shared label counter) — FIXED.
- 79.3 (pathNormalize `a/..` -> `.`) — FIXED.
- 79.4 (x86 dead `write` imports) — FIXED.
- 79.5 (`pick()(4)`: call a returned function value) — LEFT as a parse error per the
  user's decision (deliberate language limitation, not a defect). Reopen if the
  grammar extension is wanted later.
Closing: every defect is fixed; 79.5 is an intentional language restriction.
