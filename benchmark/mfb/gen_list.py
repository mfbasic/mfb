#!/usr/bin/env python3
"""Generator for benchmark/mfb/src/list.mfb.

Emits the 6-way list benchmark matrix. Every existing list op is replicated
across six groups that differ only in element type and container:

    list (Fixed)          Integer list, plain MUT local
    list (Dynamic)        String  list, plain MUT local
    list (Record-Fixed)   Integer list held in a record field (fields before+after)
    list (Record-Dynamic) String  list held in a record field
    list (State-Fixed)    Integer list held in a File STATE field
    list (State-Dynamic)  String  list held in a File STATE field

Regenerate with:  python3 benchmark/mfb/gen_list.py > benchmark/mfb/src/list.mfb
"""

# --- container model --------------------------------------------------------
# A container decides how the working list is stored, read (GET), and written
# back (PUT). readonly ops only need GET; accumulating/mutating ops need PUT.


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

    # Storage that holds a *read-only* base list, declared once before timing.
    def ro_setup(self, rec_ty, build):
        if self.kind == 'plain':
            return [f'LET base = {build}']
        if self.kind == 'record':
            return [f'LET rec AS {rec_ty} = {rec_ty}[1, {build}, 2]']
        return [f'f.state.xs = {build}']

    def ro_list(self):
        return {'plain': 'base', 'record': 'rec.xs', 'state': 'f.state.xs'}[self.kind]

    # Fresh empty storage at the top of a timed run (accumulate-from-empty ops).
    def accum_reset(self, rec_ty, elem_ty):
        if self.kind == 'plain':
            return [f'MUT nums AS List OF {elem_ty} = []']
        if self.kind == 'record':
            return [f'MUT rec AS {rec_ty} = {rec_ty}[1, [], 2]']
        return ['f.state.xs = []']

    # A typed read of the working list for a template-arg position. Plain groups
    # use the MUT local directly; record/state groups rebind the field to a typed
    # local because a bare field read types as Unknown for template inference.
    def read_typed(self, elem_ty):
        if self.kind == 'plain':
            return (None, 'nums')
        return (f'LET cur AS List OF {elem_ty} = {self.get()}', 'cur')

    # Storage holding a base list to be mutated in place (set/removeAt).
    def mut_setup(self, rec_ty, elem_ty, build):
        if self.kind == 'plain':
            return [f'MUT nums AS List OF {elem_ty} = {build}']
        if self.kind == 'record':
            return [f'MUT rec AS {rec_ty} = {rec_ty}[1, {build}, 2]']
        return [f'f.state.xs = {build}']


# --- element-type vocabulary ------------------------------------------------

INT = dict(
    ty='Integer', rec='RecFixed',
    build_range='buildRange(1000)', build_2000='buildRange(2000)',
    build_500='buildRange(500)', build_50='buildRand(50)',
    dup='buildDupRange(5000, 1000)',
    asc='sortMakeAsc(20000)', desc='sortMakeDesc(20000)', scr='sortMakeScramble(20000)',
    hash='sortHash',
    append_val='i', prepend_val='i', insert_val='i', batch='tenInts',
    get_ty='Integer', set_new='v + 1', set_sum_init='0', set_sum_add='collections::get({G}, j)',
    find_target='999', contains_target='1000',
    replace='500, 500',
    reduce_init='0', reduce_fn='addFn', transform_fn='doubleFn', filter_pred='isEvenN',
    all_build='buildPos(1000)', all_pred='isPos',
    any_build='buildNeg(1000)', any_pred='isPos',
    groupby_key='bucketKey', groupby_val='identity',
    sortby_key='keyNeg', copy_fn='copyInts',
    forEach_fn='bumpAcc', forEach_acc='forEachAcc',
    partition_pred='isEvenN',
)

STR = dict(
    ty='String', rec='RecDyn',
    build_range='buildStrRange(1000)', build_2000='buildStrRange(2000)',
    build_500='buildStrRange(500)', build_50='buildStrRange(50)',
    dup='buildStrDup(5000, 1000)',
    asc='strMakeAsc(20000)', desc='strMakeDesc(20000)', scr='strMakeScramble(20000)',
    hash='strSortHash',
    append_val='"s" & toString(i)', prepend_val='"s" & toString(i)',
    insert_val='"m" & toString(i)', batch='tenStrs',
    get_ty='String', set_new='v & "!"', set_sum_init='0', set_sum_add='len(collections::get({G}, j))',
    find_target='"s999"', contains_target='"absent"',
    replace='"s5", "S5"',
    reduce_init='""', reduce_fn='strConcatFn', transform_fn='strWrap', filter_pred='strIsShort',
    all_build='buildStrRange(1000)', all_pred='strNonEmpty',
    any_build='buildStrRange(1000)', any_pred='strIsShort',
    groupby_key='strLenKey', groupby_val='strIdentity',
    sortby_key='strLenKey', copy_fn='copyStrs',
    forEach_fn='strBumpLen', forEach_acc='strForEachAcc',
    partition_pred='strIsShort',
)

# Ops present only for the Integer element type.
INT_ONLY = {'sum'}


# --- op definitions ---------------------------------------------------------
# Each op returns the lines that go *inside* a timed run (between t0 and t1),
# plus a preceding "acc" accumulator where relevant. `c` is the Container, `E`
# the element vocab. Helpers below build the three canonical shapes.


# Every op returns a 3-tuple of line lists placed relative to the timing loop:
#   outside  — once, before `FOR r` (built-once base for read-only ops)
#   pre_t0   — inside `FOR r`, before t0 (untimed per-run rebuild, e.g. `set`)
#   timed    — inside `FOR r`, between t0 and t1 (the measured work)


def _extract(c, E):
    """Timed lines that expose the working list as a typed `base` local.

    Plain groups already bound `base` outside. Record/State groups hold the list
    in a field; a bare `f.state.xs` in a template-arg position types as Unknown,
    so we rebind it to a typed local once per run — which also makes the read-only
    rows measure the op itself, not a per-call field copy."""
    if c.kind == 'plain':
        return []
    return [f'LET base AS List OF {E["ty"]} = {c.ro_list()}']


def ro(c, E, build, k, inner, checksum='acc'):
    """Read-only op: base built once (outside), `inner` run k times into `acc`."""
    outside = c.ro_setup(E['rec'], build)
    timed = _extract(c, E)
    timed += ['MUT acc AS Integer = 0', f'FOR k = 0 TO {k} - 1']
    for ln in inner('base'):
        timed.append('  ' + ln)
    timed.append('NEXT')
    timed.append(f'checksum = {checksum}')
    return outside, [], timed


def accum(c, E, n, mutate, checksum=None):
    """Accumulate-from-empty: reset storage + mutate n times, all timed."""
    G = c.get()
    timed = c.accum_reset(E['rec'], E['ty'])
    timed += [f'FOR i = 0 TO {n} - 1', '  ' + c.put(mutate(G)), 'NEXT']
    timed.append(f'checksum = {checksum(G) if checksum else f"len({G})"}')
    return [], [], timed


# op registry: name -> function(c, E) -> (setup_lines, timed_lines)
OPS = {}


def op(name):
    def deco(fn):
        OPS[name] = fn
        return fn
    return deco


@op('append')
def _(c, E):
    return accum(c, E, 1000, lambda G: f'collections::append({G}, {E["append_val"]})')


@op('append_batch')
def _(c, E):
    return accum(c, E, 100, lambda G: f'collections::append({G}, {E["batch"]})')


@op('prepend')
def _(c, E):
    return accum(c, E, 1000, lambda G: f'collections::prepend({G}, {E["prepend_val"]})')


@op('insert')
def _(c, E):
    return accum(c, E, 1000, lambda G: f'collections::insert({G}, len({G}) / 2, {E["insert_val"]})')


@op('copy')
def _(c, E):
    return ro(c, E, E['build_range'], 1000,
              lambda L: [f'acc = acc + len({E["copy_fn"]}({L}))'])


@op('distinct')
def _(c, E):
    return ro(c, E, E['dup'], 1,
              lambda L: [f'acc = acc + len(collections::distinct({L}))'])


@op('groupby')
def _(c, E):
    return ro(c, E, E['build_2000'], 1,
              lambda L: [f'acc = acc + len(collections::groupBy({L}, {E["groupby_key"]}, {E["groupby_val"]}))'])


@op('set')
def _(c, E):
    pre_t0 = c.mut_setup(E['rec'], E['ty'], _build_n(E, 200))
    rd_line, rd = c.read_typed(E['ty'])
    fin_line, fin = c.read_typed(E['ty'])
    timed = ['FOR pass = 0 TO 9', '  FOR j = 0 TO 199']
    if rd_line:
        timed.append('    ' + rd_line)
    timed += [
        f'    LET v AS {E["get_ty"]} = collections::get({rd}, j)',
        '    ' + c.put(f'collections::set({rd}, j, {E["set_new"]})'),
        '  NEXT',
        'NEXT',
    ]
    if fin_line:
        timed.append(fin_line)
    timed += [
        'MUT sumv AS Integer = 0',
        'FOR j = 0 TO 199',
        f'  sumv = sumv + {E["set_sum_add"].format(G=fin)}',
        'NEXT',
        'checksum = sumv',
    ]
    return [], pre_t0, timed


@op('sort')
def _(c, E):
    return ro(c, E, E['build_50'], 1,
              lambda L: [f'acc = acc + len(collections::sort({L}))'])


def _sortrun(c, E, build):
    outside = c.ro_setup(E['rec'], build)
    timed = _extract(c, E)
    timed += [
        'MUT acc AS Integer = 0',
        'FOR k = 0 TO 0',
        f'  LET sorted AS List OF {E["ty"]} = collections::sort(base)',
        f'  acc = acc + {E["hash"]}(sorted)',
        'NEXT',
        'checksum = acc',
    ]
    return outside, [], timed


@op('sort_asc')
def _(c, E):
    return _sortrun(c, E, E['asc'])


@op('sort_desc')
def _(c, E):
    return _sortrun(c, E, E['desc'])


@op('sort_rand')
def _(c, E):
    return _sortrun(c, E, E['scr'])


@op('all')
def _(c, E):
    return ro(c, E, E['all_build'], 200,
              lambda L: [f'IF collections::all({L}, {E["all_pred"]}) THEN', '  acc = acc + 1', 'END IF'])


@op('any')
def _(c, E):
    return ro(c, E, E['any_build'], 200,
              lambda L: [f'IF collections::any({L}, {E["any_pred"]}) THEN', '  acc = acc + 1', 'END IF'])


@op('chunks')
def _(c, E):
    return ro(c, E, E['build_range'], 200,
              lambda L: [f'acc = acc + len(collections::chunks({L}, 10))'])


@op('contains')
def _(c, E):
    return ro(c, E, E['build_range'], 500,
              lambda L: [f'IF NOT collections::contains({L}, {E["contains_target"]}) THEN', '  acc = acc + 1', 'END IF'])


@op('drop')
def _(c, E):
    return ro(c, E, E['build_range'], 500,
              lambda L: [f'acc = acc + len(collections::drop({L}, 500))'])


@op('filter')
def _(c, E):
    return ro(c, E, E['build_range'], 200,
              lambda L: [f'acc = acc + len(collections::filter({L}, {E["filter_pred"]}))'])


@op('find')
def _(c, E):
    return ro(c, E, E['build_range'], 500,
              lambda L: [f'acc = acc + collections::find({L}, {E["find_target"]})'])


@op('findIndex')
def _(c, E):
    pred = 'ge999' if E is INT else 'strIsLong'
    return ro(c, E, E['build_range'], 500,
              lambda L: [f'acc = acc + collections::findIndex({L}, {pred})'])


@op('findLastIndex')
def _(c, E):
    pred = 'le5' if E is INT else 'strIsShort'
    return ro(c, E, E['build_range'], 500,
              lambda L: [f'acc = acc + collections::findLastIndex({L}, {pred})'])


@op('flatten')
def _(c, E):
    return ro(c, E, E['build_range'], 200,
              lambda L: [f'acc = acc + len(collections::flatten(collections::chunks({L}, 10)))'])


@op('forEach')
def _(c, E):
    outside = c.ro_setup(E['rec'], E['build_range'])
    timed = _extract(c, E)
    timed += [
        f'{E["forEach_acc"]} = 0',
        'FOR k = 0 TO 199',
        f'  collections::forEach(base, {E["forEach_fn"]})',
        'NEXT',
        f'checksum = {E["forEach_acc"]}',
    ]
    return outside, [], timed


@op('get')
def _(c, E):
    return ro(c, E, E['build_range'], 100,
              lambda L: ['FOR i = 0 TO 999', f'  acc = acc + len2(collections::get({L}, i))' if E is STR
                         else f'  acc = acc + collections::get({L}, i)', 'NEXT'])


@op('getOr')
def _(c, E):
    default = '""' if E is STR else '0'
    inner = (lambda L: ['FOR i = 0 TO 999', f'  acc = acc + len(collections::getOr({L}, i, {default}))', 'NEXT']) if E is STR \
        else (lambda L: ['FOR i = 0 TO 999', f'  acc = acc + collections::getOr({L}, i, 0)', 'NEXT'])
    return ro(c, E, E['build_range'], 100, inner)


@op('mid')
def _(c, E):
    return ro(c, E, E['build_range'], 500,
              lambda L: [f'acc = acc + len(collections::mid({L}, 250, 500))'])


@op('partition')
def _(c, E):
    return ro(c, E, E['build_range'], 200,
              lambda L: [f'LET p AS Partition OF {E["ty"]} = collections::partition({L}, {E["partition_pred"]})',
                         'acc = acc + len(p.matched)'])


@op('reduce')
def _(c, E):
    acc_expr = 'len(collections::reduce({L}, {i}, {f}))' if E is STR else 'collections::reduce({L}, {i}, {f})'
    return ro(c, E, E['build_range'], 500,
              lambda L: [f'acc = acc + ' + acc_expr.format(L=L, i=E['reduce_init'], f=E['reduce_fn'])])


@op('reduceRight')
def _(c, E):
    acc_expr = 'len(collections::reduceRight({L}, {i}, {f}))' if E is STR else 'collections::reduceRight({L}, {i}, {f})'
    return ro(c, E, E['build_range'], 500,
              lambda L: [f'acc = acc + ' + acc_expr.format(L=L, i=E['reduce_init'], f=E['reduce_fn'])])


@op('removeAt')
def _(c, E):
    G = c.get()
    rd_line, rd = c.read_typed(E['ty'])
    outside = [f'LET base = {E["build_range"]}']
    timed = c.mut_setup(E['rec'], E['ty'], 'base')
    timed += ['MUT cnt AS Integer = 0', f'WHILE len({G}) > 0']
    if rd_line:
        timed.append('  ' + rd_line)
    timed += [
        '  ' + c.put(f'collections::removeAt({rd}, 0)'),
        '  cnt = cnt + 1',
        'END WHILE',
        'checksum = cnt',
    ]
    return outside, [], timed


@op('replace')
def _(c, E):
    return ro(c, E, E['build_range'], 200,
              lambda L: [f'acc = acc + len(collections::replace({L}, {E["replace"]}))'])


@op('sortBy')
def _(c, E):
    return ro(c, E, E['build_500'], 200,
              lambda L: [f'LET s AS List OF {E["ty"]} = collections::sortBy({L}, {E["sortby_key"]})',
                         f'acc = acc + len2(collections::get(s, 0))' if E is STR
                         else f'acc = acc + collections::get(s, 0)'])


@op('sum')
def _(c, E):
    return ro(c, E, E['build_range'], 1000,
              lambda L: [f'acc = acc + collections::sum({L})'])


@op('take')
def _(c, E):
    return ro(c, E, E['build_range'], 500,
              lambda L: [f'acc = acc + len(collections::take({L}, 500))'])


@op('transform')
def _(c, E):
    return ro(c, E, E['build_range'], 200,
              lambda L: [f'acc = acc + len(collections::transform({L}, {E["transform_fn"]}))'])


@op('window')
def _(c, E):
    return ro(c, E, E['build_range'], 100,
              lambda L: [f'acc = acc + len(collections::window({L}, 10))'])


@op('zip')
def _(c, E):
    return ro(c, E, E['build_range'], 100,
              lambda L: [f'acc = acc + len(collections::zip({L}, {L}))'])


def _build_n(E, n):
    return f'buildRange({n})' if E is INT else f'buildStrRange({n})'


# order of ops in every group
OP_ORDER = [
    'append', 'append_batch', 'prepend', 'copy', 'distinct', 'groupby', 'set',
    'sort', 'sort_asc', 'sort_desc', 'sort_rand', 'all', 'any', 'chunks',
    'contains', 'drop', 'filter', 'find', 'findIndex', 'findLastIndex',
    'flatten', 'forEach', 'get', 'getOr', 'insert', 'mid', 'partition',
    'reduce', 'reduceRight', 'removeAt', 'replace', 'sortBy', 'sum', 'take',
    'transform', 'window', 'zip',
]

GROUPS = [
    ('Fixed', INT, 'plain', 'lf'),
    ('Dynamic', STR, 'plain', 'ld'),
    ('Record-Fixed', INT, 'record', 'lrf'),
    ('Record-Dynamic', STR, 'record', 'lrd'),
    ('State-Fixed', INT, 'state', 'lsf'),
    ('State-Dynamic', STR, 'state', 'lsd'),
]


def emit_func(suffix, E, kind, prefix, name):
    c = Container(kind)
    outside, pre_t0, timed = OPS[name](c, E)
    label = f'list ({suffix})'
    fn = f'test_{prefix}_{name}'
    out = []
    out.append(f'FUNC {fn}(run AS Integer, tests AS List OF BenchResult) AS List OF BenchResult')
    if kind == 'state':
        # Known temp path so we can delete it: create it, then unlink immediately
        # (the STATE benchmark only holds the handle — it never touches the file's
        # bytes — so the open fd stays valid after the unlink, and no temp file is
        # left behind). The handle is closed by RES drop at function return.
        out.append(f'  LET stPath AS String = fs::pathJoin([fs::tempDirectory(), "bench_{fn}.tmp"])')
        out.append(f'  RES f AS File STATE {E["rec"]} = fs::open(stPath, "write")')
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
' 6-way list benchmark matrix. GENERATED by benchmark/mfb/gen_list.py — do not
' edit by hand; edit the generator and regenerate:
'   python3 benchmark/mfb/gen_list.py > benchmark/mfb/src/list.mfb
'
' Every list op is run across six groups differing only in element type and
' container:
'   list (Fixed)          Integer list, plain MUT local
'   list (Dynamic)        String  list, plain MUT local
'   list (Record-Fixed)   Integer list in a record field (fields before + after)
'   list (Record-Dynamic) String  list in a record field
'   list (State-Fixed)    Integer list in a File STATE field
'   list (State-Dynamic)  String  list in a File STATE field
'
' The Record/State groups exercise the whole-record-rebuild path (bug-430):
' `rec = WITH rec { xs := ... }` and `f.state.xs = ...` re-inline the buffer on
' every mutation, so the accumulate rows (append/prepend/insert/...) are the
' ones that expose the O(n^2) accumulation vs. the plain MUT baseline.
' ===========================================================================
IMPORT io
IMPORT collections
IMPORT datetime
IMPORT math
IMPORT fs

TYPE RecFixed
  before AS Integer
  xs AS List OF Integer
  after AS Integer
END TYPE

TYPE RecDyn
  before AS Integer
  xs AS List OF String
  after AS Integer
END TYPE

' --- Integer element helpers -----------------------------------------------
MUT forEachAcc AS Integer = 0

LET tenInts AS List OF Integer = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]

FUNC buildRange(n AS Integer) AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, i)
  NEXT
  RETURN xs
END FUNC

FUNC buildPos(n AS Integer) AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 1 TO n
    xs = collections::append(xs, i)
  NEXT
  RETURN xs
END FUNC

FUNC buildNeg(n AS Integer) AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 1 TO n
    xs = collections::append(xs, 0 - i)
  NEXT
  RETURN xs
END FUNC

FUNC buildRand(n AS Integer) AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, math::rand(0, 1000000))
  NEXT
  RETURN xs
END FUNC

FUNC buildDupRange(n AS Integer, distinct AS Integer) AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, i - (i / distinct) * distinct)
  NEXT
  RETURN xs
END FUNC

FUNC copyInts(xs AS List OF Integer) AS List OF Integer
  RETURN xs
END FUNC

FUNC bucketKey(n AS Integer) AS Integer
  RETURN n - (n / 100) * 100
END FUNC

FUNC identity(n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC isEvenN(n AS Integer) AS Boolean
  RETURN n - (n / 2) * 2 = 0
END FUNC

FUNC ge999(n AS Integer) AS Boolean
  RETURN n >= 999
END FUNC

FUNC le5(n AS Integer) AS Boolean
  RETURN n <= 5
END FUNC

FUNC addFn(acc AS Integer, n AS Integer) AS Integer
  RETURN acc + n
END FUNC

FUNC doubleFn(n AS Integer) AS Integer
  RETURN n + n
END FUNC

FUNC keyNeg(n AS Integer) AS Integer
  RETURN 0 - n
END FUNC

SUB bumpAcc(n AS Integer)
  forEachAcc = forEachAcc + n
END SUB

FUNC sortMakeAsc(n AS Integer) AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, i)
  NEXT
  RETURN xs
END FUNC

FUNC sortMakeDesc(n AS Integer) AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, n - 1 - i)
  NEXT
  RETURN xs
END FUNC

FUNC sortMakeScramble(n AS Integer) AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, (i * 7919) MOD n)
  NEXT
  RETURN xs
END FUNC

FUNC sortHash(sorted AS List OF Integer) AS Integer
  MUT acc AS Integer = 0
  FOR i = 0 TO len(sorted) - 1
    acc = (acc * 31 + collections::get(sorted, i)) MOD 1000000007
  NEXT
  RETURN acc
END FUNC

' --- String element helpers ------------------------------------------------
MUT strForEachAcc AS Integer = 0

LET tenStrs AS List OF String = ["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "a8", "a9"]

FUNC len2(s AS String) AS Integer
  RETURN len(s)
END FUNC

FUNC buildStrRange(n AS Integer) AS List OF String
  MUT xs AS List OF String = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, "s" & toString(i))
  NEXT
  RETURN xs
END FUNC

FUNC buildStrDup(n AS Integer, distinct AS Integer) AS List OF String
  MUT xs AS List OF String = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, "d" & toString(i - (i / distinct) * distinct))
  NEXT
  RETURN xs
END FUNC

FUNC copyStrs(xs AS List OF String) AS List OF String
  RETURN xs
END FUNC

FUNC strNonEmpty(s AS String) AS Boolean
  RETURN len(s) > 0
END FUNC

FUNC strIsShort(s AS String) AS Boolean
  RETURN len(s) <= 2
END FUNC

FUNC strIsLong(s AS String) AS Boolean
  RETURN len(s) >= 4
END FUNC

FUNC strWrap(s AS String) AS String
  RETURN "[" & s & "]"
END FUNC

FUNC strConcatFn(acc AS String, s AS String) AS String
  RETURN acc & s
END FUNC

FUNC strLenKey(s AS String) AS Integer
  RETURN len(s)
END FUNC

FUNC strIdentity(s AS String) AS String
  RETURN s
END FUNC

SUB strBumpLen(s AS String)
  strForEachAcc = strForEachAcc + len(s)
END SUB

' Zero-padded numeric strings so lexicographic order == numeric order.
FUNC pad6(n AS Integer) AS String
  MUT s AS String = toString(n)
  WHILE len(s) < 6
    s = "0" & s
  END WHILE
  RETURN s
END FUNC

FUNC strMakeAsc(n AS Integer) AS List OF String
  MUT xs AS List OF String = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, pad6(i))
  NEXT
  RETURN xs
END FUNC

FUNC strMakeDesc(n AS Integer) AS List OF String
  MUT xs AS List OF String = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, pad6(n - 1 - i))
  NEXT
  RETURN xs
END FUNC

FUNC strMakeScramble(n AS Integer) AS List OF String
  MUT xs AS List OF String = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, pad6((i * 7919) MOD n))
  NEXT
  RETURN xs
END FUNC

FUNC strSortHash(sorted AS List OF String) AS Integer
  MUT acc AS Integer = 0
  FOR i = 0 TO len(sorted) - 1
    acc = (acc * 31 + len(collections::get(sorted, i))) MOD 1000000007
  NEXT
  RETURN acc
END FUNC
'''


def main():
    print(HEADER)
    for suffix, E, kind, prefix in GROUPS:
        print()
        print('\' ' + '=' * 75)
        print(f"' GROUP: list ({suffix})")
        print('\' ' + '=' * 75)
        for name in OP_ORDER:
            if name in INT_ONLY and E is STR:
                continue
            print()
            print(emit_func(suffix, E, kind, prefix, name))


if __name__ == '__main__':
    main()
