// generate.mjs — the seeded random-tree generator behind the fuzz modes.
//
// The trees are the INPUT to both directions: `fuzz-write` hands one to each
// side's writer and reads the output back, and `fuzz-read` writes one out in
// varied styles and checks that every side reads the same content.
//
// Everything is drawn from a seeded PRNG so a failure replays exactly: the
// runner prints the seed and the case index on every failure.
//
// What the draws deliberately include, because these are where readers and
// writers actually differ:
//   * names from non-ASCII NameStartChar ranges, not just ASCII;
//   * declared prefixes, so namespaced names appear;
//   * text holding `]]>`, `&`, `<`, CR, tab, and non-ASCII scalars;
//   * whitespace-only text as an element's SOLE child (data) and beside
//     elements (layout) — the distinction plan-138-A §4 turns on;
//   * comments and processing instructions interleaved with elements.

/** mulberry32: small, seeded, and identical across runs. */
export function rng(seed) {
  let state = seed >>> 0;
  return function next() {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const NAME_STARTS = [
  "a", "b", "item", "節", "élan", "Ωmega", "_x", "Ä", "ラベル", "naïve",
];
const TEXTS = [
  "plain",
  "a & b",
  "a < b",
  "a > b",
  "]]>",
  "a ]] b",
  "line\nbreak",
  "carriage\rreturn",
  "tab\there",
  "  ",
  "\n  ",
  "é\u{10348}ü",
  "",
  "trailing ",
  " leading",
];

function pick(next, list) {
  return list[Math.floor(next() * list.length) % list.length];
}

function name(next, prefixes) {
  const local = pick(next, NAME_STARTS) + (next() < 0.3 ? String(Math.floor(next() * 90)) : "");
  if (prefixes.length > 0 && next() < 0.35) {
    return `${pick(next, prefixes)}:${local}`;
  }
  return local;
}

function attributes(next, prefixes) {
  const out = [];
  const count = Math.floor(next() * 3);
  for (let index = 0; index < count; index += 1) {
    out.push([name(next, prefixes), pick(next, TEXTS)]);
  }
  // No duplicates: the package refuses them, so a generated duplicate would
  // only ever test the refusal path, which the corpus already covers.
  const seen = new Set();
  return out.filter(([key]) => (seen.has(key) ? false : (seen.add(key), true)));
}

/**
 * One random element subtree, as the §3 envelope shape plus comments and PIs
 * (which the content projection drops but the writers must still handle).
 */
function element(next, depth, prefixes) {
  const children = [];
  const count = depth <= 0 ? 0 : Math.floor(next() * 4);
  // An element either holds data text or holds elements — mixing them is
  // allowed and generated, but a sole whitespace child is generated too,
  // because that is the case the layout/data rule turns on.
  if (count === 0 && next() < 0.5) {
    children.push(["t", pick(next, TEXTS)]);
  } else {
    for (let index = 0; index < count; index += 1) {
      const draw = next();
      if (draw < 0.55) children.push(element(next, depth - 1, prefixes));
      else if (draw < 0.75) children.push(["t", pick(next, TEXTS)]);
      else if (draw < 0.9) children.push(["c", "note " + Math.floor(next() * 100)]);
      else children.push(["p", "target" + Math.floor(next() * 10), pick(next, ["", "data", "a b"])]);
    }
  }
  return ["e", name(next, prefixes), attributes(next, prefixes), children.filter(Boolean)];
}

/** A whole random document: exactly one root, with prefixes declared on it. */
export function tree(next) {
  const prefixes = [];
  const prefixCount = Math.floor(next() * 3);
  for (let index = 0; index < prefixCount; index += 1) prefixes.push(`p${index}`);

  const root = element(next, 3, prefixes);
  // Declare every prefix the tree may use, on the root: an undeclared prefix is
  // refused by the package, and this mode is about agreement on documents all
  // three sides accept.
  const declarations = prefixes.map((prefix) => [`xmlns:${prefix}`, `urn:test:${prefix}`]);
  root[2] = [...declarations, ...root[2]];

  const children = [];
  if (next() < 0.3) children.push(["c", "prolog"]);
  children.push(root);
  if (next() < 0.2) children.push(["p", "epilog", "data"]);
  return ["doc", children];
}

/** `count` trees from `seed`, as {id, tree} cases. */
export function trees(seed, count) {
  const next = rng(seed);
  const out = [];
  for (let index = 0; index < count; index += 1) {
    out.push({ id: `seed${seed}-${index}`, tree: tree(next) });
  }
  return out;
}

// ---------------------------------------------------------------------------
// Writing a tree out as XML, in varied styles.
//
// This is the `fuzz-read` direction: the SAME tree is serialized several ways,
// and every style must read back with the same content. The styles are the ones
// where readers differ — quoting, CDATA, character references, empty-element
// form, and layout whitespace.
// ---------------------------------------------------------------------------

function escapeText(text, style) {
  // CDATA cannot carry `]]>` (it would close the section) and cannot carry a
  // CARRIAGE RETURN either: XML 1.0 §2.11 normalizes line ends across the whole
  // entity, CDATA included, so a literal CR always reads back as LF. Only a
  // character reference preserves one, so such text falls through to escaping.
  if (style.cdata && text !== "" && !text.includes("]]>") && !text.includes("\r")) {
    return `<![CDATA[${text}]]>`;
  }
  let out = "";
  for (const character of text) {
    const code = character.codePointAt(0);
    if (character === "&") out += "&amp;";
    else if (character === "<") out += "&lt;";
    else if (character === ">") out += "&gt;";
    else if (character === "\r") out += "&#13;";
    else if (style.charRefs && code > 127) out += `&#x${code.toString(16)};`;
    else out += character;
  }
  return out;
}

function escapeAttribute(value, style) {
  let out = "";
  for (const character of value) {
    const code = character.codePointAt(0);
    if (character === "&") out += "&amp;";
    else if (character === "<") out += "&lt;";
    else if (character === style.quote) out += style.quote === '"' ? "&quot;" : "&apos;";
    else if (character === "\t") out += "&#9;";
    else if (character === "\n") out += "&#10;";
    else if (character === "\r") out += "&#13;";
    else if (style.charRefs && code > 127) out += `&#x${code.toString(16)};`;
    else out += character;
  }
  return out;
}

function writeNode(node, style, level) {
  const [kind] = node;
  if (kind === "t") return escapeText(node[1], style);
  if (kind === "c") return `<!--${node[1]}-->`;
  if (kind === "p") return node[2] === "" ? `<?${node[1]}?>` : `<?${node[1]} ${node[2]}?>`;

  const [, name, attributes, children] = node;
  const written = attributes
    .map(([key, value]) => ` ${key}=${style.quote}${escapeAttribute(value, style)}${style.quote}`)
    .join("");
  if (children.length === 0) {
    return style.longEmpty ? `<${name}${written}></${name}>` : `<${name}${written}/>`;
  }

  // Layout whitespace may be inserted only where it cannot become DATA, and the
  // rule is plan-138-A §4 step 3: whitespace-only text is layout only when the
  // parent also has an ELEMENT child. An element whose children are just
  // comments or PIs has none, so indenting it would give it text it never had —
  // which is exactly what this generator caught itself doing.
  const hasElementChild = children.some(([childKind]) => childKind === "e");
  const hasDataText = children.some(
    ([childKind, value]) => childKind === "t" && value !== "" && !/^[ \t\r\n]+$/.test(value),
  );
  const indent = style.indent && hasElementChild && !hasDataText;
  const inner = children
    .map((child) => (indent ? "\n" + style.indent.repeat(level + 1) : "") + writeNode(child, style, level + 1))
    .join("");
  const close = indent ? "\n" + style.indent.repeat(level) : "";
  return `<${name}${written}>${inner}${close}</${name}>`;
}

export const STYLES = [
  { name: "compact", quote: '"', indent: "", cdata: false, charRefs: false, longEmpty: false },
  { name: "indent-2", quote: '"', indent: "  ", cdata: false, charRefs: false, longEmpty: false },
  { name: "indent-tab", quote: '"', indent: "\t", cdata: false, charRefs: false, longEmpty: true },
  { name: "single-quoted", quote: "'", indent: "", cdata: false, charRefs: false, longEmpty: false },
  { name: "cdata", quote: '"', indent: "", cdata: true, charRefs: false, longEmpty: false },
  { name: "char-refs", quote: '"', indent: "", cdata: false, charRefs: true, longEmpty: false },
];

/** Serialize a whole document tree in one style. */
export function write(documentTree, style) {
  const [, children] = documentTree;
  const body = children
    .map((child) => writeNode(child, style, 0))
    .join(style.indent ? "\n" : "");
  return `<?xml version="1.0" encoding="UTF-8"?>${style.indent ? "\n" : ""}${body}`;
}
