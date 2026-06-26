#!/usr/bin/env python3
"""Print the built-in surface the compiler supports by scanning src/builtins/.

Each package is reported once, merging two sources:
  * the Rust builtins (``*.rs``) for the public ``pkg::name`` function/constant
    call surface, and
  * the MFBASIC package implementations (``*.mfb``) for the EXPORTed types.
"""

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).parent.parent
BUILTINS_DIR = REPO_ROOT / "src" / "builtins"
SKIP_FILES = {"mod.rs", "resource.rs"}

# FUNC/SUB/TYPE declarations in MFBASIC package sources, e.g.
#   EXPORT TYPE Instant
#   FUNC __datetime_now AS Instant
#   FUNC __collections_sort OF T(value AS List OF T) AS List OF T
DECL_RE = re.compile(
    r"^\s*(EXPORT\s+)?(FUNC|SUB|TYPE)\s+([A-Za-z_][A-Za-z0-9_]*)",
    re.MULTILINE,
)


def build_const_map(src: str) -> dict[str, str]:
    """Map Rust identifier → string value for const X: &str = "..." declarations."""
    return {
        m.group(1): m.group(2)
        for m in re.finditer(
            r"^\s*(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*\"([^\"]+)\"\s*;",
            src,
            re.MULTILINE,
        )
    }


def idents_in_fn(src: str, fn_name: str) -> list[str]:
    """Extract uppercase Rust identifiers from the body of the named function."""
    fn_re = re.compile(
        rf"\bfn\s+{re.escape(fn_name)}\s*\([^)]*\)[^{{]*\{{",
        re.DOTALL,
    )
    m = fn_re.search(src)
    if not m:
        return []
    start = m.end()
    depth = 1
    i = start
    while i < len(src) and depth > 0:
        c = src[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        i += 1
    body = src[start : i - 1]
    return re.findall(r"\b([A-Z][A-Z0-9_]*)\b", body)


def resolve(idents: list[str], const_map: dict[str, str]) -> list[str]:
    """Resolve Rust identifier names to their string values, preserving order, deduped."""
    seen: set[str] = set()
    result = []
    for ident in idents:
        val = const_map.get(ident)
        if val and val not in seen:
            seen.add(val)
            result.append(val)
    return result


def package_decls(src: str) -> list[tuple[str, str, bool]]:
    """Extract (kind, name, exported) for FUNC/SUB/TYPE in an MFBASIC source."""
    decls = []
    for m in DECL_RE.finditer(src):
        exported = bool(m.group(1))
        kind = m.group(2)
        name = m.group(3)
        decls.append((kind, name, exported))
    return decls


def package_label(stem: str) -> str:
    """Map an MFBASIC source stem to its package name (`http_package` → `http`)."""
    if stem.endswith("_package"):
        return stem[: -len("_package")]
    if stem == "regex_unicode":
        return "regex"
    return stem


def collect_packages() -> dict[str, dict[str, list[str]]]:
    """Build {label: {"functions", "constants", "types"}} across .rs and .mfb."""
    packages: dict[str, dict[str, list[str]]] = {}

    def bucket(label: str) -> dict[str, list[str]]:
        return packages.setdefault(
            label, {"functions": [], "constants": [], "types": []}
        )

    # Public call surface from the Rust builtins.
    for path in sorted(BUILTINS_DIR.glob("*.rs")):
        if path.name in SKIP_FILES:
            continue
        src = path.read_text()
        const_map = build_const_map(src)
        if not const_map:
            continue

        stem = path.stem
        label = "general" if stem == "general" else stem
        call_fn = "is_general_call" if stem == "general" else f"is_{stem}_call"

        functions = [n.replace(".", "::") for n in resolve(idents_in_fn(src, call_fn), const_map)]
        constants = (
            [n.replace(".", "::") for n in resolve(idents_in_fn(src, "is_math_constant"), const_map)]
            if stem == "math"
            else []
        )
        if functions or constants:
            b = bucket(label)
            b["functions"].extend(functions)
            b["constants"].extend(constants)

    # Exported types from the MFBASIC package implementations. The `__pkg_*`
    # helper FUNCs are internal — the public function surface comes from the
    # Rust scan above — so only EXPORTed declarations are listed here.
    for path in sorted(BUILTINS_DIR.glob("*.mfb")):
        label = package_label(path.stem)
        for kind, name, exported in package_decls(path.read_text()):
            if exported and kind == "TYPE":
                bucket(label)["types"].append(f"{label}::{name}")

    return packages


def main() -> None:
    packages = collect_packages()
    if not packages:
        print(f"No builtin source files found under {BUILTINS_DIR}", file=sys.stderr)
        sys.exit(1)

    total_fns = total_consts = total_types = 0

    for label in sorted(packages):
        b = packages[label]
        print(f"\n[{label}]")
        for name in b["functions"]:
            print(f"  {name}  (function)")
        for name in b["constants"]:
            print(f"  {name}  (constant)")
        for name in b["types"]:
            print(f"  {name}  (type)")

        total_fns += len(b["functions"])
        total_consts += len(b["constants"])
        total_types += len(b["types"])

    print(
        f"\nTotal: {total_fns} functions, {total_consts} constants, {total_types} types"
    )


if __name__ == "__main__":
    main()
