#!/usr/bin/env python3
"""Inject a package's module-level doc_intro/doc_desc (from its package.md
overview, citations stripped) into src/codegen/builtins/<pkg>/mod.rs's
BuiltinModule literal. Usage: python3 .moddoc.py <pkg> <MODULE_CONST>"""
import re
import sys

pkg, const = sys.argv[1], sys.argv[2]
MOD = f"src/codegen/builtins/{pkg}/mod.rs"
md = open(f"src/docs/man/builtins/{pkg}/package.md").read()
cite = re.compile(r" *\[\[[^\]]*\]\]")
intro = cite.sub("", re.search(r"^# .*?\n\n(.+?)\n", md, re.S | re.M).group(1)).strip()
desc = cite.sub("", re.search(r"^## Description\n(.*?)(?=\n## |\Z)", md, re.S | re.M).group(1)).strip()


def raw(s):
    n = 1
    while ('"' + "#" * n) in s:
        n += 1
    h = "#" * n
    return f'r{h}"{s}"{h}'


src = open(MOD).read()
anchor = f"pub(crate) static {const}: BuiltinModule = BuiltinModule {{"
assert anchor in src, anchor
block = (
    f"const MODULE_INTRO: &str = {raw(intro)};\n"
    f"const MODULE_DESC: &str =\n{raw(desc)};\n\n"
)
src = src.replace(anchor, block + anchor, 1)
# Point the module's doc fields at the consts (only the BuiltinModule literal has
# these bare in mod.rs; member docs live in func_*.rs).
src = src.replace("    doc_intro: \"\",\n    doc_desc: \"\",", "    doc_intro: MODULE_INTRO,\n    doc_desc: MODULE_DESC,", 1)
open(MOD, "w").write(src)
print(f"{pkg}: module doc_intro/doc_desc injected")
