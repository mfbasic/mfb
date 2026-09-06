#!/usr/bin/env python3
"""yaml_oracle_diff.py — differential-test `packages/yaml` against PyYAML.

The SECOND of two independent oracles. The first is `packages/yaml/oracle`, a
Node project built on eemeli/yaml pinned to YAML **1.2 Core** — the same
language `packages/yaml` documents — which makes it the sharper instrument:
every divergence there is nearly always a finding. PyYAML is YAML **1.1**, so
most of its disagreements are spec-version noise, listed and checked in
KNOWN_DIVERGENCES below.

Keep it anyway. Two differently-wrong references are worth more than one,
because where they disagree with EACH OTHER is exactly where the spec is worth
re-reading — that is how the lone-carriage-return question got settled (PyYAML
and the spec say it is a line break; the Node oracle refuses such a file).

This script found two real bugs of its own: a literal block scalar dropping the
blank lines inside it, and the `examples/yaml-json` emitter writing a
root-level block scalar with no indentation. It is not part of any gate — it
needs PyYAML installed.

    pip install pyyaml
    mfb build packages/yaml
    mkdir -p examples/yaml-json/packages
    cp packages/yaml/yaml.mfp examples/yaml-json/packages/yaml.mfp
    mfb build examples/yaml-json
    python3 scripts/yaml_oracle_diff.py          # every mode
    python3 scripts/yaml_oracle_diff.py corpus   # one mode

Modes:

  corpus     hand-written YAML covering the whole supported subset, read by both
  fuzz       random values dumped BY PyYAML, read by both
  emit       random values written as YAML by the example's emitter, read by PyYAML
  roundtrip  random values through emitter -> reader, compared with themselves

Exit status is 0 iff every disagreement is one of the KNOWN ones below.

PyYAML implements YAML **1.1**; `packages/yaml` implements the **1.2 Core
Schema**. Where the two versions genuinely differ, this reader is the one
following its documented schema, so those differences are listed as expected
rather than treated as failures — see KNOWN_DIVERGENCES.
"""

import json
import os
import random
import subprocess
import sys

try:
    import yaml
except ImportError:  # pragma: no cover - the message IS the handling
    sys.exit("this script needs PyYAML as an independent oracle: pip install pyyaml")

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "examples", "yaml-json", "build", "yamljson.out")
SCRATCH = os.path.join(ROOT, "target", "yaml-oracle")

# Inputs the two implementations are SUPPOSED to disagree about, each with the
# reason. Anything not in here that disagrees is a failure.
KNOWN_DIVERGENCES = {
    # YAML 1.1 typed these; the 1.2 Core Schema does not.
    "1.1 booleans": "yes/no/on/off/y/n are booleans in 1.1, strings in 1.2 Core",
    "1.1 sexagesimal": "12:30 is 750 in 1.1; 1.2 Core has no base-60 form",
    "1.2-only numbers": "0o17 and exponent-without-a-dot (1e3) are numbers only in 1.2 Core",
    # Not a schema difference: the JSON model itself.
    "float model": "json::JsonNum holds a Float, so a >2^53 integer rounds and 1.0 renders as 1",
    # Deliberate rejections, documented in packages/yaml/README.md.
    "tags": "tags are outside the supported subset and are rejected",
    "explicit keys": "an explicit `?` key is outside the supported subset",
}


def run(*args):
    return subprocess.run([BIN, *args], capture_output=True, text=True)


def canon(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), default=str)


def write(name, text):
    os.makedirs(SCRATCH, exist_ok=True)
    path = os.path.join(SCRATCH, name)
    with open(path, "w") as handle:
        handle.write(text)
    return path


def pyyaml_load(text):
    """(ok, value) — the whole stream, matching the converter's own rule that a
    multi-document stream becomes an array."""
    try:
        documents = list(yaml.safe_load_all(text))
    except Exception as problem:  # noqa: BLE001 - any parse failure is "rejected"
        return False, str(problem).splitlines()[0]
    return True, documents[0] if len(documents) == 1 else documents


# ---------------------------------------------------------------------------
# corpus — the hand-written cases
# ---------------------------------------------------------------------------

CORPUS = [
    # block mappings and sequences
    "a: 1", "a: 1\nb: 2", "a:\n  b: 1", "a:\n  - 1\n  - 2", "a:\n- 1\n- 2",
    "- 1\n- 2", "- a: 1\n  b: 2", "- - 1\n- 2", "- - - 1\n", "- \n- a\n",
    "a:\n  b:\n    c:\n      d: 1\n", "a:\n- b:\n  - c:\n    - 1\n",
    "a:\n  - 1\n  -\n  - 3\n", "  a: 1\n  b: 2\n",
    "a: 1\nb:\n  - x: 1\n    y: 2\n  - z: 3\n", "top:\n  - - a\n    - b\n  - c\n",
    "key with spaces: 1", "empty:\nother: 1", "outer:\n  inner:\n  - 1\n  - 2\n",
    # flow collections
    "[]", "{}", "a: []", "a: {}", "a: [1, 2, 3]", "a: {b: 1, c: 2}",
    "a: [1, [2, 3], {d: 4}]", "a: [[[1]]]", "a: {b: {c: {d: 1}}}",
    "a: [ 1 , 2 ,]", "a: [a: 1]\n", "empty_flow: [ ]\nempty_map: { }\n",
    "seq:\n  - {k: v}\n  - [1,2]\n", "matrix:\n  - [1, 2]\n  - [3, 4]\n",
    "a: [{b: 1}, {c: 2}]",
    # scalars
    "a: 'x'\nb: \"y\"", "a: hello world", "a: hello\n  world", "a: hello\n\n  world",
    "a: 'it''s'", 'a: "q\\"q"', 'a: "\\u00e9"', 'u: "\\u00e9\\u4e2d"\n', 't: "a\\\\b"\n',
    "a: 12:30", "a: http://example.com/x", "a: -x", "a: \"\"\nb: ''\n",
    "a: \" leading\"\n", "a: \"multi\n  line\"\n", "a: 'multi\n  line'\n",
    "a: 0x1F\nb: 0o17\nc: 1e3\nd: .5\ne: -0.5", "a: yes\nb: no\nc: on\nd: off",
    "x: 1.0\ny: 100.0\nz: 1e100\n", "n: 1\nnn: -1\nnnn: +1\n", "e: 1E+3\n",
    "neg0: -0\n", "big: 123456789012345678901234567890\n", "d: 2026-09-05\n",
    "a: \"tab\\there\"", "'quoted key': 1\n\"dq key\": 2\n", "a: 'x: y'\n",
    # block scalars
    "a: |\n  x\n  y\n", "a: |-\n  x\n  y\n", "a: |+\n  x\n\n",
    "a: >\n  x\n  y\n", "a: >-\n  x\n  y\n", "a: >\n  x\n\n  y\n",
    "a: >\n  x\n   deep\n  y\n", "a: >\n  x\n    deep\n  y\n",
    "a: |2\n    x\n", "a: |\n\n  x\n", "s: |\n  a\n\n  b\n",
    "a: |\n  x\n\n\n", "a: |-\n  x\n\n\n", "a: |+\n  x\n\n\n",
    "a: |\n    indented\n      more\n    back\n",
    "a: >-\n  x\n\n\n  y\n", "s: >\n\n  after blank\n",
    "x:\n  y: |\n    text\n  z: 2\n",
    # comments, blanks, line endings
    "# only a comment\na: 1 # trailing", "a: # comment\n  b: 1\n", "a:\n\n  b: 1\n",
    "a: 1\n#comment\nb: 2\n", "root:\n  # comment\n  k: v\n", "a: 1\n\n\nb: 2\n",
    "a: 1    # c\nb: 2\n", "a: 1\r\nb: 2\r\n",
    # documents, directives, anchors
    "---\na: 1\n", "---\na: 1\n...\n", "a: 1\n---\nb: 2\n", "--- 5\n--- 6\n",
    "%YAML 1.2\n---\na: 1\n", "---\n# just a comment\n", "--- {a: 1}\n",
    "--- [1,2]\n", "--- |\n  block\n",
    "a: &x 1\nb: *x", "a: &x [1,2]\nb: *x", "list:\n  - &i {n: 1}\n  - *i",
    "key: &a\n  x: 1\nother: *a\n", "l: &a [1, 2]\nm: [*a, *a]\n",
    # outside the subset — both sides should NOT agree, and that is expected
    "a: !!null ''\n", "? explicit\n: value\n",
]

# The corpus entries that MUST disagree, and which KNOWN_DIVERGENCES row each
# one demonstrates. A case in here that suddenly AGREES is reported too: it
# means the reader stopped following the schema it documents.
EXPECTED = {
    "a: 12:30": "1.1 sexagesimal",
    "a: 0x1F\nb: 0o17\nc: 1e3\nd: .5\ne: -0.5": "1.2-only numbers",
    "a: yes\nb: no\nc: on\nd: off": "1.1 booleans",
    "x: 1.0\ny: 100.0\nz: 1e100\n": "float model",
    "e: 1E+3\n": "1.2-only numbers",
    "big: 123456789012345678901234567890\n": "float model",
    "a: !!null ''\n": "tags",
    "? explicit\n: value\n": "explicit keys",
}


def mode_corpus():
    failures = 0
    for index, source in enumerate(CORPUS):
        reason = EXPECTED.get(source)
        path = write("corpus.yaml", source)
        result = run("to-json", path, "--compact")
        ok, expected = pyyaml_load(source)
        agreed = (
            result.returncode == 0
            and ok
            and canon(json.loads(result.stdout)) == canon(expected)
        )
        if agreed:
            if reason:
                print(f"[corpus {index}] expected to diverge ({reason}) but AGREED: {source!r}")
                failures += 1
            continue
        if reason:
            continue
        if result.returncode != 0:
            first = (result.stderr.strip().splitlines() or [""])[0]
            print(f"[corpus {index}] mfb rejects, oracle accepts: {source!r}")
            print(f"    mfb: {first}")
            print(f"    py : {canon(expected) if ok else expected}")
        elif not ok:
            print(f"[corpus {index}] mfb accepts, oracle rejects: {source!r}")
            print(f"    mfb: {result.stdout.strip()}")
            print(f"    py : {expected}")
        else:
            print(f"[corpus {index}] mismatch: {source!r}")
            print(f"    mfb: {canon(json.loads(result.stdout))}")
            print(f"    py : {canon(expected)}")
        failures += 1
    return len(CORPUS), failures


# ---------------------------------------------------------------------------
# the random-value modes
# ---------------------------------------------------------------------------

WORDS = [
    "alpha", "beta", "gamma", "key name", "x", "a b c", "",
    "line1\nline2\n", "line1\nline2", "yes", "no", "true", "null", "12", "1.5",
    "- dash", "#hash", "a: b", "tab\there", "unicode \u00e9\u4e2d\U0001F600",
    "  padded  ", "end:", "*star", "&amp", "[brack]", "{brace}",
    "q'single", 'q"double', "back\\slash", ".inf", ".nan", "0x1F", "~",
    "---", "...",
]


def random_value(depth=0):
    roll = random.random()
    if depth > 3 or roll < 0.35:
        pick = random.random()
        if pick < 0.48:
            return random.choice(WORDS)
        if pick < 0.65:
            return random.randint(-10 ** 9, 10 ** 9)
        if pick < 0.78:
            return round(random.uniform(-1e4, 1e4), 6)
        if pick < 0.9:
            return random.choice([True, False])
        return None
    if roll < 0.68:
        return [random_value(depth + 1) for _ in range(random.randint(0, 4))]
    return {
        "k%d %s" % (i, random.choice(["a", "b c", "d-e", ":colon", "#h"])): random_value(depth + 1)
        for i in range(random.randint(0, 4))
    }


def mode_fuzz(count=300):
    """PyYAML writes, both read. PyYAML's dumper quotes anything its own 1.1
    schema would re-type, so its output is inside both schemas and the two
    readers must agree exactly."""
    failures = 0
    for index in range(count):
        value = random_value()
        for flow in (False, True):
            text = yaml.safe_dump(value, default_flow_style=flow, allow_unicode=True, width=40)
            path = write("fuzz.yaml", text)
            result = run("to-json", path, "--compact")
            if result.returncode != 0:
                print(f"[fuzz {index} flow={flow}] mfb rejects PyYAML's own output: {text!r}")
                print(f"    {result.stderr.strip()}")
                failures += 1
                continue
            got = json.loads(result.stdout)
            if canon(got) != canon(yaml.safe_load(text)):
                print(f"[fuzz {index} flow={flow}] mismatch: {text!r}")
                print(f"    mfb: {canon(got)}")
                print(f"    py : {canon(yaml.safe_load(text))}")
                failures += 1
    return count * 2, failures


def mode_emit(count=300):
    """The example's emitter writes, PyYAML reads. This is what proves the
    emitter's quoting is conservative enough for a reader that is NOT ours."""
    failures = 0
    for index in range(count):
        value = random_value()
        path = write("emit.json", json.dumps(value))
        result = run("to-yaml", path)
        if result.returncode != 0:
            print(f"[emit {index}] emitter failed on {json.dumps(value)}")
            print(f"    {result.stderr.strip()}")
            failures += 1
            continue
        try:
            back = yaml.safe_load(result.stdout)
        except Exception as problem:  # noqa: BLE001
            print(f"[emit {index}] oracle cannot read the emitted YAML:\n{result.stdout}    {problem}")
            failures += 1
            continue
        if canon(back) != canon(value):
            print(f"[emit {index}] round-trip mismatch: {json.dumps(value)}")
            print(f"--- emitted ---\n{result.stdout}--- read back ---\n{canon(back)}")
            failures += 1
    return count, failures


def mode_roundtrip(count=300):
    """Emitter then reader, both ours. Catches a self-consistent pair of bugs
    only when combined with `emit`, but catches an emitter/reader disagreement
    on its own."""
    failures = 0
    for index in range(count):
        value = random_value()
        json_path = write("rt.json", json.dumps(value))
        emitted = run("to-yaml", json_path)
        if emitted.returncode != 0:
            print(f"[roundtrip {index}] emitter failed: {emitted.stderr.strip()}")
            failures += 1
            continue
        yaml_path = write("rt.yaml", emitted.stdout)
        reread = run("to-json", yaml_path, "--compact")
        if reread.returncode != 0:
            print(f"[roundtrip {index}] reader rejects our own YAML:\n{emitted.stdout}    {reread.stderr.strip()}")
            failures += 1
            continue
        if canon(json.loads(reread.stdout)) != canon(value):
            print(f"[roundtrip {index}] mismatch: {json.dumps(value)}")
            print(f"--- emitted ---\n{emitted.stdout}--- read back ---\n{reread.stdout}")
            failures += 1
    return count, failures


MODES = {
    "corpus": mode_corpus,
    "fuzz": mode_fuzz,
    "emit": mode_emit,
    "roundtrip": mode_roundtrip,
}


def main():
    if not os.path.isfile(BIN):
        sys.exit(
            f"{BIN} is missing. Build it first:\n"
            "  mfb build packages/yaml\n"
            "  mkdir -p examples/yaml-json/packages\n"
            "  cp packages/yaml/yaml.mfp examples/yaml-json/packages/yaml.mfp\n"
            "  mfb build examples/yaml-json"
        )
    wanted = sys.argv[1:] or list(MODES)
    unknown = [name for name in wanted if name not in MODES]
    if unknown:
        sys.exit(f"unknown mode(s) {unknown}; pick from {list(MODES)}")

    random.seed(20260905)
    total_failures = 0
    for name in wanted:
        cases, failures = MODES[name]()
        total_failures += failures
        print(f"{name}: {cases} cases, {failures} disagreement(s)")

    if "corpus" in wanted:
        print()
        print(f"{len(EXPECTED)} corpus cases are EXPECTED to diverge and were checked as such")
        print("(PyYAML is YAML 1.1; this reader is the 1.2 Core Schema):")
        for label, why in KNOWN_DIVERGENCES.items():
            print(f"  - {label}: {why}")
    return 1 if total_failures else 0


if __name__ == "__main__":
    sys.exit(main())
