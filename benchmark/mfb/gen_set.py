#!/usr/bin/env python3
"""Generator for benchmark/mfb/src/setops.mfb.

Emits the 6-way Set benchmark matrix, mirroring gen_list.py. The old bundled
`set build` / `set ops` rows are split into one function per Set op, each
replicated across six groups differing only in element type and container:

    set (Fixed)          Set OF Integer, plain MUT local
    set (Dynamic)        Set OF String,  plain MUT local
    set (Record-Fixed)   Set OF Integer in a record field (fields before + after)
    set (Record-Dynamic) Set OF String  in a record field
    set (State-Fixed)    Set OF Integer in a File STATE field
    set (State-Dynamic)  Set OF String  in a File STATE field

Regenerate with:  python3 benchmark/mfb/gen_set.py > benchmark/mfb/src/setops.mfb
"""


class Container:
    def __init__(self, kind):
        self.kind = kind  # 'plain' | 'record' | 'state'

    def get(self):
        return {'plain': 'nums', 'record': 'rec.xs', 'state': 'f.state.xs'}[self.kind]

    def put(self, expr):
        if self.kind == 'plain':
            return f'nums = {expr}'
        if self.kind == 'record':
            return f'rec = WITH rec {{ xs := {expr} }}'
        return f'f.state.xs = {expr}'

    def ro_setup(self, rec_ty, build):
        if self.kind == 'plain':
            return [f'LET base = {build}']
        if self.kind == 'record':
            return [f'LET rec AS {rec_ty} = {rec_ty}[1, {build}, 2]']
        return [f'f.state.xs = {build}']

    def ro_list(self):
        return {'plain': 'base', 'record': 'rec.xs', 'state': 'f.state.xs'}[self.kind]

    def read_typed(self, elem_ty):
        if self.kind == 'plain':
            return (None, 'nums')
        return (f'LET cur AS Set OF {elem_ty} = {self.get()}', 'cur')

    def accum_reset(self, rec_ty, elem_ty):
        empty = f'Set OF {elem_ty} {{ }}'
        if self.kind == 'plain':
            return [f'MUT nums AS Set OF {elem_ty} = {empty}']
        if self.kind == 'record':
            return [f'MUT rec AS {rec_ty} = {rec_ty}[1, {empty}, 2]']
        return [f'f.state.xs = {empty}']

    def mut_setup(self, rec_ty, elem_ty, build):
        if self.kind == 'plain':
            return [f'MUT nums AS Set OF {elem_ty} = {build}']
        if self.kind == 'record':
            return [f'MUT rec AS {rec_ty} = {rec_ty}[1, {build}, 2]']
        return [f'f.state.xs = {build}']


# Integer sets are cheap to churn, so they run at full size. String sets
# allocate many short-lived String temporaries; the runtime arena's free list
# degrades quadratically under that churn (process-global and cumulative across
# the run loop — see liststr_reshape in list.mfb / README), so the String groups
# run at reduced sizes as a coverage smoke-test, not a throughput benchmark.
# Set-*producing* ops (toSet + the binary algebra) allocate a full result set
# per iteration; at large size the arena's mixed-transient-churn free list goes
# quadratic (cumulative — see liststr_reshape), so they run at a dedicated small
# `alg` size as a coverage smoke-test. Bool/list-producing ops (contains,
# toList, isSubset/Superset/Disjoint) and mutation (add/remove) stay larger.
INT = dict(
    ty='Integer', rec='RecSetFixed',
    build='buildSetRange', shift='buildSetShift',
    add_val='i', member='i',
    add_n=300, rem_n=300,
    ro_n=1000, ro_sh=500,
    alg_n=300, alg_sh=150,
    k_contains=100, k_tolist=200, k_pred=300, k_alg=20,
)
STR = dict(
    ty='String', rec='RecSetDyn',
    build='buildStrSetRange', shift='buildStrSetShift',
    add_val='"s" & toString(i)', member='"s" & toString(i)',
    add_n=400, rem_n=400,
    ro_n=200, ro_sh=100,
    alg_n=100, alg_sh=50,
    k_contains=50, k_tolist=100, k_pred=60, k_alg=15,
)


def _extract(c, E):
    if c.kind == 'plain':
        return []
    return [f'LET base AS Set OF {E["ty"]} = {c.ro_list()}']


def ro(c, E, build, k, inner, checksum='acc', extra_outside=None):
    outside = c.ro_setup(E['rec'], build)
    if extra_outside:
        outside += extra_outside
    timed = _extract(c, E)
    timed += ['MUT acc AS Integer = 0', f'FOR k = 0 TO {k} - 1']
    for ln in inner('base'):
        timed.append('  ' + ln)
    timed.append('NEXT')
    timed.append(f'checksum = {checksum}')
    return outside, [], timed


OPS = {}


def op(name):
    def deco(fn):
        OPS[name] = fn
        return fn
    return deco


def _bld(E, n):
    return f'{E["build"]}({n})'


def _oth(E, n, sh):
    return f'{E["shift"]}({n}, {sh})'


@op('add')
def _(c, E):
    G = c.get()
    timed = c.accum_reset(E['rec'], E['ty'])
    timed += [
        f'FOR i = 0 TO {E["add_n"]} - 1',
        '  ' + c.put(f'collections::add({G}, {E["add_val"]})'),
        'NEXT',
        f'checksum = len({G})',
    ]
    return [], [], timed


@op('remove')
def _(c, E):
    G = c.get()
    rd_line, rd = c.read_typed(E['ty'])
    outside = [f'LET base = {E["build"]}({E["rem_n"]})']
    timed = c.mut_setup(E['rec'], E['ty'], 'base')
    timed += ['MUT cnt AS Integer = 0', f'FOR i = 0 TO {E["rem_n"]} - 1']
    if rd_line:
        timed.append('  ' + rd_line)
    timed += [
        '  ' + c.put(f'collections::remove({rd}, {E["member"]})'),
        '  cnt = cnt + 1',
        'NEXT',
        'checksum = cnt',
    ]
    return outside, [], timed


@op('contains')
def _(c, E):
    return ro(c, E, _bld(E, E['ro_n']), E['k_contains'],
              lambda L: [f'FOR i = 0 TO {E["ro_n"]} - 1',
                         f'  IF collections::contains({L}, {E["member"]}) THEN',
                         '    acc = acc + 1', '  END IF', 'NEXT'])


@op('toList')
def _(c, E):
    return ro(c, E, _bld(E, E['ro_n']), E['k_tolist'],
              lambda L: [f'acc = acc + len(collections::toList({L}))'])


@op('toSet')
def _(c, E):
    return ro(c, E, _bld(E, E['alg_n']), E['k_alg'],
              lambda L: [f'acc = acc + len(collections::toSet(collections::toList({L})))'])


def _binary(name):
    @op(name)
    def _(c, E):
        return ro(c, E, _bld(E, E['alg_n']), E['k_alg'],
                  lambda L: [f'acc = acc + len(collections::{name}({L}, other))'],
                  extra_outside=[f'LET other AS Set OF {E["ty"]} = {_oth(E, E["alg_n"], E["alg_sh"])}'])


for _n in ('union', 'intersection', 'difference', 'symmetricDifference'):
    _binary(_n)


# plan-121-E: the three set predicates USED to run at `ro_n`/`ro_sh` against a
# partially-overlapping `other`, which was wrong twice over.
#
#  1. WRONG SIZE. C (`setmatrix.c`) and Python (`setmatrix.py`) both build these
#     rows from `alg_n`/`alg_sh`; only mfb used `ro_n`/`ro_sh`, so it compared
#     sets 3.3x larger (Integer) than its peers. A templating slip -- `_pred` was
#     written from the `contains`/`toList` shape, which legitimately uses `ro_n`,
#     instead of from `_binary` above, which correctly uses `alg_n`.
#
#  2. EARLY-EXIT LUCK. The predicate was FALSE on every call, so all three
#     languages searched for one counterexample and stopped -- and they meet it at
#     different points, because C walks its hash SLOT array while mfb walks the
#     set in ENTRY order. Measured (plan-121-E Phase 1): per call C did 2 probes
#     for `isSuperset` and 3 for `isDisjoint`; mfb did 501 of each. The rows were
#     reporting iteration order, not throughput.
#
# Both are fixed here. The predicate is now TRUE in every case, which forces a
# FULL scan in all three languages and removes early-exit luck entirely -- there
# is no counterexample to find early or late. `other` is chosen per predicate to
# make it so, at the shared `alg_n` size:
#
#     isSubset(base, other)   other == base            -> TRUE, scans all alg_n
#     isSuperset(base, other) other == base            -> TRUE, scans all alg_n
#     isDisjoint(base, other) other == base + alg_n    -> TRUE, scans all alg_n
#
# Every probe now hits (subset/superset) or misses (disjoint) and the loop always
# runs to completion, so the three languages examine exactly the same number of
# elements. The checksum changes from 0 to k_pred, which is the point: a checksum
# of 0 was the signal that the predicate never held.
_PRED_SHIFT = {'isSubset': 0, 'isSuperset': 0, 'isDisjoint': None}


def _pred(name):
    @op(name)
    def _(c, E):
        sh = _PRED_SHIFT[name]
        shift = E['alg_n'] if sh is None else sh
        return ro(c, E, _bld(E, E['alg_n']), E['k_pred'],
                  lambda L: [f'IF collections::{name}({L}, other) THEN', '  acc = acc + 1', 'END IF'],
                  extra_outside=[f'LET other AS Set OF {E["ty"]} = {_oth(E, E["alg_n"], shift)}'])


for _n in ('isSubset', 'isSuperset', 'isDisjoint'):
    _pred(_n)


OP_ORDER = [
    'add', 'remove', 'contains', 'toList', 'toSet', 'union', 'intersection',
    'difference', 'symmetricDifference', 'isSubset', 'isSuperset', 'isDisjoint',
]

GROUPS = [
    ('Fixed', INT, 'plain', 'sf'),
    ('Dynamic', STR, 'plain', 'sd'),
    ('Record-Fixed', INT, 'record', 'srf'),
    ('Record-Dynamic', STR, 'record', 'srd'),
    ('State-Fixed', INT, 'state', 'ssf'),
    ('State-Dynamic', STR, 'state', 'ssd'),
]


def emit_func(suffix, E, kind, prefix, name):
    c = Container(kind)
    outside, pre_t0, timed = OPS[name](c, E)
    label = f'set ({suffix})'
    fn = f'test_{prefix}_{name}'
    out = []
    out.append(f'FUNC {fn}(run AS Integer, tests AS List OF BenchResult) AS List OF BenchResult')
    if kind == 'state':
        out.append(f'  LET stPath AS String = fs::pathJoin([fs::tempDirectory(), "bench_{fn}.tmp"])')
        out.append(f'  RES f AS fs::File STATE {E["rec"]} = fs::open(stPath, "write")')
        out.append('  fs::deleteFile(stPath)')
    for ln in outside:
        out.append('  ' + ln)
    out.append('  MUT times AS List OF Integer = []')
    out.append('  MUT checksum AS Integer = 0')
    out.append('  FOR r = 0 TO run - 1')
    for ln in pre_t0:
        out.append('    ' + ln)
    out.append('    LET t0 AS Integer = datetime::monotonicNanos()')
    for ln in timed:
        out.append('    ' + ln)
    out.append('    LET t1 AS Integer = datetime::monotonicNanos()')
    out.append('    times = collections::append(times, t1 - t0)')
    out.append('  NEXT')
    out.append(f'  io::printError("{fn} = " & toString(checksum))')
    out.append(f'  RETURN collections::append(tests, makeResult("{label}", "{name}", times))')
    out.append('END FUNC')
    return '\n'.join(out)


HEADER = '''\
' ===========================================================================
' 6-way Set benchmark matrix. GENERATED by benchmark/mfb/gen_set.py — do not
' edit by hand; edit the generator and regenerate:
'   python3 benchmark/mfb/gen_set.py > benchmark/mfb/src/setops.mfb
'
' Every Set op is run across six groups differing only in element type and
' container:
'   set (Fixed)          Set OF Integer, plain MUT local
'   set (Dynamic)        Set OF String,  plain MUT local
'   set (Record-Fixed)   Set OF Integer in a record field (fields before + after)
'   set (Record-Dynamic) Set OF String  in a record field
'   set (State-Fixed)    Set OF Integer in a File STATE field
'   set (State-Dynamic)  Set OF String  in a File STATE field
'
' The Record/State groups exercise the whole-record-rebuild path (bug-430):
' `rec = WITH rec { xs := ... }` and `f.state.xs = ...` rebuild the record on
' every mutation, so the add/remove rows expose the O(n^2) accumulation vs. the
' plain MUT baseline; the read-only algebra rows (union/intersection/...) show
' whether wrapping regresses a pure read.
' ===========================================================================
IMPORT io
IMPORT collections
IMPORT datetime
IMPORT fs

TYPE RecSetFixed
  before AS Integer
  xs AS Set OF Integer
  after AS Integer
END TYPE

TYPE RecSetDyn
  before AS Integer
  xs AS Set OF String
  after AS Integer
END TYPE

' --- Set builders ----------------------------------------------------------
FUNC buildSetRange(n AS Integer) AS Set OF Integer
  MUT s AS Set OF Integer = Set OF Integer { }
  FOR i = 0 TO n - 1
    s = collections::add(s, i)
  NEXT
  RETURN s
END FUNC

FUNC buildSetShift(n AS Integer, sh AS Integer) AS Set OF Integer
  MUT s AS Set OF Integer = Set OF Integer { }
  FOR i = 0 TO n - 1
    s = collections::add(s, i + sh)
  NEXT
  RETURN s
END FUNC

FUNC buildStrSetRange(n AS Integer) AS Set OF String
  MUT s AS Set OF String = Set OF String { }
  FOR i = 0 TO n - 1
    s = collections::add(s, "s" & toString(i))
  NEXT
  RETURN s
END FUNC

FUNC buildStrSetShift(n AS Integer, sh AS Integer) AS Set OF String
  MUT s AS Set OF String = Set OF String { }
  FOR i = 0 TO n - 1
    s = collections::add(s, "s" & toString(i + sh))
  NEXT
  RETURN s
END FUNC
'''


def main():
    print(HEADER)
    for suffix, E, kind, prefix in GROUPS:
        print()
        print('\' ' + '=' * 75)
        print(f"' GROUP: set ({suffix})")
        print('\' ' + '=' * 75)
        for name in OP_ORDER:
            print()
            print(emit_func(suffix, E, kind, prefix, name))


if __name__ == '__main__':
    main()
