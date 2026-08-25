#!/usr/bin/env python3
"""Generator for benchmark/mfb/src/mapmatrix.mfb.

Two families of Map benchmark, both driving the same map-shaped op set
(`set`/`get`/`getOr`/`hasKey`/`removeKey`/`keys`/`values`/`mapValues`/`merge`),
one function per op:

  Value-matrix (6 groups) — vary the VALUE type across the full container grid,
  key fixed at Integer:
    map (Fixed)          Map OF Integer TO Integer, plain MUT local
    map (Dynamic)        Map OF Integer TO String,  plain MUT local
    map (Record-Fixed)   Map OF Integer TO Integer in a record field (before + after)
    map (Record-Dynamic) Map OF Integer TO String  in a record field
    map (State-Fixed)    Map OF Integer TO Integer in a File STATE field
    map (State-Dynamic)  Map OF Integer TO String  in a File STATE field

  Key-hash pair (2 groups) — vary the KEY type, value fixed at Integer, plain
  standalone only (the key-hash path is independent of the container, so no
  Record/State split):
    map (key-Fixed)      Map OF Integer TO Integer, plain  (Integer key hash)
    map (key-Dynamic)    Map OF String  TO Integer, plain  (String  key hash)

Fixed = Integer, Dynamic = String on whichever axis a group varies. A group runs
at the reduced (Dynamic) sizes when EITHER its key or value is String, because
either side churns the arena; see the README comparability caveat.

Regenerate with:  python3 benchmark/mfb/gen_map.py > benchmark/mfb/src/mapmatrix.mfb
"""


def mapty(E):
    return f'Map OF {E["kty"]} TO {E["vty"]}'


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

    def read_typed(self, mty):
        if self.kind == 'plain':
            return (None, 'nums')
        return (f'LET cur AS {mty} = {self.get()}', 'cur')

    def accum_reset(self, rec_ty, mty):
        empty = f'{mty} {{ }}'
        if self.kind == 'plain':
            return [f'MUT nums AS {mty} = {empty}']
        if self.kind == 'record':
            return [f'MUT rec AS {rec_ty} = {rec_ty}[1, {empty}, 2]']
        return [f'f.state.xs = {empty}']

    def mut_setup(self, rec_ty, mty, build):
        if self.kind == 'plain':
            return [f'MUT nums AS {mty} = {build}']
        if self.kind == 'record':
            return [f'MUT rec AS {rec_ty} = {rec_ty}[1, {build}, 2]']
        return [f'f.state.xs = {build}']


# Integer-only maps run full size; a String on either axis pulls the arena churn
# ceiling in, so those groups use the reduced sizes.
INT_SIZES = dict(set_n=300, rem_n=300, ro_n=1000, ro_sh=500, prod_n=300, prod_sh=150,
                 k_get=100, k_keys=200, k_prod=20)
STR_SIZES = dict(set_n=400, rem_n=400, ro_n=200, ro_sh=100, prod_n=100, prod_sh=50,
                 k_get=50, k_keys=100, k_prod=15)


def make_E(kty, vty, rec, build, shift):
    key_expr = 'i' if kty == 'Integer' else '"k" & toString(i)'
    val_expr = 'i' if vty == 'Integer' else '"v" & toString(i)'
    if vty == 'Integer':
        get_agg = lambda L: f'collections::get({L}, {key_expr})'
        getor = lambda L: f'collections::getOr({L}, {key_expr}, 0)'
        mapfn = 'mapDbl'
    else:
        get_agg = lambda L: f'len(collections::get({L}, {key_expr}))'
        getor = lambda L: f'len(collections::getOr({L}, {key_expr}, ""))'
        mapfn = 'mapTag'
    E = dict(kty=kty, vty=vty, rec=rec, build=build, shift=shift,
             key_expr=key_expr, set_val=val_expr, get_agg=get_agg, getor=getor, mapfn=mapfn)
    E.update(INT_SIZES if (kty == 'Integer' and vty == 'Integer') else STR_SIZES)
    return E


def _extract(c, E):
    if c.kind == 'plain':
        return []
    return [f'LET base AS {mapty(E)} = {c.ro_list()}']


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


@op('set')
def _(c, E):
    G = c.get()
    timed = c.accum_reset(E['rec'], mapty(E))
    timed += [
        f'FOR i = 0 TO {E["set_n"]} - 1',
        '  ' + c.put(f'collections::set({G}, {E["key_expr"]}, {E["set_val"]})'),
        'NEXT',
        f'checksum = len(collections::keys({G}))',
    ]
    return [], [], timed


@op('removeKey')
def _(c, E):
    rd_line, rd = c.read_typed(mapty(E))
    outside = [f'LET base = {_bld(E, E["rem_n"])}']
    timed = c.mut_setup(E['rec'], mapty(E), 'base')
    timed += ['MUT cnt AS Integer = 0', f'FOR i = 0 TO {E["rem_n"]} - 1']
    if rd_line:
        timed.append('  ' + rd_line)
    timed += [
        '  ' + c.put(f'collections::removeKey({rd}, {E["key_expr"]})'),
        '  cnt = cnt + 1',
        'NEXT',
        'checksum = cnt',
    ]
    return outside, [], timed


@op('get')
def _(c, E):
    return ro(c, E, _bld(E, E['ro_n']), E['k_get'],
              lambda L: [f'FOR i = 0 TO {E["ro_n"]} - 1',
                         f'  acc = acc + {E["get_agg"](L)}', 'NEXT'])


@op('getOr')
def _(c, E):
    return ro(c, E, _bld(E, E['ro_n']), E['k_get'],
              lambda L: [f'FOR i = 0 TO {E["ro_n"]} - 1',
                         f'  acc = acc + {E["getor"](L)}', 'NEXT'])


@op('hasKey')
def _(c, E):
    return ro(c, E, _bld(E, E['ro_n']), E['k_get'],
              lambda L: [f'FOR i = 0 TO {E["ro_n"]} - 1',
                         f'  IF collections::hasKey({L}, {E["key_expr"]}) THEN', '    acc = acc + 1',
                         '  END IF', 'NEXT'])


@op('keys')
def _(c, E):
    return ro(c, E, _bld(E, E['ro_n']), E['k_keys'],
              lambda L: [f'acc = acc + len(collections::keys({L}))'])


@op('values')
def _(c, E):
    return ro(c, E, _bld(E, E['ro_n']), E['k_keys'],
              lambda L: [f'acc = acc + len(collections::values({L}))'])


@op('mapValues')
def _(c, E):
    return ro(c, E, _bld(E, E['prod_n']), E['k_prod'],
              lambda L: [f'acc = acc + len(collections::keys(collections::mapValues({L}, {E["mapfn"]})))'])


@op('merge')
def _(c, E):
    return ro(c, E, _bld(E, E['prod_n']), E['k_prod'],
              lambda L: [f'acc = acc + len(collections::keys(collections::merge({L}, other, TRUE)))'],
              extra_outside=[f'LET other AS {mapty(E)} = {E["shift"]}({E["prod_n"]}, {E["prod_sh"]})'])


OP_ORDER = ['set', 'get', 'getOr', 'hasKey', 'removeKey', 'keys', 'values', 'mapValues', 'merge']

# (label, E, container-kind, fn-prefix)
GROUPS = [
    # value-matrix — vary VALUE, key = Integer, full container grid
    ('Fixed', make_E('Integer', 'Integer', 'RecMapFixed', 'buildMapII', 'buildMapIIShift'), 'plain', 'mf'),
    ('Dynamic', make_E('Integer', 'String', 'RecMapDyn', 'buildMapIS', 'buildMapISShift'), 'plain', 'md'),
    ('Record-Fixed', make_E('Integer', 'Integer', 'RecMapFixed', 'buildMapII', 'buildMapIIShift'), 'record', 'mrf'),
    ('Record-Dynamic', make_E('Integer', 'String', 'RecMapDyn', 'buildMapIS', 'buildMapISShift'), 'record', 'mrd'),
    ('State-Fixed', make_E('Integer', 'Integer', 'RecMapFixed', 'buildMapII', 'buildMapIIShift'), 'state', 'msf'),
    ('State-Dynamic', make_E('Integer', 'String', 'RecMapDyn', 'buildMapIS', 'buildMapISShift'), 'state', 'msd'),
    # key-hash pair — vary KEY, value = Integer, plain standalone only
    ('key-Fixed', make_E('Integer', 'Integer', '', 'buildMapII', 'buildMapIIShift'), 'plain', 'mkf'),
    ('key-Dynamic', make_E('String', 'Integer', '', 'buildMapSI', 'buildMapSIShift'), 'plain', 'mkd'),
]


def emit_func(suffix, E, kind, prefix, name):
    c = Container(kind)
    outside, pre_t0, timed = OPS[name](c, E)
    label = f'map ({suffix})'
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
' Map benchmarks. GENERATED by benchmark/mfb/gen_map.py — do not edit by hand;
' edit the generator and regenerate:
'   python3 benchmark/mfb/gen_map.py > benchmark/mfb/src/mapmatrix.mfb
'
' Two families, both over the same map-op set, one function per op:
'
' Value-matrix (vary VALUE, key = Integer, full plain/Record/State grid):
'   map (Fixed) / (Dynamic) / (Record-Fixed) / (Record-Dynamic) /
'   (State-Fixed) / (State-Dynamic)
'   The Record/State groups exercise the whole-record-rebuild path (bug-430):
'   `set`/`removeKey` show the O(n^2) accumulation vs. the plain MUT baseline.
'
' Key-hash pair (vary KEY, value = Integer, plain standalone only — the key-hash
' path is independent of the container):
'   map (key-Fixed)    Integer-key hash
'   map (key-Dynamic)  String-key hash
'
' A group runs at reduced sizes when either its key or value is String (arena
' mixed-transient-churn ceiling — see the README comparability caveat before
' comparing Fixed vs Dynamic).
' ===========================================================================
IMPORT io
IMPORT collections
IMPORT datetime
IMPORT fs

TYPE RecMapFixed
  before AS Integer
  xs AS Map OF Integer TO Integer
  after AS Integer
END TYPE

TYPE RecMapDyn
  before AS Integer
  xs AS Map OF Integer TO String
  after AS Integer
END TYPE

FUNC mapDbl(v AS Integer) AS Integer
  RETURN v + v
END FUNC

FUNC mapTag(v AS String) AS String
  RETURN "[" & v & "]"
END FUNC

' Integer key -> Integer value
FUNC buildMapII(n AS Integer) AS Map OF Integer TO Integer
  MUT m AS Map OF Integer TO Integer = Map OF Integer TO Integer { }
  FOR i = 0 TO n - 1
    m = collections::set(m, i, i)
  NEXT
  RETURN m
END FUNC

FUNC buildMapIIShift(n AS Integer, sh AS Integer) AS Map OF Integer TO Integer
  MUT m AS Map OF Integer TO Integer = Map OF Integer TO Integer { }
  FOR i = 0 TO n - 1
    m = collections::set(m, i + sh, i + sh)
  NEXT
  RETURN m
END FUNC

' Integer key -> String value
FUNC buildMapIS(n AS Integer) AS Map OF Integer TO String
  MUT m AS Map OF Integer TO String = Map OF Integer TO String { }
  FOR i = 0 TO n - 1
    m = collections::set(m, i, "v" & toString(i))
  NEXT
  RETURN m
END FUNC

FUNC buildMapISShift(n AS Integer, sh AS Integer) AS Map OF Integer TO String
  MUT m AS Map OF Integer TO String = Map OF Integer TO String { }
  FOR i = 0 TO n - 1
    m = collections::set(m, i + sh, "v" & toString(i + sh))
  NEXT
  RETURN m
END FUNC

' String key -> Integer value
FUNC buildMapSI(n AS Integer) AS Map OF String TO Integer
  MUT m AS Map OF String TO Integer = Map OF String TO Integer { }
  FOR i = 0 TO n - 1
    m = collections::set(m, "k" & toString(i), i)
  NEXT
  RETURN m
END FUNC

FUNC buildMapSIShift(n AS Integer, sh AS Integer) AS Map OF String TO Integer
  MUT m AS Map OF String TO Integer = Map OF String TO Integer { }
  FOR i = 0 TO n - 1
    m = collections::set(m, "k" & toString(i + sh), i + sh)
  NEXT
  RETURN m
END FUNC
'''


def main():
    print(HEADER)
    for suffix, E, kind, prefix in GROUPS:
        print()
        print('\' ' + '=' * 75)
        print(f"' GROUP: map ({suffix})")
        print('\' ' + '=' * 75)
        for name in OP_ORDER:
            print()
            print(emit_func(suffix, E, kind, prefix, name))


if __name__ == '__main__':
    main()
