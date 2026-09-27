#!/usr/bin/env python3
"""Resource-usage audit for MFBASIC examples (bug-714).

For an example directory with a `resources` block in its `project.json`,
statically determines which resource file names the example's own MFBASIC
source actually references (via `os::appResourcePath(...)` call sites) and
compares that against what the `resources` glob(s) actually ship on disk.

Usage:
    audit.py <example-dir> [<example-dir> ...]
    audit.py --all          # every examples/* with a "resources" block

Exit status: 0 if, for every checked example, every shipped file is matched
by some referenced pattern and every exact (non-wildcard) referenced name is
shipped; 1 otherwise. Prints the offending file lists.

## Method

This is deliberately a best-effort static analysis, not a full MFBASIC
parser or interpreter -- see the bug doc (bugs/bug-714-*.md, or
bugs/completed/ once archived) for why a full parse is unnecessary here.

- Each `resources[]` entry's `src` glob and `dst` prefix are read from
  `project.json`; the glob is resolved against the example directory to get
  the shipped file set (as dst-relative names, e.g. `dungeon/stone_N.png`).
- Every `**/*.mfb` file under the example's source roots is scanned for
  `FUNC`/`SUB` definitions (name, parameter list, body lines).
- Starting at the entry function named by `project.json`'s `"entry"` field
  (default `main`), the tool walks each function body top-to-bottom as a
  flat (branch-blind) symbolic interpreter tracking, per local variable, the
  *set* of string values it could hold:
    - a quoted string literal is an exact value;
    - `a & b` concatenates the value sets of `a` and `b` (cross product);
    - a bare identifier resolves to whatever reached it via the nearest
      preceding `LET`/`MUT` assignment or an enclosing `FOR EACH ... IN`
      (loop variables take the union of their iterable's element values);
    - a call to a `FUNC`/`SUB` defined in the same source tree binds its
      formal parameters to the caller's argument value sets and recurses
      into every `RETURN` expression reachable in its body (all branches,
      unioned -- again branch-blind);
    - anything unresolvable (a built-in/external call such as
      `strings::left`, `toString`, an enum literal, etc.) becomes a single
      wildcard segment matching any non-empty text.
  Every `os::appResourcePath(...)` call site encountered during this walk
  is recorded with whatever value set is in scope at that point -- so a
  literal argument like `os::appResourcePath("wind/coastline.bin")` records
  one exact name, while a concatenation built from a known literal list and
  an unresolvable per-iteration suffix (dungeon's `name & "_" &
  facingSuffix(f)`) records one wildcard pattern per known literal name.
- A value with no wildcard segment is an exact expected dst-relative file
  name. A value with wildcard segments becomes a regex anchored on its
  literal segments (`.+` for each wildcard). Every shipped file is checked
  against every recorded pattern; a file matching none is "unreferenced".
  Every exact pattern matching no shipped file is "referenced but missing".

This resolves the two shapes that occur in this repository today (a bare
literal path, and a literal-name-list threaded through one level of
directional-suffix concatenation) without hardcoding either example's
function names. A pattern this tool cannot resolve at all degrades to a
wildcard rather than a false "unreferenced" -- i.e. it is conservative
against false positives, not false negatives, so a passing run is real
signal but is not a substitute for the "does the app still load its tiles"
runtime check called out in the bug's Validation Plan.
"""

import glob as globmod
import json
import os
import re
import sys
from pathlib import Path

MAX_CALL_DEPTH = 24
MAX_PATTERNS_PER_CALL = 4096

# A "value" is a list of patterns; a pattern is a tuple of segments, each
# segment either ("lit", text) or ("any",).
ANY_VALUE = [(("any",),)]


def unescape(s):
    return s.replace('\\"', '"').replace("\\\\", "\\")


def split_top_level(s, sep):
    """Split `s` on `sep` (a single character) at paren/bracket/quote depth 0."""
    parts = []
    depth = 0
    in_str = False
    buf = []
    i = 0
    while i < len(s):
        c = s[i]
        if in_str:
            buf.append(c)
            if c == "\\" and i + 1 < len(s):
                buf.append(s[i + 1])
                i += 2
                continue
            if c == '"':
                in_str = False
            i += 1
            continue
        if c == '"':
            in_str = True
            buf.append(c)
        elif c in "([":
            depth += 1
            buf.append(c)
        elif c in ")]":
            depth -= 1
            buf.append(c)
        elif c == sep and depth == 0:
            parts.append("".join(buf))
            buf = []
        else:
            buf.append(c)
        i += 1
    parts.append("".join(buf))
    return parts


def find_calls(line, name):
    """Every balanced-paren argument substring of calls to `name(...)` in `line`."""
    results = []
    start = 0
    while True:
        idx = line.find(name + "(", start)
        if idx == -1:
            break
        open_paren = idx + len(name)
        depth = 0
        in_str = False
        end = None
        i = open_paren
        while i < len(line):
            c = line[i]
            if in_str:
                if c == "\\" and i + 1 < len(line):
                    i += 2
                    continue
                if c == '"':
                    in_str = False
            elif c == '"':
                in_str = True
            elif c == "(":
                depth += 1
            elif c == ")":
                depth -= 1
                if depth == 0:
                    end = i
                    break
            i += 1
        if end is None:
            break
        results.append(line[open_paren + 1 : end])
        start = end + 1
    return results


CALL_RE = re.compile(r"^([A-Za-z_][A-Za-z0-9_:]*)\((.*)\)$", re.DOTALL)
IDENT_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
LET_RE = re.compile(r"^(?:LET|MUT)\s+(\w+)\s+AS\s+.+?=\s*(.+)$")
LET_NOTYPE_RE = re.compile(r"^(?:LET|MUT)\s+(\w+)\s*=\s*(.+)$")
FOREACH_RE = re.compile(r"^FOR\s+EACH\s+(\w+)\s+IN\s+(.+)$")
RETURN_RE = re.compile(r"\bRETURN\s+([^\n:]+)")
FUNCDEF_RE = re.compile(r"^(?:FUNC|SUB)\s+(\w+)\s*(\(([^)]*)\))?")


def strip_trailer(expr):
    """Drop a trailing ` TRAP(...)` clause some call-site lines carry."""
    m = re.search(r"\sTRAP\(", expr)
    if m:
        return expr[: m.start()]
    return expr


class Registry:
    def __init__(self):
        self.funcs = {}  # name -> {"params": [...], "body": [lines...]}

    def parse_file(self, text):
        lines = text.splitlines()
        i = 0
        while i < len(lines):
            stripped = lines[i].strip()
            m = FUNCDEF_RE.match(stripped)
            if m:
                name = m.group(1)
                params_str = m.group(3) or ""
                params = []
                for p in split_top_level(params_str, ","):
                    p = p.strip()
                    if not p:
                        continue
                    params.append(p.split()[0])
                body = []
                i += 1
                while i < len(lines) and lines[i].strip() not in ("END FUNC", "END SUB"):
                    body.append(lines[i])
                    i += 1
                self.funcs[name] = {"params": params, "body": body}
            i += 1


class Auditor:
    def __init__(self, registry):
        self.registry = registry
        self.memo = {}
        self.collected = []  # list of patterns (each a list of segment-tuples)
        self.depth = 0

    def resolve_call(self, funcname, arg_values):
        if funcname not in self.registry.funcs or self.depth >= MAX_CALL_DEPTH:
            return None
        key = (funcname, tuple(tuple(v) for v in arg_values))
        if key in self.memo:
            return self.memo[key]
        self.memo[key] = ANY_VALUE  # recursion guard
        fn = self.registry.funcs[funcname]
        env = {}
        for i, pname in enumerate(fn["params"]):
            env[pname] = arg_values[i] if i < len(arg_values) else ANY_VALUE
        self.depth += 1
        return_values = []
        for line in fn["body"]:
            self.process_line(line, env, return_values)
        self.depth -= 1
        if return_values:
            merged = []
            for rv in return_values:
                merged.extend(rv)
            result = merged[:MAX_PATTERNS_PER_CALL]
        else:
            result = ANY_VALUE
        self.memo[key] = result
        return result

    def process_line(self, line, env, return_values):
        stripped = line.strip()
        if not stripped or stripped.startswith("'"):
            return

        for arg in find_calls(stripped, "os::appResourcePath"):
            try:
                value = self.eval_expr(arg, env)
            except RecursionError:
                value = ANY_VALUE
            self.collected.append(value)

        m = LET_RE.match(stripped) or LET_NOTYPE_RE.match(stripped)
        if m:
            name, rhs = m.group(1), strip_trailer(m.group(2))
            env[name] = self.eval_expr(rhs, env)
            return

        m = FOREACH_RE.match(stripped)
        if m:
            var, iterable = m.group(1), m.group(2)
            env[var] = self.eval_expr(iterable, env)
            return

        for expr in RETURN_RE.findall(stripped):
            return_values.append(self.eval_expr(strip_trailer(expr), env))

        # Bare call-statement (e.g. `loadCoastline()`), to trigger call-graph
        # side effects even outside an assignment/return.
        if CALL_RE.match(stripped):
            self.eval_expr(strip_trailer(stripped), env)

    def eval_expr(self, expr, env):
        expr = expr.strip()
        if not expr:
            return ANY_VALUE
        tokens = split_top_level(expr, "&")
        if len(tokens) == 1:
            return self.eval_atom(tokens[0].strip(), env)
        values = [self.eval_atom(t.strip(), env) for t in tokens]
        acc = values[0]
        for v in values[1:]:
            joined = []
            for a in acc:
                for b in v:
                    joined.append(a + b)
                    if len(joined) >= MAX_PATTERNS_PER_CALL:
                        break
                if len(joined) >= MAX_PATTERNS_PER_CALL:
                    break
            acc = joined
        return acc

    def eval_atom(self, tok, env):
        if len(tok) >= 2 and tok[0] == '"' and tok[-1] == '"':
            return [((("lit", unescape(tok[1:-1])),))]

        if tok.startswith("[") and tok.endswith("]"):
            inner = tok[1:-1]
            result = []
            for part in split_top_level(inner, ","):
                part = part.strip()
                if len(part) >= 2 and part[0] == '"' and part[-1] == '"':
                    result.append((("lit", unescape(part[1:-1])),))
                else:
                    result.append((("any",),))
            return result or ANY_VALUE

        if IDENT_RE.match(tok):
            return env.get(tok, ANY_VALUE)

        m = CALL_RE.match(tok)
        if m:
            funcname, args_str = m.group(1), m.group(2)
            arg_exprs = [a for a in split_top_level(args_str, ",") if a.strip()] if args_str.strip() else []
            arg_values = [self.eval_expr(a, env) for a in arg_exprs]
            resolved = self.resolve_call(funcname, arg_values)
            if resolved is not None:
                return resolved
            return ANY_VALUE

        return ANY_VALUE


def pattern_to_regex(pattern):
    parts = []
    exact = True
    literal_chunks = []
    for seg in pattern:
        if seg[0] == "lit":
            parts.append(re.escape(seg[1]))
            literal_chunks.append(seg[1])
        else:
            parts.append(".+")
            exact = False
    return re.compile("^" + "".join(parts) + "$"), exact, "".join(literal_chunks) if exact else None


def load_resources(project_json_path):
    data = json.loads(Path(project_json_path).read_text())
    example_dir = Path(project_json_path).parent
    out = []
    for res in data.get("resources", []):
        src_glob = res["src"]
        dst = res.get("dst", "")
        matches = sorted(globmod.glob(str(example_dir / src_glob)))
        shipped = {}
        for m in matches:
            shipped[dst + Path(m).name] = m
        out.append({"src": src_glob, "dst": dst, "shipped": shipped})
    return data, out


def audit_example(example_dir):
    example_dir = Path(example_dir)
    project_json = example_dir / "project.json"
    if not project_json.exists():
        return None
    data, resource_entries = load_resources(project_json)
    if not resource_entries:
        return None

    registry = Registry()
    for mfb in sorted(example_dir.rglob("*.mfb")):
        registry.parse_file(mfb.read_text())

    entry = data.get("entry", "main")
    auditor = Auditor(registry)
    auditor.resolve_call(entry, [])

    exact_referenced = set()
    wildcard_regexes = []
    for value in auditor.collected:
        for pattern in value:
            regex, exact, literal = pattern_to_regex(pattern)
            if exact:
                exact_referenced.add(literal)
            else:
                wildcard_regexes.append(regex)

    all_shipped = {}
    for entry_res in resource_entries:
        all_shipped.update(entry_res["shipped"])

    unreferenced = []
    for name in sorted(all_shipped):
        if name in exact_referenced:
            continue
        if any(r.match(name) for r in wildcard_regexes):
            continue
        unreferenced.append(name)

    missing = sorted(n for n in exact_referenced if n not in all_shipped)

    return {
        "example": str(example_dir),
        "shipped_count": len(all_shipped),
        "unreferenced": unreferenced,
        "missing": missing,
    }


def find_all_examples(root):
    root = Path(root)
    out = []
    for pj in sorted(root.glob("examples/*/project.json")):
        try:
            data = json.loads(pj.read_text())
        except Exception:
            continue
        if data.get("resources"):
            out.append(pj.parent)
    return out


def main(argv):
    if not argv or argv == ["--all"]:
        repo_root = Path(__file__).resolve().parents[2]
        example_dirs = find_all_examples(repo_root)
        if not example_dirs:
            print("no examples with a resources block found", file=sys.stderr)
            return 1
    else:
        example_dirs = [Path(a) for a in argv]

    failed = False
    for example_dir in example_dirs:
        result = audit_example(example_dir)
        if result is None:
            print(f"{example_dir}: no resources block, skipped")
            continue
        ok = not result["unreferenced"] and not result["missing"]
        status = "OK" if ok else "FAIL"
        print(f"{result['example']}: {status} ({result['shipped_count']} shipped, "
              f"{len(result['unreferenced'])} unreferenced, {len(result['missing'])} missing)")
        if result["unreferenced"]:
            print(f"  unreferenced ({len(result['unreferenced'])}):")
            for n in result["unreferenced"][:20]:
                print(f"    {n}")
            if len(result["unreferenced"]) > 20:
                print(f"    ... and {len(result['unreferenced']) - 20} more")
        if result["missing"]:
            print(f"  referenced but missing ({len(result['missing'])}):")
            for n in result["missing"]:
                print(f"    {n}")
        if not ok:
            failed = True

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
