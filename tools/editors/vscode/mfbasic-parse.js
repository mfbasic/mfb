// Pure, dependency-free parsing for MFBASIC editor features.
//
// These functions take source text and return plain data structures. They do
// NOT import the `vscode` module, so they can be unit-tested with plain Node.
// `extension.js` maps the results onto the VS Code provider APIs.
//
// The parsing is line-based and heuristic (no real parser), which is the right
// trade-off for symbols and folding: it must be fast, incremental-friendly, and
// tolerant of incomplete code while the user is typing. It mirrors src/lexer.rs
// where it matters (case-insensitive keywords, `'`/REM comments, verbatim DOC
// blocks, `END <modifier>` block terminators).

'use strict';

const MOD = '(?:(?:EXPORT|PRIVATE|ISOLATED)\\s+)*'; // optional leading modifiers

// Blocks that produce an outline symbol, with their explicit `END` terminator.
// `members` (when set) means leaf lines inside the block become child symbols
// of the given kind. `container` means nested FUNC/SUB declarations become
// child symbols (the LINK FFI block).
const SYMBOL_OPENERS = [
  { re: new RegExp('^\\s*' + MOD + 'FUNC\\s+([A-Za-z_]\\w*)', 'i'), kind: 'function', end: /^\s*END\s+FUNC\b/i, detail: true },
  { re: new RegExp('^\\s*' + MOD + 'SUB\\s+([A-Za-z_]\\w*)', 'i'), kind: 'method', end: /^\s*END\s+SUB\b/i, detail: true },
  { re: new RegExp('^\\s*' + MOD + 'TYPE\\s+([A-Za-z_]\\w*)', 'i'), kind: 'struct', end: /^\s*END\s+TYPE\b/i, members: 'field' },
  { re: new RegExp('^\\s*' + MOD + 'ENUM\\s+([A-Za-z_]\\w*)', 'i'), kind: 'enum', end: /^\s*END\s+ENUM\b/i, members: 'enummember' },
  { re: new RegExp('^\\s*' + MOD + 'UNION\\s+([A-Za-z_]\\w*)', 'i'), kind: 'struct', end: /^\s*END\s+UNION\b/i, members: 'field' },
  { re: /^\s*LINK\b.*?\bAS\s+([A-Za-z_]\w*)/i, kind: 'module', end: /^\s*END\s+LINK\b/i, container: true },
];

// A single-line declaration header (no body / no END).
const PACKAGE_RE = new RegExp('^\\s*' + MOD + '(?:PACKAGE|PROGRAM)\\s+([A-Za-z_][\\w.]*)', 'i');

// A DOC keyword line: `DOC` optionally followed by attribute words only.
const DOC_OPEN_RE = /^\s*DOC\b[ \t]*(?:[A-Za-z][A-Za-z \t]*)?$/i;
const DOC_END_RE = /^\s*END\s+DOC\b/i;

const COMMENT_RE = /^\s*'/;

function nameSpan(m) {
  // Every SYMBOL_OPENERS regex ends its match at the captured name, so the
  // name occupies the tail of m[0].
  const end = m.index + m[0].length;
  return { start: end - m[1].length, end };
}

// Parse the document into a nested array of outline symbols.
//   { name, kind, detail, startLine, endLine, nameLine, nameStart, nameEnd, children }
function parseSymbols(text) {
  const lines = text.split(/\r?\n/);
  const roots = [];
  const stack = []; // frames: { sym, end, members, container }

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    // Verbatim DOC block — skip its body entirely (it is not code).
    if (DOC_OPEN_RE.test(line) && !(stack.length && stack[stack.length - 1].members)) {
      let j = i + 1;
      while (j < lines.length && !DOC_END_RE.test(lines[j])) j++;
      i = Math.min(j, lines.length - 1);
      continue;
    }

    const top = stack[stack.length - 1];

    // Close the current block?
    if (top && top.end.test(line)) {
      top.sym.endLine = i;
      stack.pop();
      continue;
    }

    // Open a new symbol block? (LINK containers accept nested FUNC/SUB; other
    // blocks only nest when the current top is a container.)
    let opened = false;
    if (!top || top.container) {
      for (const o of SYMBOL_OPENERS) {
        const m = o.re.exec(line);
        if (!m) continue;
        const span = nameSpan(m);
        const sym = {
          name: m[1],
          kind: o.kind,
          detail: o.detail ? line.slice(span.end).trim() : '',
          startLine: i,
          endLine: i,
          nameLine: i,
          nameStart: span.start,
          nameEnd: span.end,
          children: [],
        };
        (top ? top.sym.children : roots).push(sym);
        stack.push({ sym, end: o.end, members: o.members, container: o.container });
        opened = true;
        break;
      }
    }
    if (opened) continue;

    // Single-line PACKAGE / PROGRAM header.
    if (!top) {
      const pm = PACKAGE_RE.exec(line);
      if (pm) {
        const span = nameSpan(pm);
        roots.push({
          name: pm[1], kind: 'module', detail: '',
          startLine: i, endLine: i, nameLine: i,
          nameStart: span.start, nameEnd: span.end, children: [],
        });
        continue;
      }
    }

    // Leaf member of a TYPE/ENUM/UNION (a field or enum variant).
    if (top && top.members && line.trim() !== '' && !COMMENT_RE.test(line)) {
      const mm = /^(\s*)([A-Za-z_]\w*)/.exec(line);
      if (mm) {
        top.sym.children.push({
          name: mm[2], kind: top.members, detail: line.slice(mm[1].length + mm[2].length).trim(),
          startLine: i, endLine: i, nameLine: i,
          nameStart: mm[1].length, nameEnd: mm[1].length + mm[2].length, children: [],
        });
      }
    }
  }

  // Anything still open at EOF runs to the last line.
  for (const f of stack) f.sym.endLine = Math.max(f.sym.endLine, lines.length - 1);
  return roots;
}

// Folding: structural blocks, DOC blocks, region markers, and comment runs.
const FOLD_OPENERS = [
  { open: new RegExp('^\\s*' + MOD + 'FUNC\\b', 'i'), close: /^\s*END\s+FUNC\b/i },
  { open: new RegExp('^\\s*' + MOD + 'SUB\\b', 'i'), close: /^\s*END\s+SUB\b/i },
  { open: new RegExp('^\\s*' + MOD + 'TYPE\\b', 'i'), close: /^\s*END\s+TYPE\b/i },
  { open: new RegExp('^\\s*' + MOD + 'ENUM\\b', 'i'), close: /^\s*END\s+ENUM\b/i },
  { open: new RegExp('^\\s*' + MOD + 'UNION\\b', 'i'), close: /^\s*END\s+UNION\b/i },
  { open: /^\s*LINK\b/i, close: /^\s*END\s+LINK\b/i },
  { open: /^\s*MATCH\b/i, close: /^\s*END\s+MATCH\b/i },
  { open: /^\s*TRAP\b/i, close: /^\s*END\s+TRAP\b/i },
  { open: /^\s*WITH\b/i, close: /^\s*END\s+WITH\b/i },
  // Block IF only: the line must END with THEN (optionally trailing comment).
  { open: /^\s*IF\b.*\bTHEN\s*(?:'.*)?$/i, close: /^\s*END\s+IF\b/i },
  { open: /^\s*FOR\b/i, close: /^\s*NEXT\b/i },
  // DO must be tested before WHILE so `DO WHILE` pushes one frame, not two.
  { open: /^\s*DO\b/i, close: /^\s*LOOP\b/i },
  { open: /^\s*WHILE\b/i, close: /^\s*END\s+WHILE\b/i },
];

const REGION_START_RE = /^\s*'\s*#region\b/i;
const REGION_END_RE = /^\s*'\s*#endregion\b/i;

// Returns [{ start, end, kind }] with kind one of undefined | 'comment' | 'region'.
function parseFoldingRanges(text) {
  const lines = text.split(/\r?\n/);
  const ranges = [];
  const stack = [];
  const regions = [];

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    // DOC block — fold as one unit and skip its interior.
    if (DOC_OPEN_RE.test(line)) {
      let j = i + 1;
      while (j < lines.length && !DOC_END_RE.test(lines[j])) j++;
      if (j < lines.length && j > i) ranges.push({ start: i, end: j, kind: 'comment' });
      i = Math.min(j, lines.length - 1);
      continue;
    }

    if (REGION_START_RE.test(line)) { regions.push(i); continue; }
    if (REGION_END_RE.test(line)) {
      if (regions.length) { const s = regions.pop(); if (i > s) ranges.push({ start: s, end: i, kind: 'region' }); }
      continue;
    }

    // Close the nearest matching open block.
    let closed = false;
    for (let s = stack.length - 1; s >= 0; s--) {
      if (stack[s].close.test(line)) {
        if (i > stack[s].start) ranges.push({ start: stack[s].start, end: i });
        stack.length = s; // pop this frame and any unmatched frames above it
        closed = true;
        break;
      }
    }
    if (closed) continue;

    // Open a block (first matching opener wins).
    for (const o of FOLD_OPENERS) {
      if (o.open.test(line)) { stack.push({ start: i, close: o.close }); break; }
    }
  }

  // Comment runs of two or more lines (excluding region-marker lines).
  let runStart = -1;
  for (let i = 0; i <= lines.length; i++) {
    const isComment = i < lines.length && COMMENT_RE.test(lines[i]) &&
      !REGION_START_RE.test(lines[i]) && !REGION_END_RE.test(lines[i]);
    if (isComment) { if (runStart === -1) runStart = i; }
    else { if (runStart !== -1 && i - 1 > runStart) ranges.push({ start: runStart, end: i - 1, kind: 'comment' }); runStart = -1; }
  }

  return ranges;
}

module.exports = { parseSymbols, parseFoldingRanges };
