# bug-785: Set, MapEntry, and ThreadWorker ABI hashes ignore component shapes

Last updated: 2026-10-03
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (ABI compatibility)
Status: Fixed
Regression Test: `src/binary_repr/tests/reader_tests.rs:abi_serializer_hashes_set_mapentry_and_threadworker_structurally` (beside bug-277's STATE test)

The correct behavior is that composite signature hashes change when a component's
structure changes and remain stable under unrelated type-table renumbering.
`AbiSerializer::serialize_type_inner` instead hashes kinds 9, 10, and 13 by their
interned names containing numeric type IDs. An incompatible record change is
invisible to these function hashes; an unrelated private declaration changes them.

Found while checking `mfb spec package` against the implementation. This report
covers the remaining sibling cases of completed bug-277, whose STATE fix did not
cover MapEntry/ThreadWorker; Set also uses the same fallback.

## Failing reproduction

From the repository root, run `cargo build --release`, then save and run the
following as `/tmp/probe-package-abi.py` with `python3 /tmp/probe-package-abi.py`.
It builds real unsigned packages and reads their emitted ABI_INDEX bytes.

```python
from pathlib import Path
import tempfile,json,subprocess,struct
binary=str(Path('target/release/mfb').resolve())
root=Path(tempfile.mkdtemp(prefix='mfb-package-abi-'))
def hashes(path):
 b=path.read_bytes();p=20
 for _ in range(11): n=struct.unpack_from('<I',b,p)[0];p+=4+n
 p+=32;n=struct.unpack_from('<Q',b,p)[0];p+=8
 sig=struct.unpack_from('<I',b,p+2)[0];p+=6+sig;b=b[p:p+n]
 sections={}
 for i in range(struct.unpack_from('<I',b,12)[0]):
  ident,_,_,off,n=struct.unpack_from('<HHIQQ',b,16+i*24);sections[ident]=b[off:off+n]
 s=sections[2];p=4;strings=[]
 for _ in range(struct.unpack_from('<I',s)[0]):
  n=struct.unpack_from('<I',s,p)[0];p+=4;strings.append(s[p:p+n].decode());p+=n
 a=sections[15];p=8;out={}
 for _ in range(struct.unpack_from('<I',a,4)[0]):
  name,kind=struct.unpack_from('<IH',a,p);p+=6;out[strings[name]]=a[p:p+32].hex();p+=32
 return out
for variant,field,prefix in [('base','Integer',''),('shape','String',''),('renumber','Integer','TYPE Unrelated\n extra AS Integer\nEND TYPE\n')]:
 d=root/variant;(d/'src').mkdir(parents=True)
 (d/'project.json').write_text(json.dumps({'name':'abiprobe','version':'1.0.0','mfb':'1.0','kind':'package','description':'ABI probe','sources':[{'root':'src','role':'lib','include':['**/*.mfb']}],'targets':['native']}))
 (d/'src/main.mfb').write_text(prefix+f'EXPORT TYPE Item\n value AS {field}\nEND TYPE\nEXPORT FUNC inspectSet(items AS Set OF Item) AS Integer\n RETURN 0\nEND FUNC\nEXPORT FUNC inspectList(items AS List OF Item) AS Integer\n RETURN 0\nEND FUNC\nEXPORT FUNC inspectEntry(item AS MapEntry OF String TO Item) AS Integer\n RETURN 0\nEND FUNC\nEXPORT FUNC inspectWorker(worker AS ThreadWorker OF Item TO Integer) AS Integer\n RETURN 0\nEND FUNC\n')
 r=subprocess.run([binary,'build','--unsigned',str(d)],capture_output=True,text=True)
 if r.returncode: print(r.stdout,r.stderr);raise SystemExit(r.returncode)
 print(variant,json.dumps(hashes(d/'abiprobe.mfp'),sort_keys=True))
print(root)
```

Observed on macOS AArch64 with the release compiler:

| Export | Integer → String record field | Unrelated type inserted first |
| --- | --- | --- |
| inspectSet | hash unchanged (wrong) | hash changes (wrong) |
| inspectEntry | hash unchanged (wrong) | hash changes (wrong) |
| inspectWorker | hash unchanged (wrong) | hash changes (wrong) |
| inspectList | hash changes (correct) | hash unchanged (correct) |
| Item | hash changes (correct) | hash unchanged (correct) |

The base and changed-shape `inspectSet` hash is
`98769fa688b6a82a6595228bddc9dbf2c633d23efec6ccddba1a11ada4e73d08`;
the unrelated-renumber hash is
`36cfc7771fe56b98f4c660445cbd1961204951fcd77cf7b2e41aa6caff4f0bd3`.
Expected: the first comparison differs and the second agrees, for all composites.
The direct Item ABI entry changes correctly; this probe establishes the function
hash defect, not that every resolver scenario bypasses the separate type pin.

## Root cause

`src/binary_repr/sections.rs:AbiSerializer::serialize_type_inner` has structural
arms for kinds 4/5/6/7/8/11 and foreign references, but not kinds 9/10/13. The
fallback hashes `opaque` plus `entry.name`. `map_entry_type`, `thread_worker_type`,
and `set_type` construct those names using package-local numeric component IDs.
The component payload is never traversed by the fallback.

## Goal and non-goals

Hash every component structurally: MapEntry key/value; ThreadWorker message,
output, and optional resource; Set element. Preserve recursion/back-reference
handling. Do not change wire type IDs, payload layouts, or unrelated type hashes.
Do not hide the defect by changing the fixture to primitive-only types or by
sorting away just one renumbering case.

## Blast radius

Search: `rg -n 'serialize_type_inner|function_sig_hash|type_sig_hash|set_type|map_entry_type|thread_worker_type' src/binary_repr`.

- `sections.rs:serialize_type_inner`: all three missing arms are in scope.
- `sections.rs:function_sig_hash` and `type_sig_hash`: both traverse the same
  serializer; nested occurrences in exported records/functions are affected.
- `reader.rs:validate_abi_index`: recomputes hashes with the same serializer;
  existing affected packages need a deliberate rebuild/compatibility decision.
- `cli/pkg.rs:check_abi`, dependency used-symbol hashes, and registry ABI maps:
  consume these hashes; include them in end-to-end validation.
- List/Map/Result/Thread/STATE structural arms: controls, unchanged by the fix.
- Foreign references: hash the owner's ABI identity; keep their existing policy.

## Fix design

Add explicit structural arms with distinct tags and recursive component hashing.
Mirror Thread's optional resource component for ThreadWorker. Determine how stale
published hashes are rejected/rebuilt; do not silently accept either hash as an
ABI match. Document the chosen compatibility policy before regenerating output.

## Phases

### Phase 1 — failing tests and audit

- [ ] Add shape-sensitivity and renumber-invariance tests for kinds 9/10/13,
      including nested composites and ThreadWorker's resource component.
- [ ] Confirm RED and audit all serializer consumers.

Acceptance: tests fail for the mechanism above; controls pass.
Commit: —

### Phase 2 — fix

- [ ] Add structural serialization and settle stale-package handling.
- [ ] Update the package metadata spec to describe the new behavior.

Acceptance: regression tests pass without changing wire layouts or controls.
Commit: —

### Phase 3 — validation

- [ ] Run the full suite before correcting any proven-obsolete golden lines.
- [ ] Inspect expected hash changes and run the package acceptance tests.
- [ ] Re-run the above emitted-package reproduction and resolver/check-ABI cases.

Acceptance: shape changes differ, renumbering agrees, and all required gates pass.
Commit: —

## Validation plan

Use `cargo test --bin mfb binary_repr`, the full project suite, and
`scripts/test-accept.sh target/debug/mfb target/accept-actual` during the fix.
The emitted-package reproduction is the end-to-end evidence. Update
`src/docs/spec/package/03_metadata-encoding.md` when the implementation changes.

## Resolution (2026-10-03)

`serialize_type_inner` has structural arms for kind 9 (`"mapentry"`: key and
value), kind 10 (`"threadworker"`: message, output, and the optional resource
plane, mirroring `"thread"`) and kind 13 (`"set"`: element). Every defined kind
is now structural, so the `"opaque"` fallback is reachable only for an unknown
kind in a decoded table.

Compatibility policy (the spec's existing rule, now stated for this case):
`abiFormatVersion` gates the ABI_INDEX wire encoding, not what the hash covers. A
package built before this fix with one of these composites in an exported
signature carries hashes `validate_abi_index` no longer reproduces, so it is
refused until rebuilt. Neither hash is silently accepted. The committed `.mfp`
fixtures are regenerated at the bug-staging batch gate.

Proof: the new test fails on main ("Set sig hash must not track type-table
position") and passes with the fix, as do the 5 other `abi_serializer` tests.
