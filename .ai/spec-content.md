# The `mfb spec` content standard

> **Audience: the compiler contributor — and the developer who wants the
> internal detail.** Internals are not merely permitted here, they are the
> point. The mirror standard for the other surface is
> [`.ai/man-content.md`](man-content.md), whose audience is the MFBASIC
> developer; a sentence cut from a man page for being too internal is a
> candidate **spec obligation**, not deleted knowledge. See §8.

**Read this before editing any file under `src/docs/spec/**`.**

This is the *review* standard: what a spec topic must contain, what it must
not, how to verify a claim, and how to triage a spec/code disagreement. It
sits **on top of** [`.ai/specifications.md`](specifications.md), which states
the mechanical rules — single source of truth, `[[ ]]` provenance,
`PACKAGE_ORDER`, the error-code registry as build input — and it does not
replace any of them. Authored by plan-125-A.

## 0. Where the content lives, and what reads it

`src/docs/spec/**` is markdown embedded in the binary and rendered by
`mfb spec <pkg> [<topic>] [--all]`. 12 packages in the reading order of
`src/docs/spec/mod.rs:PACKAGE_ORDER`; 146 files at the time of writing
(`./scripts/spec-census.sh --fill`).

The spec is **version-locked to the code**: the spec you read always matches
the binary you have. That is the property the whole surface exists to
provide, and it is also what makes a stale claim dangerous rather than merely
untidy — a reader has no way to tell a current contract from a retired one.

**One file is not inert prose.** `src/docs/spec/diagnostics/02_error-codes.md`
is **build input**: `build.rs` generates the `errorCode::` constants from its
Constant Registry table, and
`src/codegen/builtins/errorcode/mod.rs:table_matches_registry` guards the
drift. Any edit to that table is a compiler-visible change — run `cargo build`
and `cargo test errorcode`.

## 1. Who the topic is for

*A spec topic is written for someone changing the compiler, or for a developer
who has asked for the exact contract.*

The test, and it is the mirror of the man standard's: **if a sentence is only
reassurance, orientation, or encouragement, it does not belong here.** If a
reader needs to be taught the concept before they can use it, that is
`mfb man`'s job — link it.

The two surfaces are deliberate opposites, and the table is worth stating once:

| | `mfb man` | `mfb spec` |
|---|---|---|
| Reader | a developer **using and learning MFBASIC** | a **compiler contributor**, or a developer who wants the internal detail |
| Answers | "what does this do, how do I call it, what goes wrong" | "what is the exact contract, and where in the compiler is it implemented" |
| Internals | **banned** — no IR, no lowering, no ABI, no codegen, no plan/bug numbers, no mangled symbols | **required**, and cited with `[[path:Symbol]]` |
| Memory words | the four permitted only: copy, mutate, value, alias | the precise contract in its own vocabulary — **the ban does not apply** |
| Examples | runnable MFBASIC a developer would write; compiled and run during review | illustrative fragments; correctness of the *claim* outranks runnability |
| Voice | second person, task-first | normative, precise, no tutorial scaffolding |

## 2. What a topic MUST contain

- **The normative contract, stated precisely.** Exact values, exact ranges,
  exact ordering, exact error behaviour. "Generally returns quickly" is not a
  contract; "returns within one scheduler quantum, or raises
  `ErrWouldBlock`" is.
- **What is guaranteed versus what merely happens to be true today.** These
  are different facts and a reader cannot distinguish them by inspection. A
  spec that silently promotes an implementation accident to a contract is
  worse than one that omits it, because the next contributor will preserve it.
- **Provenance for every non-obvious implementation claim** — a magic number,
  an offset, an ABI register, an enum variant, a capability list, a pass
  ordering — as an invisible `[[src/file.rs:Symbol]]` citation at
  claim-cluster granularity. Symbol-preferred; `[[src/file.rs:line]]` only
  where no symbol fits. **Grep-confirm the symbol before citing it**; §5 is the
  instrument.
- **The failure modes**, not only the success path. What happens on overflow,
  on an empty input, at a boundary, under concurrency, on each backend where
  backends differ.
- **Where backends differ, which backend.** A claim true only on macOS
  AArch64 and stated unqualified is a wrong claim on four other targets.

## 3. What a topic MUST NOT contain

- **Tutorial prose.** Motivation, encouragement, "don't worry", a gentle
  build-up. Link `mfb man` instead.
- **Marketing.** "blazingly fast", "elegant", "simply". A superlative with no
  measurement behind it is not a contract.
- **A second full copy of another topic's body.** `.ai/specifications.md`'s
  first convention: each fact has one canonical topic; others give a short
  summary and a `mfb spec <pkg> <topic>` link. A rats-nest of duplicated
  bodies is how two topics come to disagree, and nothing detects it.
- **Unverifiable claims.** If you cannot point at the code that makes it true,
  either find the code or cut the sentence. Do not add a claim you could not
  check.
- **Aspirational behaviour.** *The spec describes the compiler as it is at
  HEAD, not as it is designed, not as it will be after the next plan lands.*
  A planned contract that does not exist yet is a lie with a citation
  attached. If a topic must mention direction, it says so explicitly and
  carries no citation.
- **Plan and bug numbers.** They are session state; the spec is the contract.
  Cite the symbol that implements the behaviour, not the ticket that changed
  it — git owns that.

## 4. Accuracy: the as-is rule, and how to satisfy it

**Every claim describes the compiler at HEAD.** The only ways to establish
that are:

1. **Read the cited symbol.** A claim with a resolving `[[path:Symbol]]`
   citation is checked *at that symbol*. This is the cheap path and it is why
   citations are mandatory for non-obvious claims — they make the spec
   auditable in bounded time.
2. **Run it.** Build a probe with the release binary and observe. Preferred
   whenever the claim is externally observable.
3. **Cut it.** A claim with no citation, that you cannot locate in the code
   and cannot observe, is not a contract. Give it a citation or remove it.
   Leaving it is the failure mode this standard exists to stop.

Verifying a spec claim can mean reading a whole compiler pass, so the triage
above is the budget: citation first, probe second, cut third.

### 4.1 The two rot classes — and why they need different repairs

`./scripts/spec-census.sh --citations` reports, for every citation whose
symbol is not in the cited file, whether that symbol exists **anywhere** under
`src/`, `build.rs` or `repository/src`. That single column splits the failures
into two classes that must never be handled the same way:

| Class | What it means | The repair |
|---|---|---|
| **STALE BY MOVE** (`ELSEWHERE=yes`) | the symbol still exists, at another path — typically a `package.mfb` split or a module reorganisation | re-point the citation, then spot-check that the claim still holds at the new site |
| **STALE BY DELETION** (`ELSEWHERE=no`) | the symbol exists nowhere | **the CLAIM is suspect, not just the link.** The thing the sentence describes has been deleted or renamed out of existence. Verify the behaviour from scratch, then re-cite it or cut the sentence |

**A citation fixer that only re-points paths silently ratifies class two.**
That is the specific failure mode this split exists to prevent: the link goes
green, the sentence stays wrong, and the surface now looks audited.

## 5. Verification instruments

`scripts/spec-census.sh` is the whole toolbox; before plan-125 there was none.

```
./scripts/spec-census.sh                       # per-package inventory
./scripts/spec-census.sh --citations [pkg...]  # resolve every [[ ]] marker
./scripts/spec-census.sh --links [pkg...]      # resolve every mfb spec/man ref
./scripts/spec-census.sh --render [pkg...]     # render; check for leaked [[
./scripts/spec-census.sh --fences [pkg...]     # code fences by language tag
```

`--citations` and `--links` are the two that turn "the spec is probably fine"
into a number. Run both at the top and the bottom of any spec work.

Exit status is 0 only when `--citations` reports no `MISS-*` and `--links`
reports no unresolved target.

**The renderer strips `[[ ]]` everywhere**, including headings, so a marker
that survives into rendered output is malformed — that is what `--render`
checks, along with a package that renders empty.

### 5.1 The gates

Unlike the man surface, spec work **does** have compiler gates, because the
spec is embedded and one file is build input. The blast radius, and no more:

```
cargo build                     # regenerates the embedded table
cargo test --bin mfb spec
cargo test errorcode            # ONLY if diagnostics/02_error-codes.md changed
```

If a brand-new file is not picked up, `touch build.rs` and rebuild.

## 6. Code fences

Spec fences are **illustrative**, and legitimately partial — a fragment
showing one register move or one struct layout is the right thing to write.
Correctness of the surrounding *claim* outranks runnability of the fence.

So: a non-compiling fence is a **finding to triage**, never an automatic
defect. `--fences` inventories them by language tag so a reviewer knows which
are MFBASIC and worth compiling at all. A fence tagged as MFBASIC that reads
like a whole program and does not compile is a real defect; an untagged
three-line fragment is not.

## 7. Triaging a spec/code disagreement

When the spec says X and the code does Y, **never average them.** Decide which
is wrong:

- **Spec stale** → fix the spec. Cite the symbol you read.
- **Code wrong** → this is a found compiler bug and AGENTS.md applies: it is
  not left. Small → fix it now. Large → a `bug-NN` document with a repro.
  Record it either way; a documentation pass is allowed to find compiler bugs
  and is not allowed to ignore them.
- **Genuinely undecided** → that is itself the finding. Say which is
  authoritative and why, in the topic, with the citation.

## 8. The seam with `mfb man`

The two standards are mirror images, and that is the useful part.

**A sentence cut from a man page for being too internal is a candidate spec
obligation.** It was true — it was merely written for the wrong reader. When a
man-surface pass cuts one, it appends a row to
`planning/plan-125-belongs-in-spec.md` naming the unit, the cut sentence, and
the spec package it belongs to. A spec pass opens that ledger as a **coverage
checklist**: every entry is either already covered by a topic (record where)
or a spec gap to fill.

The reverse direction has a rule too: **if a man page needs spec-level
precision, that is the signal it is saying too much** (`.ai/man-content.md`
§4.4). Cut it, link `mfb man variable` or the owning topic, and put the
precision here.

The two surfaces must not contradict each other. Where they overlap — the
memory model, resources, stdlib semantics — the spec is authoritative for the
contract and the man page is authoritative for how a developer should think
about it, and both must be true.
