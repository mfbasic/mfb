# bug-791: audit omits fallible TCP and UDP calls

Last updated: 2026-10-02
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness
Status: Open
Regression Test: add collector regressions in `src/audit/collect/source.rs` and an audit CLI acceptance fixture.

`mfb audit` reports a function containing `tcp::listen` and `udp::bind` as
non-fallible and emits no call rows. A fix must report these raising calls and
propagate their fallibility through callers, while preserving handled-TRAP and
total-call behavior. Filed during the tooling-spec review; no code fix is part
of that documentation change.

References:

- `src/codegen/builtins/tcp/func_listen.rs:register` declares
  `ErrAddressInvalid`, `ErrNetworkFailed`, and `ErrOutOfMemory`.
- `src/codegen/builtins/udp/func_bind.rs:register` declares the UDP bind contract.
- `src/docs/spec/tooling/04_audit-format.md` describes the as-is table.

## Failing Reproduction

From the repository root:

```sh
python3 - <<'PY'
from pathlib import Path
import json, subprocess, tempfile
p = Path(tempfile.mkdtemp(prefix='mfb-audit-transport-'))
(p / 'src').mkdir()
(p / 'project.json').write_text(json.dumps(dict(
    name='probe', version='1.0.0', mfb='1.0', kind='executable',
    sources=[{'root': 'src'}])))
(p / 'src/main.mfb').write_text('''IMPORT tcp
IMPORT udp
FUNC main AS Integer
  RES listener = tcp::listen("127.0.0.1", 0)
  RES socket = udp::bind("127.0.0.1", 0)
  RETURN 0
END FUNC
''')
r = subprocess.run(['./target/release/mfb', 'audit', '--format', 'json', str(p)],
                   capture_output=True, text=True)
assert r.returncode == 0, r.stderr
flow = json.loads(r.stdout)['sourceFlow']
print(json.dumps(flow, indent=2))
assert flow[0]['fallible'] is True
assert {c['callee'] for c in flow[0]['calls']} == {'tcp.listen', 'udp.bind'}
PY
```

Observed on macOS AArch64: audit exits 0, reports `main` with
`"fallible": false, "trap": null, "calls": []`; the assertion fails.
The same report correctly includes both network permission sites and both
resource acquisitions. No network operation is executed by this reproduction.
Other hosts have not been run; the collector is platform-independent.

## Root Cause

`src/audit/collect/source.rs:is_fallible_call` includes `net`, `tls`, and
`http` in its namespace table but omits `tcp` and `udp`.
`is_fallible_builtin` also contains no TCP/UDP members. These registered native
builtins are not declarations in the user-function fixpoint, so that final
fallback cannot recover their fallibility. `builtin_capability` and
`resource_producer` have transport entries, explaining the contrasting output.

Evidence: `sed -n '970,1165p' src/audit/collect/source.rs` and the reproduction.

## Goal

Report each raising TCP/UDP call, its enclosing propagation context, and the
resulting caller fallibility in both text and JSON audits.

### Non-goals (must NOT change)

Do not change runtime socket behavior, ABI, JSON schema, or permission/resource
rows. Do not hide the reproduction by removing socket calls or weakening an
expected fallibility assertion. Do not mark total operations fallible merely
because they share a package with raising operations.

## Blast Radius

Search: `rg -n 'is_fallible_call|is_fallible_builtin|builtin_capability|resource_producer' src/audit`.

- `source.rs:walk_expression` — affected: missing call rows and propagation edges.
- `source.rs:fallible_functions` / `block_escapes` — affected: transitive caller verdicts.
- `source.rs:builtin_capability` / `resource_producer` — unaffected for this repro;
  retain their existing rows as regression controls.
- `src/audit/json.rs:source_flow` and `src/audit/text.rs:render` — affected consumers
  of the shared collector, with no renderer change expected.
- The compiler's separate IR fallibility analysis is not the source of this
  audit result; runtime/lowering changes are out of scope.

## Fix Design

Audit the TCP/UDP descriptors and raise sites, then extend the collector's
fallibility census for their raising operations. Preserve its conservative
call-context analysis. Verify nested expression, caller, and inline TRAP cases.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add collector and CLI regressions for the reproduction.
- [ ] Audit all TCP/UDP members and select total-operation contrast cases.
- [ ] Add transitive-caller and handled-TRAP regression cases.

Acceptance: regressions fail for the omitted transport calls.
Commit: —

### Phase 2 — the fix

- [ ] Correct the fallibility census and update the tooling spec's as-is limitation.

Acceptance: regressions pass; permission/resource rows and total calls stay correct.
Commit: —

### Phase 3 — expected outputs and validation

- [ ] Run the full suite before any golden correction; document which exact
      lines are disproved by the descriptor/raise-site evidence.
- [ ] Correct only affected audit expectations, then run the full suite again.
- [ ] Rebuild release and rerun the CLI reproduction.

Acceptance: correct CLI report and full-suite success with explained deltas.
Commit: —

## Validation Plan

Use collector tests plus the CLI reproduction for observable proof; run
`cargo test` and the repository's applicable acceptance gates. Update
`src/docs/spec/tooling/04_audit-format.md` when the code fix lands.
