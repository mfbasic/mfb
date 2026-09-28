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
import difflib
import os
import re
import sys

VIS = r"(?:(?:PUBLIC|PRIVATE|EXPORT)\s+)?"
# `{name}` prefixes are Rust format!() placeholders splicing a prelude in front
# of a declaration (--rust-literals); MFBASIC itself never starts a line with `{`.
PLACEHOLDERS = r"(?:\{[A-Za-z_0-9]*\})*"
DECL_RE = re.compile(r"^(\s*" + PLACEHOLDERS + VIS + r"(?:ISOLATED\s+)?)(SUB)\s+(?=[A-Za-z_])", re.IGNORECASE)
END_RE = re.compile(r"\bEND(\s+)SUB\b", re.IGNORECASE)
EXIT_RE = re.compile(r"\bEXIT\s+SUB\b", re.IGNORECASE)
WORD_SUB_RE = re.compile(r"(?<![-\w])SUB(?![-\w])")
PROSE_SUB_RE = re.compile(r"\bSUB\b")
DOC_START_RE = re.compile(r"^\s*DOC(\s+[A-Za-z \t]*)?$", re.IGNORECASE)
DOC_HEADER_RE = re.compile(r"^(\s*)(SUB)\s+(?=[A-Za-z_])", re.IGNORECASE)
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


# --- Rust string literals (--rust-literals) ----------------------------------

# A literal is MFBASIC source when one of its lines is a SUB declaration header,
# or is exactly `END SUB` / `EXIT SUB` (a fragment of a program assembled from
# several literals). Anything else that mentions SUB (a diagnostic message, an
# assertion on one) is only reported.
SRC_LINE_RE = re.compile(r"^\s*" + PLACEHOLDERS + r"(?:" + VIS + r"(?:ISOLATED\s+)?SUB\s+[A-Za-z_]|END\s+SUB\s*$|EXIT\s+SUB\s*$)")
IDENT_CH = re.compile(r"[A-Za-z0-9_]")


def rust_scan(src):
    """Lex just enough Rust to find string literals, comments and `fn` bodies.

    Returns (literals, comments, fn_spans): literals are (start, end, raw) spans of
    the literal CONTENT (raw=True for r"…"/r#"…"#); comments are (start, end);
    fn_spans are (start, end) from the `fn` keyword to its body's closing brace.
    """
    lits, comments, fns = [], [], []
    i, n = 0, len(src)
    depth = 0
    pending_fn = []  # fn keyword offsets waiting for their body `{`
    open_fns = []  # (fn_start, depth_at_open)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j == -1 else j
            comments.append((i, j))
            i = j
            continue
        if src.startswith("/*", i):
            d, j = 1, i + 2
            while j < n and d:
                if src.startswith("/*", j):
                    d += 1; j += 2
                elif src.startswith("*/", j):
                    d -= 1; j += 2
                else:
                    j += 1
            comments.append((i, j))
            i = j
            continue
        prev_ident = i > 0 and IDENT_CH.match(src[i - 1])
        m = None if prev_ident else re.compile(r"(?:b|c)?r(#*)\"").match(src, i)
        if m:
            hashes = m.group(1)
            start = m.end()
            end = src.find('"' + hashes, start)
            lits.append((start, end, True))
            i = end + 1 + len(hashes)
            continue
        if c == '"' or (not prev_ident and c in "bc" and src.startswith('"', i + 1)):
            start = i + (1 if c == '"' else 2)
            j = start
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            lits.append((start, j, False))
            i = j + 1
            continue
        if c == "'":
            if src.startswith("\\", i + 1):
                j = src.find("'", i + 3)
                i = j + 1
                continue
            if i + 2 < n and src[i + 2] == "'":
                i += 3
                continue
            i += 1  # a lifetime
            continue
        if not prev_ident and src.startswith("fn", i) and (i + 2 >= n or not IDENT_CH.match(src[i + 2])):
            pending_fn.append(i)
            i += 2
            continue
        if c == "{":
            if pending_fn:
                open_fns.append((pending_fn.pop(), depth))
                pending_fn.clear()
            depth += 1
        elif c == "}":
            depth -= 1
            if open_fns and open_fns[-1][1] == depth:
                fns.append((open_fns.pop()[0], i + 1))
        elif c == ";":
            pending_fn.clear()  # a declaration without a body (trait item, extern)
        i += 1
    return lits, comments, fns


SIMPLE_ESCAPES = {"n": "\n", "t": "\t", '"': '"', "\\": "\\", "'": "'", "0": "\0", "r": "\r"}
ESC_RE = re.compile(r"\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f]+\}|\n[ \t]*|.)", re.DOTALL)


def _decode_mapped(raw):
    """Decode a Rust (non-raw) string body. Returns (text, starts, ends): decoded
    char k came from raw[starts[k]:ends[k]]. A line continuation (backslash,
    newline, indentation) decodes to nothing."""
    text, starts, ends = [], [], []
    i = 0
    while i < len(raw):
        if raw[i] == "\\":
            m = ESC_RE.match(raw, i)
            tok = m.group(0)
            if tok[1] == "\n":
                i = m.end()
                continue
            if tok[1] == "x":
                ch = chr(int(tok[2:], 16))
            elif tok[1] == "u":
                ch = chr(int(tok[3:-1], 16))
            elif tok[1] in SIMPLE_ESCAPES:
                ch = SIMPLE_ESCAPES[tok[1]]
            else:
                return None
            text.append(ch); starts.append(i); ends.append(m.end())
            i = m.end()
        else:
            text.append(raw[i]); starts.append(i); ends.append(i + 1)
            i += 1
    return "".join(text), starts, ends


def _encode(text):
    return text.replace("\\", "\\\\").replace('"', '\\"').replace("\t", "\\t").replace("\n", "\\n")


def rewrite_literal(content, raw, rw, first_line):
    """Rewrite one literal's content; returns the new content (or the same)."""
    fenced = "```" in content
    if raw and not fenced:
        return rw.rewrite_text(content, first_line)
    if raw:
        text, starts, ends = content, list(range(len(content))), list(range(1, len(content) + 1))
    else:
        dec = _decode_mapped(content)
        if dec is None:
            rw.report(first_line, "unhandled-escape", content[:120])
            return content
        text, starts, ends = dec
    if fenced:
        # A man page: only the fenced code blocks are MFBASIC.
        lines = text.split("\n")
        out, inside = [], False
        for l in lines:
            if l.strip().startswith("```"):
                inside = not inside
                out.append(l)
            elif inside:
                out.append(rw.rewrite_code_line(first_line, l))
            else:
                if WORD_SUB_RE.search(l):
                    rw.report(first_line, "doc-prose", l)
                out.append(l)
        new = "\n".join(out)
    else:
        new = rw.rewrite_text(text, first_line)
    if new == text:
        return content
    edits = []
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, text, new, autojunk=False).get_opcodes():
        if tag == "equal":
            continue
        r1 = starts[i1] if i1 < len(starts) else len(content)
        r2 = ends[i2 - 1] if i2 > i1 else r1
        edits.append((r1, r2, new[j1:j2] if raw else _encode(new[j1:j2])))
    out = content
    for r1, r2, rep_text in reversed(edits):
        out = out[:r1] + rep_text + out[r2:]
    return out


def rewrite_rust(src, rw, skip_fn_re=None):
    lits, comments, fns = rust_scan(src)
    out, last = [], 0
    line_of = lambda off: src.count("\n", 0, off) + 1
    for start, end, raw in lits:
        content = src[start:end]
        dec = None if raw else _decode_mapped(content)
        text = content if raw else (dec[0] if dec else content)
        # Test the DECODED text: in the raw form a `SUB` right after a `\n` escape
        # follows the letter `n`, which hides it from a word-boundary match.
        if not WORD_SUB_RE.search(text):
            continue
        if not any(SRC_LINE_RE.match(l) for l in text.split("\n")):
            rw.report(line_of(start), "rust-other", content[:160].replace("\n", "⏎"))
            continue
        if skip_fn_re is not None:
            enclosing = [f for f in fns if f[0] <= start < f[1]]
            if enclosing:
                fs, fe = min(enclosing, key=lambda f: f[1] - f[0])
                if skip_fn_re.search(src[fs:fe]):
                    rw.report(line_of(start), "skipped-fn", content[:80].replace("\n", "⏎"))
                    continue
        new = rewrite_literal(content, raw, rw, line_of(start))
        if new != content:
            out.append(src[last:start])
            out.append(new)
            last = end
    for cs, ce in comments:
        if PROSE_SUB_RE.search(src[cs:ce]):
            for k, l in enumerate(src[cs:ce].split("\n")):
                if PROSE_SUB_RE.search(l):
                    rw.report(line_of(cs) + k, "rust-comment", l.strip())
    out.append(src[last:])
    return "".join(out)


def iter_files(paths, ext=".mfb"):
    for p in paths:
        if os.path.isdir(p):
            for root, dirs, files in os.walk(p):
                dirs[:] = sorted(d for d in dirs if d not in (".git", "target", "build"))
                for f in sorted(files):
                    if f.endswith(ext):
                        yield os.path.join(root, f)
        else:
            yield p


def main(argv):
    ap = argparse.ArgumentParser(description="Rewrite MFBASIC SUB to FUNC … AS Nothing.")
    ap.add_argument("paths", nargs="+", help=".mfb files or directories (recursed for *.mfb)")
    ap.add_argument("--check", action="store_true", help="write nothing; exit 1 if any file would change")
    ap.add_argument("--quiet", action="store_true", help="do not print the report lines")
    ap.add_argument(
        "--rust-literals",
        action="store_true",
        help="treat inputs as Rust (*.rs): rewrite MFBASIC held in string literals",
    )
    ap.add_argument(
        "--skip-fn",
        metavar="REGEX",
        help="with --rust-literals: leave literals inside a fn whose text matches REGEX",
    )
    args = ap.parse_args(argv)
    skip_fn_re = re.compile(args.skip_fn) if args.skip_fn else None

    changed = []
    unhandled = 0
    for path in iter_files(args.paths, ".rs" if args.rust_literals else ".mfb"):
        with open(path, encoding="utf-8", newline="") as fh:
            src = fh.read()
        rw = Rewriter()
        if args.rust_literals:
            new = rewrite_rust(src, rw, skip_fn_re)
        else:
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
