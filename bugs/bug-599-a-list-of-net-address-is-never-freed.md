# bug-599: a `List OF net::Address` is never freed, in any position

Last updated: 2026-09-12
Effort: medium (a deep drop, or a layout change)
Severity: MEDIUM — unbounded growth in any loop that resolves a host, successful or not
Class: Memory

Status: Open
Regression Test: an RSS case in `tests/runtime/rt_scope_drop_leaks.rs`, once fixed

## How it was found

The bug-593 agent found this while separating variables. It is not bug-593: bug-593's
fix gives an inline `TRAP`'s `Result` wrapper an owner, and this list leaks with **no
`TRAP` at all**. Its note is in `bugs/bug-593-…md` on branch
`bug-593-failing-helper-flat-block` ("`List OF net::Address` bindings — a SEPARATE
defect"). The number was verified free on main, every worktree, and
`git log --all --grep=bug-599`. Defect search: `grep -rliE "net::Address|pointer-string"
bugs/` finds only bug-483's layout doc, which says nothing about drops, and bug-55, which
is `net::lookup`'s `freeaddrinfo` on a fault path.

## The shape, as measured by that agent (macOS, release)

- A **succeeding** `net::lookup("127.0.0.1")` propagated out of a `FUNC`, with no `TRAP`,
  grows **18.0 → 34.8 MB** from 20 000 to 40 000 iterations.
- On the error path, the default empty list `$trap_valN` leaks the same way.
- The emitted code of the `List OF net::Address` probe carries no
  `_mfb_rt_drop_owned_collection` at all.

A `net.Address` record holds pointer `String`s (the host). A list of such a record is not a
flat value, so no drop is ever emitted for it, in any position.

## What a fix must decide

Two candidates, recorded by the finder and not yet evaluated:

1. **A deep drop** over each element's out-of-line host `String`, then the list block. On
   the `TRAP` Ok path the success binding aliases the wrapper's payload (bug-593 frees that
   wrapper only on the failure tag for exactly this reason), so the drop must be owned by
   exactly one binding.
2. **Flattening `net.Address`** (an inline host), which bug-483 considered and did not do.
   That is an ABI change for every `net`/`tcp`/`udp`/`tls` member that takes or returns an
   address.

## Memory gate (when fixed)

1. A RED RSS pin at >=200k iterations that flips flat, measured with `--test-threads=1`.
2. Name the contract: `mfb spec` §14's preamble and §14.7, as bug-593 did. Show the change
   only ADDS a free or, if it changes layout, say so and show every consumer.
3. A POSITIVE pin: a looked-up address still prints its host and port after the list that
   produced it is gone, a copied-out element survives, and an iterated list is unchanged.
4. Golden deltas confined to emitting fixtures; zero `.run` goldens moving.
