#!/usr/bin/env python3
"""Print all built-in functions the compiler supports by scanning src/builtins/."""

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).parent.parent
BUILTINS_DIR = REPO_ROOT / "src" / "builtins"
SKIP_FILES = {"mod.rs", "resource.rs"}


def build_const_map(src: str) -> dict[str, str]:
    """Map Rust identifier → string value for const X: &str = "..." declarations."""
    return {
        m.group(1): m.group(2)
        for m in re.finditer(
            r"^\s*const\s+([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*\"([^\"]+)\"\s*;",
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


def main() -> None:
    files = sorted(
        p for p in BUILTINS_DIR.glob("*.rs") if p.name not in SKIP_FILES
    )
    if not files:
        print(f"No builtin source files found under {BUILTINS_DIR}", file=sys.stderr)
        sys.exit(1)

    total_fns = 0
    total_consts = 0

    for path in files:
        src = path.read_text()
        const_map = build_const_map(src)
        if not const_map:
            continue

        stem = path.stem
        call_fn = "is_general_call" if stem == "general" else f"is_{stem}_call"
        functions = resolve(idents_in_fn(src, call_fn), const_map)

        constants: list[str] = []
        if stem == "math":
            constants = resolve(idents_in_fn(src, "is_math_constant"), const_map)

        if not functions and not constants:
            continue

        label = "general" if stem == "general" else stem
        print(f"\n[{label}]")
        for name in functions:
            print(f"  {name.replace('.', '::')}")
        for name in constants:
            print(f"  {name.replace('.', '::')}  (constant)")

        total_fns += len(functions)
        total_consts += len(constants)

    print(f"\nTotal: {total_fns} functions, {total_consts} constants")


if __name__ == "__main__":
    main()
