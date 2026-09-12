# bug-598: the browser example maps ~1.5–2.3 KB of fresh arena blocks per byte of HTML it loads

Last updated: 2026-09-12
Effort: large (3h–1d) — the attribution is most of the work; the cause is not yet localized
Severity: HIGH — one 604 KB page maps ~0.9 GB that is never returned; every further load adds more
Class: Memory

Status: Open
Regression Test: planned `tests/runtime/rt_dom_parse_arena_growth.rs` (Phase 1; does not exist yet)

Loading a page in `examples/browser` makes the fetch worker, and then the main thread,
map new arena blocks at a rate proportional to the page size, almost all of them
4 KiB. Loading `https://en.wikipedia.org/wiki/BASIC` (603,614 bytes of HTML) made one
worker thread map **194,400 blocks / 939,003,904 bytes** in under a minute. The
browser uses threads, so the arena is never torn down at exit (by design,
`./mfb spec threading os-integration`), and an arena never returns memory to the OS
before teardown (`./mfb spec memory arenas`). So all of it stays mapped until the
process exits, and every page load adds to it.

**The single correct behavior a fix produces:** the memory an arena maps while
loading a page is bounded by the page's live data (the DOM plus its render state),
not by the total allocation volume of the fetch/parse/render work. Concretely,
parsing the same HTML twice in a row must not map a second copy of that memory: the
second parse reuses the freed memory of the first.

Nothing crashes or prints wrong output, which is what makes this dangerous: a
long-running browser session grows without bound and nothing reports it.

References:

- `src/docs/spec/memory/04_arenas.md` — `arena_alloc` grow policy (maps a new block
  only when no bin, carve chunk or free-list chunk fits), "Freeing is internal reuse
  only", and **Scope-Drop Frees** ("recursive / non-flat composites … are excluded
  from scope-drop frees").
- `src/docs/spec/threading/09_os-integration.md` — "Arena Teardown at Process Exit"
  (a threaded program never runs `arena_destroy`, hence 0 munmaps here).
- `src/docs/spec/threading/08_queue-semantics.md` — worker arena lifetime rules.
- `.ai/canvas-threading.md` — bug-498 corollary: a boundary copy is made in the
  sender's arena and adopted by the receiver; a free only touches the freeing
  thread's arena.
- Related, different shape: `bugs/bug-593-a-failing-runtime-helper-call-grows-a-flat-block-per-call.md`
  (~1 KB flat per *failing* helper call; this bug scales with input size on the
  success path).
- Found while measuring how often each example maps an arena block (session of
  2026-09-12; results table in that conversation, method below).

## Failing Reproduction

Measured on remote box **2223** (Kali aarch64, glibc), which has `expect`, network
access, and `ptrace_scope = 0`. Its `strace` is not installed system-wide; unpack it
without root:

```sh
ssh -p 2223 test@127.0.0.1 'mkdir -p ~/strace-pkg && cd ~/strace-pkg && apt-get download strace && dpkg -x strace_*.deb root'
```

Build (from a scratch copy of `examples/`; `examples/**/packages/` is gitignored, so
rebuild the three browser packages from source first — a stale local `display.mfp`
fails with `TYPE_UNKNOWN_FIELD: record astrings.AttrSpan has no member last`):

```sh
M=target/release/mfb; cd <scratch>/examples/browser
$M build dom && cp dom/dom.mfp fetch/packages/ && cp dom/dom.mfp display/packages/ && cp dom/dom.mfp app/packages/
$M build fetch && $M build display && cp fetch/fetch.mfp display/display.mfp app/packages/
$M build --target linux-aarch64 app      # -> app/build/browser-glibc.out
```

Run it under strace in a pty and load pages. The browser needs a terminal; this
`expect` session is what was measured (clear the address with Backspace before each
URL — Address Mode keeps the previous URL, and Esc on an empty field leaves Address
Mode):

```tcl
set stty_init "rows 40 columns 120"; set env(TERM) xterm-256color
spawn $env(HOME)/strace-pkg/root/usr/bin/strace -f -tt -i -e trace=mmap,munmap,clone3,exit -o browser.strace ./build/browser-glibc.out
# then: g, 80×"\x7f", "example.com", "\r"; wait 8 s; the same for
# "https://en.wikipedia.org/wiki/BASIC" (wait 15 s) and "news.ycombinator.com" (wait 10 s),
# draining output continuously; q to quit.
```

Count **arena** maps: anonymous private `mmap`s whose instruction pointer is inside the
executable (aarch64 PIE, `0000aaaa…`), which is the arena grow path's raw syscall
(`emit_arena_map`, `src/target/linux_aarch64/code.rs`). ld.so and libc maps sit at
`0000ffff…` and are excluded:

```sh
P='\[0000aaaa[0-9a-f]+\] mmap\(NULL, [0-9]+, PROT_READ\|PROT_WRITE, MAP_PRIVATE\|MAP_ANONYMOUS, -1, 0\) = 0x'
grep -E "$P" browser.strace | sed -E 's/^([0-9]+) .*mmap\(NULL, ([0-9]+),.*/\1 \2/' \
  | awk '{c[$1]++; b[$1]+=$2} END {for (p in c) print p, c[p], b[p]}'
```

The classification was validated on `yamljson to-json samples/config.yaml`: its 12
executable-IP anonymous maps are matched one-for-one by 12 executable-IP `munmap`s at
`arena_destroy`.

- Observed (run 1, `strace -f -i -e trace=mmap,munmap`, 59.4 s, all three pages loaded
  and rendered — confirmed by replaying the captured pty stream through a terminal
  emulator):

  | thread | arena maps | bytes mapped | maps of exactly 4096 B |
  | --- | ---: | ---: | --- |
  | 55204 (Wikipedia fetch worker, see below) | 194,400 | 939,003,904 | |
  | 55167 (main thread) | 29,385 | 287,293,440 | |
  | 55254 (Hacker News fetch worker) | 14,328 | 81,563,648 | |
  | 55174 (example.com fetch worker) | 54 | 454,656 | |
  | **total** | **238,167** | **1,308,315,648** | 221,783 |

  All 238,167 maps come from one call site (`[0000aaaae1ff4550]`); 0 `munmap`s (a
  threaded program skips `arena_destroy`).

- Observed (run 4, same session plus `-tt` and `clone3,exit` tracing, 59.5 s). The
  heavier tracing slowed the worker so the Wikipedia load never finished, which pins
  the maps to the worker by timestamp:

  | thread | started | last map | arena maps | bytes | 4096 B maps |
  | --- | --- | --- | ---: | ---: | ---: |
  | 81673 main | 15:24:49 | 15:25:05 (entered the `isRunning` poll loop) | 253 | 2,338,816 | 225 |
  | 81675 example.com worker | 15:24:51.93 | exited 15:24:52.11 | 54 | 454,656 | 36 |
  | 81690 Wikipedia worker | 15:25:05.30 | 15:25:48.58 (process exit, still loading) | 22,908 | 227,999,744 | 17,629 |

  Worker 81690 mapped 3,800 / 4,544 / 3,920 / 2,299 / 2,028 / 852 / 1,851 / 2,382 / 1,232
  blocks in successive 5-second windows. The final screen still read
  `Loading https://en.wikipedia.org/wiki/BASIC ...`.

- Expected: a page load maps memory on the order of the page's live DOM. A 604 KB page
  should not need ~0.9 GB of fresh blocks, and a thread that frees what it allocates
  should reuse the freed blocks instead of mapping new ones.

Scaling across the three pages (run 1; HTML sizes from `curl -sSL -w %{size_download}`
on the same box):

| page | HTML bytes | worker arena maps | bytes mapped | mapped per HTML byte |
| --- | ---: | ---: | ---: | ---: |
| example.com | 559 | 54 | 454,656 | ~813 |
| news.ycombinator.com | 34,924 | 14,328 | 81,563,648 | ~2,335 |
| en.wikipedia.org/wiki/BASIC | 603,614 (+2 stylesheets) | 194,400 | 939,003,904 | ~1,556 |

Contrast cases measured in the same session (60 s of driven use, same method), which
bound the bug:

| example | arena maps | bytes mapped |
| --- | ---: | ---: |
| snake, hangman | 4 | 1,138,688 |
| life | 13 | 1,499,136 |
| ai_chat (terminal UI, `process::` children) | 24 | 1,220,608 |
| network-server (`thread::` workers, TCP, 11 clients) | 156 | 790,528 |
| yamljson to-json (small YAML) | 12 | 57,344 |

network-server also runs `thread::` workers and stays at 156 maps, so "a threaded
program" alone does not trigger it.

| Environment | arch / libc | Result |
| --- | --- | --- |
| box 2223 Kali | aarch64 glibc | fails ✗ (both runs above) |
| macOS | aarch64 | not measured (dtruss blocked by SIP) |
| Linux x86_64 / riscv64 / Windows | — | not measured |

## Root Cause

**Unknown.** The measurement localizes it to the work a page load does (fetch worker:
`http::read` → `dom::parse` → stylesheets → `dom::indexFields`; main thread:
`display::links`, `dom::fieldSpecs`, layout and render), and shows the growth is
roughly linear in HTML size. Hypotheses, most likely first:

1. **DOM values are never freed because `dom::Node` is recursive.** The spec excludes
   "recursive / non-flat composites (kept as pointer graphs, `type_is_flat` is false)"
   from scope-drop frees. (That spec sentence still names `type_is_flat`; the code now
   asks `type_is_memcpy_copyable`, so check which predicate the scope-drop path
   actually consults.) If the parser builds or copies intermediate `Node` values
   (a `WITH`, a collection append, a return copy) each copy would stay allocated for
   the life of the arena. That fits: linear in input size, present on both the worker
   (parse) and the main thread (render), and absent from every example without a
   recursive type.
   *Confirm/eliminate:* on the **main thread, no threads**, run `dom::parse` over a saved
   copy of the Wikipedia HTML twice in a loop and count arena maps (or peak RSS with
   `common::run_bounded_with_rss`). If the second parse maps as much as the first, the
   first parse's memory was never freed. Then repeat with a flat workload of the same
   volume, for example the same bytes through `strings::` operations, to check the
   allocator reuses correctly when values are freed.
2. **The memory is freed but not reused (allocator reuse failure).** Mixed-size
   transient churn is already documented as a weak spot (`.ai/codegen-invariants.md`,
   "Arena free-list goes quadratic on mixed-size transient churn"). Large chunks
   (> 2048 B) that miss their exact-size large bin skip the flush and grow directly.
   *Confirm/eliminate:* count `_mfb_arena_alloc` and `_mfb_arena_free` calls during
   one parse (gdb breakpoint counts on box 2223, or the plan-67-F debug perf rows on
   macOS). A free count near the alloc count, with maps still growing, points here.
   A tiny free count points to hypothesis 1.
3. **A worker-arena–specific reuse defect.** Frees on the worker do not land where the
   worker's next allocation looks.
   *Weakened by:* the main thread (no worker arena) also mapped 29,385 blocks / 287 MB
   in run 1, and network-server's workers stay at 156 maps.
   *Confirm/eliminate:* hypothesis 1's main-thread experiment. If the no-thread parse
   is small, come back here.
4. **`http::read` body accumulation.** A 604 KB body grown by repeated copying would
   map a geometric series of large blocks (the audio example shows that shape: sizes
   ×1.5 up to 43 MiB). *Weakened by:* 93% of the maps here are exactly 4 KiB, not a
   geometric size series. *Confirm/eliminate:* time-align the maps against the moment
   `http::read` returns (add `-e trace=read,recvfrom` or `-e trace=%net` to the strace
   run).

## Goal

- Parsing the same HTML twice on one thread maps no more new arena memory on the
  second parse than a small constant, i.e. the first parse's garbage is reused. The
  regression test asserts this with peak-RSS or map counts at 1× vs 2× iterations,
  following `tests/runtime/rt_scope_drop_leaks.rs`.
- Re-running the reproduction on box 2223, the Wikipedia worker's arena maps drop
  from ~194k / 939 MB to a small multiple of the page's live DOM size. Record the new
  numbers here.

### Non-goals (must NOT change)

- Arena layout, `ARENA_STATE_SIZE`, the block header, and allocator ABI
  (`_mfb_arena_alloc` / `_mfb_arena_free` register contract).
- The threaded-program shutdown rule (no `arena_destroy` when a worker may be live).
  Making the program unmap at exit would hide the symptom in this measurement and fix
  nothing: the growth happens during the session.
- The browser example's behavior and output. **Tempting wrong fix:** changing
  `examples/browser` to avoid the allocating shape (fewer copies, smaller pages,
  restarting workers). The example is a correct program exposing a runtime or codegen
  defect, and any MFBASIC program with a large recursive value would hit the same
  defect.
- Memory-safety guarantees: a fix that frees recursive values must not reintroduce
  double frees or use-after-free on shared subtrees. The ownership-tree invariant in
  `./mfb spec language memory-semantics` must still hold.

## Blast Radius

Not yet audited. The cause decides which search is right, so the audit is a Phase 1
task. Candidate classes to search once the cause is confirmed:

- Hypothesis 1: every recursive `TYPE`/`UNION` in `packages/**` and `examples/**`
  (`dom::Node`, `yaml`, `json::Json`-style trees), and every codegen site that skips
  a free because the value is not flat. `type_is_flat` no longer exists: plan-114-B
  split it into `type_is_memcpy_copyable` and `type_is_arena_transferable`
  (`src/codegen/collection/layout/builder_collection_layout.rs`), so search for
  `git grep -nE "type_is_(memcpy_copyable|arena_transferable)" src/`.
- Hypothesis 2: every allocation shape that mixes large, distinct sizes
  (string/collection growth inside loops).
- Hypothesis 3: every `ISOLATED` worker path (`src/codegen/runtime/thread/`).

Record a verdict per site here (fixed by this bug / latent, out of scope because … /
unaffected because …).

## Fix Design

Deferred until Phase 1 names the cause. If it is hypothesis 1, the fix belongs in
the ownership model for recursive values (free a recursive value at scope drop by
walking it, or keep such values in a disposable sub-arena), and the risk is
concentrated in shared-subtree aliasing. If it is hypothesis 2 or 3, the fix belongs
in `lower_arena_alloc` / `lower_arena_free` (`src/codegen/memory/arena/arena.rs`),
where the risk is the allocator's hot path and every `.ncodesum` golden it shifts.

## Phases

### Phase 1 — failing test + attribution + audit (no behavior change)

- [ ] Save the Wikipedia `BASIC` HTML as a test fixture, so the reproduction needs no
      network, and write a no-thread MFBASIC program that runs `dom::parse` over it
      N times.
- [ ] Measure maps or peak RSS at N and 2N on the main thread, and in a
      `thread::start` worker. Record both here.
- [ ] Run the confirm/eliminate step for each hypothesis above; strike the eliminated
      ones and cite the mechanism for the survivor (`file:symbol`).
- [ ] Add `tests/runtime/rt_dom_parse_arena_growth.rs` (1× vs 2× peak-RSS pattern from
      `rt_scope_drop_leaks.rs`, plus a flat-workload contrast pinned to stay flat).
      Confirm it fails today.
- [ ] Complete the blast-radius audit with a verdict per site.

Acceptance: the new test fails for the documented reason; the root cause is cited;
the audit has a verdict per site.
Commit: —

### Phase 2 — the fix

- [ ] Implement the fix at the confirmed cause's site.
- [ ] Apply it to every in-scope sibling site from the audit.

Acceptance: the Phase 1 test passes; the contrast case stays flat; nothing in
Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate any goldens / `.ncodesum` the fix shifts; confirm the delta is only
      the intended change.
- [ ] Full suite: `cargo test --no-fail-fast` and `scripts/test-accept.sh`.
- [ ] Re-run the reproduction above on box 2223 and record the new per-thread table.
      Also run it on macOS (plan-67-F perf rows) and one x86_64 Linux box.

Acceptance: full suite green; expected-output deltas are exactly the intended change;
the Wikipedia load's arena maps are bounded as stated in Goal.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_dom_parse_arena_growth.rs` (Phase 1), failing today.
- Runtime proof: the box-2223 strace reproduction above, before/after per-thread tables.
- Doc sync: `src/docs/spec/memory/04_arenas.md` Scope-Drop Frees (if recursive values
  gain frees) or the `arena_alloc` / `arena_free` sections (if the allocator changes);
  `./mfb spec language memory-semantics` if the ownership rule for recursive values
  changes.
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`, and the per-target
  byte-identity / execution gates in `.ai/testing-gates.md`.

## Open Decisions

- Where the regression test measures: peak RSS (portable, what
  `rt_scope_drop_leaks.rs` uses) vs. an arena map counter (exact but needs
  instrumentation). Recommended: peak RSS with a flat contrast pin. (§Goal)

## Summary

The risk is in attribution. The symptom is large and easy to reproduce, but four
plausible mechanisms remain, and they lead to fixes in different subsystems (the
ownership model for recursive values vs. the allocator hot path). Phase 1's
no-thread `dom::parse` experiment separates them cheaply. Nothing about arena layout,
threaded shutdown, or the browser example itself should change.
