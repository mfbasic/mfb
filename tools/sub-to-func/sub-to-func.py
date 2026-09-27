#!/usr/bin/env python3
"""Rewrite MFBASIC source from `SUB` to `FUNC … AS Nothing` (plan-159).

Three rewrites, applied only to code (never inside a string, a char literal or a
comment):

    <vis>? ISOLATED? SUB <header>   ->  <vis>? ISOLATED? FUNC <header> AS Nothing
    END SUB                         ->  END FUNC
    EXIT SUB                        ->  RETURN

A `DOC … END DOC` block's header line (`SUB name(Type, …)`) becomes
`FUNC name(Type, …)` — a doc header names the callable, it carries no return type.
Code inside a DOC block's `EXAMPLE … END EXAMPLE` is rewritten like any other code.

Any `SUB` the tool does not rewrite (prose in a comment, a string, DOC text, or a
header shape it cannot classify) is printed as a report line for a human.

Usage: see README.md.
"""

import argparse
import os
import re
import sys

VIS = r"(?:(?:PUBLIC|PRIVATE|EXPORT)\s+)?"
DECL_RE = re.compile(r"^(\s*" + VIS + r"(?:ISOLATED\s+)?)(SUB)\b", re.IGNORECASE)
END_RE = re.compile(r"\bEND(\s+)SUB\b", re.IGNORECASE)
EXIT_RE = re.compile(r"\bEXIT\s+SUB\b", re.IGNORECASE)
WORD_SUB_RE = re.compile(r"\bSUB\b", re.IGNORECASE)
PROSE_SUB_RE = re.compile(r"\bSUB\b")
DOC_START_RE = re.compile(r"^\s*DOC(\s+[A-Za-z \t]*)?$", re.IGNORECASE)
DOC_HEADER_RE = re.compile(r"^(\s*)(SUB)\b", re.IGNORECASE)
REM_RE = re.compile(r"^\s*REM\b", re.IGNORECASE)


def _words(text):
    return text.split()


def _is_end(text, keyword):
    w = _words(text)
    return len(w) == 2 and w[0].upper() == "END" and w[1].upper() == keyword


def split_segments(line):
    """Split one source line into (kind, text) segments: 'code', 'str', 'comment'.

    Strings are `"…"` with backslash escapes; char literals are `` `…` `` with
    backslash escapes; a `'` outside a literal starts a comment to end of line.
    """
    segs = []
    i = 0
    start = 0
    n = len(line)
    while i < n:
        ch = line[i]
        if ch in "\"`":
            if i > start:
                segs.append(("code", line[start:i]))
            j = i + 1
            while j < n and line[j] != ch:
                j += 2 if line[j] == "\\" else 1
            j = min(j + 1, n)
            segs.append(("str", line[i:j]))
            i = start = j
            continue
        if ch == "'":
            if i > start:
                segs.append(("code", line[start:i]))
            segs.append(("comment", line[i:]))
            return segs
        i += 1
    if start < n:
        segs.append(("code", line[start:]))
    return segs


def header_end(code):
    """Index in `code` just past the SUB header (name, template, parameter list).

    Returns None when the header has trailing code the tool cannot classify.
    """
    paren = code.find("(")
    if paren != -1:
        depth = 0
        for k in range(paren, len(code)):
            if code[k] == "(":
                depth += 1
            elif code[k] == ")":
                depth -= 1
                if depth == 0:
                    end = k + 1
                    rest = code[end:]
                    if rest.strip() == "" or rest.lstrip().startswith(":"):
                        return end
                    return None
        return None  # unbalanced: a multi-line header
    colon = code.find(":")
    body = code if colon == -1 else code[:colon]
    return len(body.rstrip())


class Rewriter:
    def __init__(self):
        self.reports = []  # (line_no, reason, text)

    def report(self, line_no, reason, text):
        self.reports.append((line_no, reason, text.rstrip("\r\n")))

    def rewrite_code_line(self, line_no, line):
        """Rewrite one line of MFBASIC code; returns the new line."""
        if REM_RE.match(line):
            if PROSE_SUB_RE.search(line):
                self.report(line_no, "comment", line)
            return line
        # Header: the declaration keyword leads the line. Its end is found on a
        # copy with literals blanked out and the comment cut, so a string default
        # or a trailing comment cannot be mistaken for header syntax.
        m = DECL_RE.match(line)
        if m:
            masked = "".join(
                text if kind == "code" else ("_" * len(text) if kind == "str" else "")
                for kind, text in split_segments(line)
            )
            end = header_end(masked)
            if end is None:
                self.report(line_no, "unhandled-header", line)
                return line
            func = "func" if m.group(2).islower() else "FUNC"
            line = line[: m.start(2)] + func + line[m.end(2) : end] + " AS Nothing" + line[end:]
        segs = split_segments(line)
        out = []
        for kind, text in segs:
            if kind == "code":
                text = EXIT_RE.sub(lambda mm: "RETURN" if mm.group(0)[0].isupper() else "return", text)
                text = END_RE.sub(lambda mm: mm.group(0)[:-3] + ("FUNC" if mm.group(0)[-3:].isupper() else "func"), text)
                if WORD_SUB_RE.search(text):
                    self.report(line_no, "unhandled-code", line)
            elif PROSE_SUB_RE.search(text):
                self.report(line_no, "comment" if kind == "comment" else "string", line)
            out.append(text)
        return "".join(out)

    def rewrite_text(self, text, first_line=1):
        """Rewrite a whole MFBASIC source text. Line endings are preserved."""
        lines = text.splitlines(keepends=True)
        out = []
        in_doc = False
        in_example = False
        doc_header_pending = False
        for idx, raw in enumerate(lines):
            line_no = first_line + idx
            body = raw.rstrip("\r\n")
            eol = raw[len(body) :]
            if in_doc:
                if in_example:
                    if _is_end(body, "EXAMPLE"):
                        in_example = False
                        out.append(raw)
                    else:
                        out.append(self.rewrite_code_line(line_no, body) + eol)
                    continue
                if _is_end(body, "DOC"):
                    in_doc = False
                    out.append(raw)
                    continue
                w = _words(body)
                if len(w) == 1 and w[0].upper() == "EXAMPLE":
                    in_example = True
                    out.append(raw)
                    continue
                if doc_header_pending and body.strip():
                    doc_header_pending = False
                    m = DOC_HEADER_RE.match(body)
                    if m:
                        func = "func" if m.group(2).islower() else "FUNC"
                        body = body[: m.start(2)] + func + body[m.end(2) :]
                        out.append(body + eol)
                        continue
                if PROSE_SUB_RE.search(body):
                    self.report(line_no, "doc-text", body)
                out.append(raw)
                continue
            if DOC_START_RE.match(body):
                in_doc = True
                in_example = False
                doc_header_pending = True
                out.append(raw)
                continue
            out.append(self.rewrite_code_line(line_no, body) + eol)
        return "".join(out)


def iter_files(paths):
    for p in paths:
        if os.path.isdir(p):
            for root, dirs, files in os.walk(p):
                dirs[:] = sorted(d for d in dirs if d not in (".git", "target", "build"))
                for f in sorted(files):
                    if f.endswith(".mfb"):
                        yield os.path.join(root, f)
        else:
            yield p


def main(argv):
    ap = argparse.ArgumentParser(description="Rewrite MFBASIC SUB to FUNC … AS Nothing.")
    ap.add_argument("paths", nargs="+", help=".mfb files or directories (recursed for *.mfb)")
    ap.add_argument("--check", action="store_true", help="write nothing; exit 1 if any file would change")
    ap.add_argument("--quiet", action="store_true", help="do not print the report lines")
    args = ap.parse_args(argv)

    changed = []
    unhandled = 0
    for path in iter_files(args.paths):
        with open(path, encoding="utf-8", newline="") as fh:
            src = fh.read()
        rw = Rewriter()
        new = rw.rewrite_text(src)
        for line_no, reason, text in rw.reports:
            if reason.startswith("unhandled"):
                unhandled += 1
            if not args.quiet or reason.startswith("unhandled"):
                print(f"{path}:{line_no}: [{reason}] {text.strip()}")
        if new != src:
            changed.append(path)
            if not args.check:
                with open(path, "w", encoding="utf-8", newline="") as fh:
                    fh.write(new)
    verb = "would change" if args.check else "changed"
    print(f"sub-to-func: {len(changed)} file(s) {verb}; {unhandled} unhandled line(s)", file=sys.stderr)
    if unhandled:
        return 2
    if args.check and changed:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
