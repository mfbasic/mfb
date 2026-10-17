# bug-1094: a builtin package's code and globals are linked once per importing package, even when every copy is identical

Last updated: 2026-10-17
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Other (binary size; possible correctness — see Hypothesis H1)

Status: Open
Regression Test: (to add) tests/runtime/rt_package_builtin_linked_once.rs — a package and its consumer both call `color`; the executable holds one copy of each `#color_*` function and global

A builtin package written in MFBASIC (`color`, `regex`, `astrings`, `http`, `encoding`,
`json`, `datetime`, `net`, …) is compiled into an executable **once per compilation
unit that imports it**: once for the program and once more for every manifest
package (`.mfp`) that imports it. The copies are byte-for-byte the same code apart
from symbol names and the global slots they read, yet all of them are emitted.
`examples/browser/app` imports three local packages. Its `browser.out` carries
`encoding` three times (the app's and two packages' copies), `astrings` and `color`
twice, and 175 builtin functions in two or three copies — 668,700 bytes of
duplicates in a 5,424,184-byte binary.

The builtin's **globals** are duplicated the same way, so each copy reads its own
copy of a table. For a mutable builtin global this means each copy also keeps its
own copy of the package's state — see H1.

**The single correct behavior a fix produces:** when a package's copy of a builtin
function (with every builtin function and global it reaches) is identical to the
program's copy or to another package's, the executable links **one** instance. It is
reached by every caller, and its globals have one storage slot. A copy that differs
in any way keeps today's per-package prefixed instance.

References:

- `./mfb spec architecture native-ir` — `merge_packages` merges every package into
  the one `IrProject` (`src/docs/spec/architecture/13_native-ir.md`).
- `./mfb spec architecture internal-naming` — the `#` sigil space and its two forms
  (`src/docs/spec/architecture/19_internal-naming.md`).
- `bugs/completed/bug-557-package-calling-crypto-does-not-link-into-an-executable.md`
  — the same prefixing broke natively-called crypto helpers. It records the
  rejected "stop prefixing every `#` name" fix and why it miscompiles.
- `bugs/completed/bug-673` (in `src/ir/package.rs:order_builtin_bindings_first`'s
  comment) — already states "A manifest package carries its own copy of every
  builtin it imports".
- `planning/optimizations.md` rows "Identical code folding" / "Function merging (IR-level)"
  (Level 4, unbuilt): the general optimizer form. This bug is the importer-side
  link-once rule, not that row; see Fix Design.
- Found while measuring `examples/browser/app/build/browser.out` (a `--debug`
  build: `nm` names exist only under `--debug`).

## Failing Reproduction

```sh
R=/tmp/bug1094; rm -rf $R; mkdir -p $R/pkg/src $R/app/src $R/app/packages
cat > $R/pkg/project.json <<'EOF'
{"name":"esc","version":"0.1.0","mfb":"1.0","kind":"package","description":"m",
 "sources":[{"root":"src","role":"package","include":["**/*.mfb"]}]}
EOF
cat > $R/pkg/src/lib.mfb <<'EOF'
IMPORT color

EXPORT FUNC lum(name AS String) AS Float
  RETURN color::luminance(color::fromName(name))
END FUNC
EOF
cat > $R/app/project.json <<'EOF'
{"name":"escapp","version":"0.1.0","mfb":"1.0","kind":"executable",
 "sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],
 "packages":[{"name":"esc","version":"=0.1.0","source":"file:packages/esc.mfp","direct":true,"requiredBy":[]}],
 "entry":"main","targets":["native"]}
EOF
cat > $R/app/src/main.mfb <<'EOF'
IMPORT color
IMPORT esc
IMPORT io

FUNC main() AS Integer
  io::print(toString(esc::lum("red")) & " " & toString(color::luminance(color::fromName("red"))))
  RETURN 0
END FUNC
EOF
./target/release/mfb build $R/pkg && cp $R/pkg/esc.mfp $R/app/packages/
./target/release/mfb build --debug $R/app
nm $R/app/build/escapp.out | grep -c '_mfb_ifn_color_'          # the program's copy
nm $R/app/build/escapp.out | grep -c '_2Eesc_2E_23color_'       # the package's copy
```

- Observed (2026-10-17, macos-aarch64, `target/release/mfb` built 2026-10-17 07:53):
  prints `0.21 0.21`; **45** `_mfb_ifn_color_*` functions and **45**
  `_mfb_fn_<id>.esc.#color_*` functions, e.g. both
  `_mfb_ifn_color_5FnameTable` and `_mfb_fn_9f890208b71c37a6_2Eesc_2E_23color_5FnameTable`.
- Expected: `0.21 0.21`; 45 `color` functions in total, 0 package-prefixed.

The copies are identical. The Appendix probe (it runs
`otool -tv` and treats a call to the other copy of the same builtin function, an
`adrp` page and an `x19+imm` global offset as equal) reports
`duplicated 45 identical modulo callee-prefix + global offset 45`. Four of them
(`color_toLinear`, `fromLinear`, `fromName`, `nameOf`) read **different global
slots** (`toLinear`: `add x9, x19, #0xf78` vs `#0xf60` in `browser.out`), so the
globals are duplicated too.

Size, without `--debug` (`ls -l`): 364,008 bytes as above. Drop the program's own
`color` use (`main` calls only `esc::lum`) and it is 231,912 — a 132,096-byte
second copy of `color`.

Contrast cases (work today, must keep working):

- The program alone imports `color` (no package uses it): one copy, 45 functions.
- Only the package imports `color`: one prefixed copy (`ifn 0 pkg 45`, measured as
  above with `main` calling only `esc::lum`).
- bug-557's composition (a package calls `crypto`, the program does not): links via
  `define_natively_called_helpers` (`tests/runtime/rt_package_calls_software_curve_builtin.rs`).

At scale (`examples/browser/app/build/browser.out`, `--debug`, `nm -n` address gaps +
`otool -tv`): 175 duplicated builtin functions, 668,700 duplicate bytes. 160 groups
are identical apart from callee prefix (649,528 bytes). The other 15 (`color_*`)
differ only by global slot.

## Root Cause

Three steps, each deliberate on its own:

1. **A package build injects the builtin's whole source.**
   `src/resolver/mod.rs:augment_project` (via `registry().augment_project` and the
   late passes) parses each imported builtin's companion source into the unit being
   compiled. A package is such a unit, so its `.mfp` IR carries its own compiled
   copy of every builtin function and global it imports
   (`.ai/resources-packages.md`, "Editing an embedded builtin .mfb ripples…").
2. **Merging renames everything a package carries.**
   `src/ir/package.rs:prefix_package_symbols` renames every function and binding of
   a decoded package to `<identity>.<package>.<name>`, including the injected
   `#color_*` sigil names. It also scopes the package's private types
   (`scope_private_types`), and that includes the builtin's private types
   (`browser.out` holds `_mfb_thread_copy_List_OF_display__regex_Node_45a270dbeb9b8bad`).
3. **The merge removes duplicates by exact name.**
   `src/ir/package.rs:merge_package` → `push_unique(.., |a, b| a.name == b.name)`.
   After step 2 no package name can equal the program's `#color_x` or another
   package's, so every copy survives. Nothing later removes it: no stage
   between `merge_packages` and emission deletes a function (`grep -rnE
   'functions\.(remove|swap_remove|drain|retain)|\.functions = ' src` hits tests only).

Why the prefix is not simply dropped for builtin names — bug-557 tried it ("Two
fixes that do not work") and it **miscompiled**:

- The `#` space is not only builtin code. `src/internal_name.rs:mangle_private`
  puts file-scoped PRIVATE user names there (`#<filehash>$name`), and generated names
  live there too. Their bodies differ between packages, and name-dedupe collapsed
  them (`closure capture index 0 appears in a function that is not a closure body`).
- **The same builtin name can legitimately carry a different body**, so name
  equality never proves identity:
  - `src/codegen/registry/mod.rs:HelperGate::WhenNeitherCalledNorNamed` and its
    pair inject one of two chunks per unit, and "the two chunks may define the
    same functions". `WhenNotConstructed` and `WhenUsedOrConstructed` are the same
    kind of pair.
  - The `--debug` split (`NormalBuildOnly` / `DebugBuildOnly`) is chosen when the
    package is built.
  - A `.mfp` built by another compiler version carries that version's bodies.
  - `$ownN` ownership-variant suffixes are numbered per unit, so `#x$own4` in two
    units need not be the same variant.

So the duplication is the safe default, not an accident. What's missing is any
check that **merges the copies that are identical**.

### Hypothesis H1 — split mutable builtin state (unverified)

Because each copy has its own globals, a **mutable** builtin global exists once per
importing unit. Today that covers `regex` (`__regex_cache*`, `__regex_steps`,
`__regex_call*` in `src/codegen/builtins/regex/helper_compile.rs` and `helper_steps.rs`)
and about 189 `canvas` globals (`grep -rnE '^(EXPORT |PRIVATE )?MUT '
src/codegen/builtins/canvas | wc -l`). Examples are `__CANVAS_AUTO_*`
(`helper_autopaint.rs`), `__CANVAS_HIT_*` (`helper_hit.rs`), `__CANVAS_SCENE_IDS`
(`func_present.rs`) and the glyph cache.

- `regex`: not observable. Its counters are reset per search or call, and the cache
  only affects speed.
- `canvas`: possibly observable. Suppose a package calls `canvas::present` and the
  program calls `canvas::pollMouse`. The hit list that `present`'s copy records sits
  in the package's globals, and the program's `pollMouse` reads its own (empty)
  copy, so `overID` would never resolve. **Not reproduced.** It needs a
  `Mode.Canvas` program. Confirm or eliminate it in Phase 1 with a canvas-harness
  test (`tests/canvas/`) where a package presents and the program polls.
  If it reproduces, this bug becomes Severity HIGH / Class Correctness, and the fix
  below is what resolves it. If it doesn't, record why here.

## Goal

- In the reproduction, the `--debug` executable has 45 `color` functions and 0
  `<id>.esc.#color_*` functions. It prints `0.21 0.21`, and its release size equals
  the `color`-in-program-only build plus the program's own code.
- `browser.out` (`--debug`) has no builtin function in more than one copy whose
  group is identical (Appendix probe: 175 duplicated groups → 0
  identical groups left).
- A builtin global shared by merged copies has exactly one storage slot, and its
  initializer runs once.
- Where copies differ (version skew, paired chunks, `--debug` split, `$ownN`
  numbering), the build links both as today and behaves exactly as today.

### Non-goals (must NOT change)

- **No `.mfp` format change.** Packages keep carrying their builtin copies, and an
  old `.mfp` still links (merging falls back to prefixing for it).
- **Do not merge by name.** Merging every `#` name or every `is_builtin_internal`
  name without comparing bodies is bug-557's rejected fix, and it miscompiles. Only
  a content match is allowed to merge.
- **Never merge a `mangle_private` name (`#<hash>$name`) or a lambda.**
  `src/internal_name.rs:is_builtin_internal` already separates these from builtin
  names. Use it, don't re-derive it.
- **Not the optimizer's identical-code-folding row.** That row stays a separate,
  general Level-4 item. This rule runs at every `-O` level, including `-O0`.
- Package-owned (non-builtin) functions and types keep their identity prefix.
- bug-557's natively-called aliasing keeps working when copies are *not* merged.

## Blast Radius

Found by reading the merge path (`src/target/shared/nir/lower.rs:merge_packages`)
and grepping for each name form:

- `src/ir/package.rs:prefix_package_symbols` — renames functions and bindings;
  **fixed by this bug** (skips names chosen for merging).
- `src/ir/package.rs:scope_private_types` / `qualify_package_types` — builtin
  private types (`#regex_Node`) are scoped per package, and any function whose
  signature or body names them differs by type name. **Fixed by this bug**: a
  merged group's private builtin types merge with it, or the group can never match.
- `src/ir/package.rs:merge_package` (`push_unique` on functions, bindings, types) —
  **fixed by this bug**: receives already-unprefixed matching entries, and the
  name match then keeps one.
- `src/ir/package.rs:order_builtin_bindings_first` (bug-673) — must still place a
  merged bare `#X` global, initialized once, before any binding that reads it.
  **Fixed / re-verified by this bug.**
- `src/target/shared/nir/lower.rs:define_natively_called_helpers` (bug-557) — when
  the crypto group merges, the bare name is already defined and no alias is
  needed. When it doesn't merge, the alias path is unchanged. **Re-verified by
  this bug** (`rt_package_calls_software_curve_builtin` stays green).
- Thread copy/transfer helpers generated per type (`_mfb_thread_copy_List_OF_…`) —
  follow the type names, so they merge when the types do. **Fixed by this bug**
  (verify in `browser.out`).
- `src/docs/spec/tooling/10_debugger.md` symbol table (`_mfb_ifn_` = `MB`,
  `_mfb_fn_` = `MU`) — a merged builtin function becomes `_mfb_ifn_…` and is shown
  as the builtin package's. **Spec update in this bug.**
- Goldens: package-consuming fixtures' `.ir`/`.nir`/`.ncode`/`.ncodesum` shift
  wherever a builtin group merges. **Expected shift.** Every changed fixture must
  only lose prefixed builtin copies (Phase 3).
- Function merging inside one unit (two different user functions with identical
  bodies) — **unaffected / out of scope.** That is the optimizer's
  "Function merging" row.
- `.mfp` writer (`src/binary_repr/writer.rs`) — **unaffected** (no format change).

## Fix Design

**Where: the importer** (`merge_packages`, before `prefix_package_symbols`
renames anything). This is the stage that creates the duplication, so it should
also decide which copies are the same. It's a link-once rule, like a C++ linker
keeping one instance of an inline function, not an optional optimization:

- It must hold at `-O0`.
- If H1 reproduces, it is a behavior fix, which the `-O` dial is not allowed to
  carry (`src/optimizer/mod.rs`: "`-O0`..`-O5` change the emitted code, never the
  observable results").
- Running it first also means NIR lowering, the optimizer, regalloc and emission
  each compile one copy instead of N.

**Shape:**

1. For each decoded package, before prefixing, take the candidates: functions,
   bindings and private types whose names pass `is_builtin_internal`.
2. Pair each candidate with the same-named entry already in the merged project
   (the program's own copy, or an earlier package's merged copy).
3. A pair *matches* when the two are structurally equal. Builtin names that both
   sides use count as equal, and `$ownN` suffixes are resolved by content, never
   by number. `IrFunction` derives only `Clone` (`src/ir/types.rs`), so this needs
   a structural comparison, either a written-out equality or a canonical encoding.
   For a binding, compare its type, mutability and initializer.
4. Shrink the set to a **fixed point**: drop any pair that reaches (calls, reads,
   or names as a type) a builtin entry whose own pair didn't match. Repeat until
   nothing changes. A group is merged only if everything it reaches merges too.
5. Matched entries are left unprefixed, so `push_unique` keeps one. The
   package's references to them already spell the bare name. Everything else is
   prefixed exactly as today.

**Correctness risk concentrates in** (a) the equality: a false "equal" is a silent
miscompile, so compare conservatively, and any unrecognized node means "not equal";
and (b) global merging: one slot and one initializer run, still ordered by
`order_builtin_bindings_first`.

**Rejected alternatives:**

- *Drop the prefix for every builtin name* — bug-557 measured it miscompiling, and
  it's unsound under paired chunks, the `--debug` split and version skew.
- *Stop storing builtin bodies in `.mfp` and re-inject the program's builtin at
  link time* — changes the format. It also re-typechecks package code against a
  different builtin version, and evaluates helper gates against the program's
  usage instead of the package's.
- *Leave it to the optimizer's ICF / function-merging rows* — they run after every
  copy is compiled, are gated off at `-O1`, and can't merge two mutable globals
  without changing behavior.

**Expected output shift:** the goldens of fixtures that consume a package which
shares a builtin with the program or with another package. The diff should be
removals of `<id>.<pkg>.#…` entries plus the matching symbol/offset moves.
Nothing else.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/runtime/rt_package_builtin_linked_once.rs`, modelled on
      `tests/runtime/rt_package_calls_software_curve_builtin.rs` (package + program
      built in a temp root, `MFB_HOME` isolated). It builds the reproduction above with
      `--debug` and asserts the output `0.21 0.21` and zero
      `_2E<pkg>_2E_23color_` symbols. Confirm it fails (45 today).
- [ ] Add a contrast case in the same file: a package built with a differing
      builtin body must keep its prefixed copy. Paired-gate chunks or `--debug` vs
      normal are the easiest ways to make the bodies differ. The case must run
      correctly before and after the fix.
- [ ] Settle H1 with a canvas-harness test (package presents, program polls).
      Record confirmed/eliminated here, and adjust Severity/Class.
- [ ] Re-verify each Blast Radius verdict above against the tree at that commit.

Acceptance: the link-once test fails for the documented reason (prefixed copies
present). The contrast test passes. H1 has a recorded verdict.
Commit: —

### Phase 2 — the fix

- [ ] Structural equality for IR functions, bindings and types, conservative
      (`src/ir/` beside `package.rs`).
- [ ] The candidate/match/fixed-point pass in `merge_packages` /
      `prefix_package_symbols` (`src/target/shared/nir/lower.rs`,
      `src/ir/package.rs`). Only `is_builtin_internal` names, and private builtin
      types with them.
- [ ] Keep bug-557's `define_natively_called_helpers` correct for both the merged
      and the unmerged case.
- [ ] Update `./mfb spec architecture native-ir` (the merge rule),
      `./mfb spec architecture internal-naming` (which sigil names may merge, and
      when), and `./mfb spec tooling debugger` (merged builtin symbols are `MB`).
      Update `.ai/resources-packages.md` for the new rule.

Acceptance: Phase 1 tests pass; `rt_package_calls_software_curve_builtin` passes;
nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Run the full suite before touching any golden. For each changed golden,
      inspect one fixture's diff and confirm it only removes package-prefixed
      builtin copies, with the symbol/offset moves that follow. Then regenerate.
- [ ] Rebuild `examples/browser` (`--debug`). Re-run the duplicate-group check
      (Appendix probe). Expect 0 identical groups left, and record
      the size before and after, both `--debug` and release.
- [ ] Re-run the reproduction end to end.

Acceptance: full suite green; golden deltas are exactly the intended removals;
browser numbers recorded here.
Commit: —

## Appendix — duplicate-group probe

The one-off check behind the "identical modulo callee-prefix + global offset" counts
(run as `python3 cmp.py <binary built with --debug>`):

```python
import re,sys,collections,subprocess
asm=subprocess.run(['otool','-tv',sys.argv[1]],capture_output=True,text=True).stdout
def key(n):
    if n.startswith('_mfb_ifn_'): return n[9:]
    m=re.search(r'_2E_23(.*)',n); return m.group(1) if m else None
cur=None;b={};gl={}
for l in asm.splitlines():
    if l.startswith('_') and l.endswith(':'): cur=l[:-1]; b[cur]=[]; gl[cur]=[]; continue
    p=l.split('\t')
    if not cur or len(p)<3: continue
    a=p[2]; m=re.search(r'(_mfb_\w+)',a)
    if m and key(m.group(1)): a='K:'+key(m.group(1))
    a=re.sub(r'0x1[0-9a-f]{8}','ADDR',a)
    if p[1]=='adrp': a=a.split(',')[0]+',PAGE'
    m2=re.match(r'(x\d+), x19, #(0x[0-9a-f]+)',a)
    if p[1]=='add' and m2: gl[cur].append(m2.group(2)); a=m2.group(1)+',x19,GLOBAL'
    b[cur].append(p[1]+' '+a)
g=collections.defaultdict(list)
for n in b:
    k=key(n)
    if k: g[k].append(n)
same=sum(1 for k,ns in g.items() if len(ns)>1 and len({tuple(b[n]) for n in ns})==1)
multi=[k for k,ns in g.items() if len(ns)>1]
print('duplicated',len(multi),'identical modulo callee-prefix + global offset',same)
glob=[k for k in multi if len({tuple(gl[n]) for n in g[k]})>1]
print('groups reading different global slots',len(glob),glob[:6])
```
