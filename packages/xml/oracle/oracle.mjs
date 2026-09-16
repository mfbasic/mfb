// oracle.mjs — the Node side of the XML differential oracle.
//
// Reading is done with `saxes`, a strict streaming XML parser, and NOT with
// @xmldom/xmldom's own parser, which is lenient: a lenient reader would agree
// with our acceptance bugs, which is the one failure mode an oracle may not
// have. xmldom appears only in the write direction, as a DOM and serializer.
//
// The declaration checks (version, encoding) are done HERE rather than left to
// saxes. plan-138-C §2 records that whether saxes refuses `version="1.1"` or a
// non-UTF-8 encoding on its own is UNVERIFIED, and policy agreement between the
// three sides must not rest on an unverified library behaviour.
//
// A refusal is a result: every entry point returns an envelope, never throws.

import { SaxesParser } from "saxes";
import { DOMImplementation, XMLSerializer } from "@xmldom/xmldom";
import xpathLib from "xpath";

/** Refusal envelope. `kind` is triage only; the runner never compares it. */
function refuse(kind, reason) {
  return { ok: false, kind, reason: String(reason) };
}

// XML 1.0 §2.2 Char, as a predicate over a code point.
function isChar(code) {
  return (
    code === 0x9 ||
    code === 0xa ||
    code === 0xd ||
    (code >= 0x20 && code <= 0xd7ff) ||
    (code >= 0xe000 && code <= 0xfffd) ||
    (code >= 0x10000 && code <= 0x10ffff)
  );
}

function isSpaceOnly(text) {
  for (const character of text) {
    const code = character.codePointAt(0);
    if (code !== 0x20 && code !== 0x9 && code !== 0xd && code !== 0xa) return false;
  }
  return text.length > 0;
}

/**
 * Check the XML declaration ourselves, from the leading text.
 *
 * Returns a refusal envelope, or null when the declaration is acceptable (or
 * absent). Only `version` and `encoding` carry policy; `standalone` does not.
 */
export function checkDeclaration(text) {
  const withoutBom = text.startsWith("﻿") ? text.slice(1) : text;
  if (!withoutBom.startsWith("<?xml")) return null;
  const end = withoutBom.indexOf("?>");
  if (end < 0) return refuse("parse", "the XML declaration is not closed");
  const declaration = withoutBom.slice(5, end);

  const version = /version\s*=\s*("([^"]*)"|'([^']*)')/.exec(declaration);
  const versionText = version ? version[2] ?? version[3] : null;
  if (versionText === null) return refuse("parse", "the XML declaration has no version");
  if (versionText !== "1.0") {
    return refuse("unsupported", `XML version ${versionText} is not supported`);
  }

  const encoding = /encoding\s*=\s*("([^"]*)"|'([^']*)')/.exec(declaration);
  const encodingText = encoding ? encoding[2] ?? encoding[3] : null;
  if (encodingText !== null && encodingText.toLowerCase() !== "utf-8") {
    return refuse("unsupported", `encoding ${encodingText} is not supported`);
  }
  return null;
}

/**
 * Read `text` and return the §3 envelope.
 *
 * The tree is built from saxes events into the same shape the probe and the
 * Rust oracle build, and the content projection is applied on the way out.
 */
export function read(text) {
  const declaration = checkDeclaration(text);
  if (declaration) return declaration;

  // A stack of child arrays; the root holds the document's own children.
  const documentChildren = [];
  const stack = [{ name: null, attributes: [], children: documentChildren }];
  let failure = null;

  const parser = new SaxesParser({ xmlns: true, position: true, fileName: "case" });

  parser.on("error", (error) => {
    if (!failure) failure = refuse("parse", error.message);
  });
  parser.on("doctype", () => {
    if (!failure) failure = refuse("unsupported", "a DOCTYPE declaration is not supported");
  });
  parser.on("opentag", (node) => {
    if (failure) return;
    const attributes = [];
    for (const [name, value] of Object.entries(node.attributes)) {
      attributes.push([name, typeof value === "string" ? value : value.value]);
    }
    stack.push({ name: node.name, attributes, children: [] });
  });
  parser.on("closetag", () => {
    if (failure) return;
    const finished = stack.pop();
    stack[stack.length - 1].children.push({
      kind: "e",
      name: finished.name,
      attributes: finished.attributes,
      children: finished.children,
    });
  });
  parser.on("text", (text) => {
    if (failure) return;
    stack[stack.length - 1].children.push({ kind: "t", text });
  });
  parser.on("cdata", (text) => {
    if (failure) return;
    stack[stack.length - 1].children.push({ kind: "t", text });
  });
  parser.on("comment", () => {});
  parser.on("processinginstruction", () => {});

  try {
    parser.write(text).close();
  } catch (error) {
    if (!failure) failure = refuse("parse", error.message);
  }
  if (failure) return failure;

  // saxes accepts characters XML 1.0 forbids in some positions, so check the
  // whole input ourselves rather than trusting the parser's view of §2.2.
  for (const character of text) {
    if (!isChar(character.codePointAt(0))) {
      return refuse("parse", `U+${character.codePointAt(0).toString(16)} is not an XML 1.0 character`);
    }
  }

  const elements = documentChildren.filter((child) => child.kind === "e");
  if (elements.length !== 1) {
    return refuse("parse", `a document must hold exactly one root element, not ${elements.length}`);
  }
  return { ok: true, content: ["doc", project(documentChildren)] };
}

/**
 * plan-138-A §4's content projection: drop comments and PIs (never collected),
 * merge adjacent text, drop layout whitespace where the parent also has an
 * element child, and sort attributes.
 */
export function project(children) {
  const hasElement = children.some((child) => child.kind === "e");
  const out = [];
  let pending = "";
  const flush = () => {
    if (pending === "") return;
    if (!(hasElement && isSpaceOnly(pending))) out.push(["t", pending]);
    pending = "";
  };
  for (const child of children) {
    if (child.kind === "t") {
      pending += child.text;
      continue;
    }
    flush();
    const attributes = child.attributes
      .slice()
      .sort((left, right) => (left[0] < right[0] ? -1 : left[0] > right[0] ? 1 : 0));
    out.push(["e", child.name, attributes, project(child.children)]);
  }
  flush();
  return out;
}

/** Answer a whole job file, in the same shape the other two sides answer. */
export function readJob(job) {
  return {
    results: job.cases.map((entry) => ({ id: entry.id, ...read(entry.xml) })),
  };
}

// ---------------------------------------------------------------------------
// Writing.
//
// xmldom appears here and ONLY here: its parser is lenient, which is why saxes
// does the reading, but its DOM and XMLSerializer are an independent writer —
// which is exactly what the write direction needs.
// ---------------------------------------------------------------------------

/** Build an xmldom Document from the §3 envelope shape. */
function buildDocument(documentTree) {
  const [, children] = documentTree;
  const doc = new DOMImplementation().createDocument(null, null, null);
  for (const child of children) appendNode(doc, doc, child);
  return doc;
}

function appendNode(doc, parent, node) {
  const [kind] = node;
  if (kind === "t") {
    parent.appendChild(doc.createTextNode(node[1]));
    return;
  }
  if (kind === "c") {
    parent.appendChild(doc.createComment(node[1]));
    return;
  }
  if (kind === "p") {
    parent.appendChild(doc.createProcessingInstruction(node[1], node[2]));
    return;
  }
  const [, name, attributes, kids] = node;
  const element = doc.createElement(name);
  for (const [key, value] of attributes) element.setAttribute(key, value);
  for (const kid of kids) appendNode(doc, element, kid);
  parent.appendChild(element);
}

/**
 * Write one tree, honouring `indent` the way the package does: an element's
 * children go on their own lines only when it holds no data text AND has an
 * element child — indenting anywhere else would change the content.
 */
export function write(documentTree, indent) {
  try {
    const doc = buildDocument(documentTree);
    const serialized = new XMLSerializer().serializeToString(doc);
    const body = indent ? reindent(documentTree, indent) : serialized;
    // xmldom writes a literal carriage return, which XML 1.0 §2.11 normalizes
    // to a line feed on the way back in — so the text would come back changed.
    // Only a character reference survives. In xmldom's output a CR can appear
    // only inside text or an attribute value, never in structural markup, so
    // rewriting every one is exactly right.
    const safe = body.replaceAll("\r", "&#13;");
    return { ok: true, xml: `<?xml version="1.0" encoding="UTF-8"?>${indent ? "\n" : ""}${safe}` };
  } catch (error) {
    return refuse("parse", error.message);
  }
}

/** The indented form, built from the tree rather than by re-parsing output. */
function reindent(documentTree, indent) {
  const [, children] = documentTree;
  const doc = new DOMImplementation().createDocument(null, null, null);
  const serializer = new XMLSerializer();
  const one = (node, level) => {
    const [kind] = node;
    if (kind !== "e") {
      const holder = doc.createDocumentFragment();
      appendNode(doc, holder, node);
      return serializer.serializeToString(holder);
    }
    const [, name, attributes, kids] = node;
    const hasText = kids.some(([childKind]) => childKind === "t");
    const hasElement = kids.some(([childKind]) => childKind === "e");
    const element = doc.createElement(name);
    for (const [key, value] of attributes) element.setAttribute(key, value);
    if (kids.length === 0) return serializer.serializeToString(element);
    if (hasText || !hasElement) {
      for (const kid of kids) appendNode(doc, element, kid);
      return serializer.serializeToString(element);
    }
    const open = serializer.serializeToString(element).replace(/\/>$/, ">");
    const inner = kids
      .map((kid) => "\n" + indent.repeat(level + 1) + one(kid, level + 1))
      .join("");
    return `${open}${inner}\n${indent.repeat(level)}</${name}>`;
  };
  return children.map((child) => one(child, 0)).join("\n");
}

// ---------------------------------------------------------------------------
// XPath.
//
// The DOM the query runs against is built from SAXES events, never by xmldom's
// own parser: the read direction refuses what the package refuses, and a query
// oracle that silently accepted a malformed document would disagree for the
// wrong reason. Comments and processing instructions are kept here (unlike the
// content projection, which drops them) because XPath can select them.
// ---------------------------------------------------------------------------

/** A full xmldom Document built from saxes events, or a refusal. */
function domOf(text) {
  const declaration = checkDeclaration(text);
  if (declaration) return { failure: declaration };

  const doc = new DOMImplementation().createDocument(null, null, null);
  const stack = [doc];
  let failure = null;

  const parser = new SaxesParser({ xmlns: true, position: true, fileName: "case" });
  parser.on("error", (error) => {
    if (!failure) failure = refuse("parse", error.message);
  });
  parser.on("doctype", () => {
    if (!failure) failure = refuse("unsupported", "a DOCTYPE declaration is not supported");
  });
  parser.on("opentag", (node) => {
    if (failure) return;
    const element = doc.createElement(node.name);
    for (const [name, value] of Object.entries(node.attributes)) {
      element.setAttribute(name, typeof value === "string" ? value : value.value);
    }
    stack[stack.length - 1].appendChild(element);
    stack.push(element);
  });
  parser.on("closetag", () => {
    if (failure) return;
    stack.pop();
  });
  parser.on("text", (chunk) => {
    if (failure) return;
    // XPath 1.0 §5.1: the root node's children are the document element plus
    // the prolog's and epilog's comments and processing instructions -- there
    // are NO text nodes outside the document element, so the whitespace around
    // it is not part of the data model.
    if (stack.length === 1) return;
    stack[stack.length - 1].appendChild(doc.createTextNode(chunk));
  });
  parser.on("cdata", (chunk) => {
    if (failure) return;
    if (stack.length === 1) return;
    stack[stack.length - 1].appendChild(doc.createTextNode(chunk));
  });
  parser.on("comment", (chunk) => {
    if (failure) return;
    stack[stack.length - 1].appendChild(doc.createComment(chunk));
  });
  parser.on("processinginstruction", (node) => {
    if (failure) return;
    stack[stack.length - 1].appendChild(
      doc.createProcessingInstruction(node.target, node.body ?? ""),
    );
  });

  try {
    parser.write(text).close();
  } catch (error) {
    if (!failure) failure = refuse("parse", error.message);
  }
  if (failure) return { failure };
  return { doc };
}

/** XPath's own string() of a number (§4.2): "2", not "2.0". */
export function xpathNumberToString(value) {
  if (Number.isNaN(value)) return "NaN";
  if (value === Infinity) return "Infinity";
  if (value === -Infinity) return "-Infinity";
  if (Number.isInteger(value)) return String(value);
  return String(value);
}

/** One element of a result node-set, in the content-envelope shape. */
function nodeEnvelope(node) {
  // 9 = DOCUMENT_NODE. XPath's root node has no form of its own in this
  // envelope, and selecting it (`.` at the top, or `/`) means the document --
  // so it is reported as its document element, which is what the package does.
  if (node.nodeType === 9) {
    const element = Array.from(node.childNodes ?? []).find((child) => child.nodeType === 1);
    return element ? nodeEnvelope(element) : ["t", ""];
  }
  // 1 = ELEMENT_NODE
  if (node.nodeType !== 1) {
    return ["t", node.nodeValue ?? node.textContent ?? ""];
  }
  const attributes = [];
  for (const attribute of Array.from(node.attributes ?? [])) {
    attributes.push([attribute.name, attribute.value]);
  }
  attributes.sort((left, right) => (left[0] < right[0] ? -1 : left[0] > right[0] ? 1 : 0));
  const children = Array.from(node.childNodes ?? []).map((child) => {
    if (child.nodeType === 1) return { kind: "e", element: child };
    if (child.nodeType === 3 || child.nodeType === 4) return { kind: "t", text: child.nodeValue };
    return { kind: "x" };
  });
  return ["e", node.nodeName, attributes, projectDomChildren(children)];
}

/** plan-138-A §4's projection over DOM children. */
function projectDomChildren(children) {
  const hasElement = children.some((child) => child.kind === "e");
  const out = [];
  let pending = "";
  const flush = () => {
    if (pending === "") return;
    if (!(hasElement && isSpaceOnly(pending))) out.push(["t", pending]);
    pending = "";
  };
  for (const child of children) {
    if (child.kind === "t") {
      pending += child.text;
      continue;
    }
    if (child.kind !== "e") continue;
    flush();
    out.push(nodeEnvelope(child.element));
  }
  flush();
  return out;
}

/**
 * XPath 1.0 counts *characters*; JavaScript's `String.length`, indexing and
 * iteration all count UTF-16 code units, so every character above the BMP
 * counts twice. `string-length`, `substring` and `translate` are the three
 * §4.2 functions defined in terms of characters, and npm `xpath` implements
 * all three on raw JavaScript strings -- so the Node oracle supplies its own.
 * Everything else falls through to the library's implementation.
 */
const characters = (text) => Array.from(text);

/** The context node's string-value, for the one-argument forms. */
function contextString(context) {
  return xpathLib.XNodeSet.prototype.stringForNode(context.contextNode);
}

/** `String.prototype.substring`'s clamping (NaN and negatives to 0, swap if crossed), over code points. */
function substringByCharacter(cps, from, to) {
  const clamp = (value) => (value > 0 ? Math.min(Math.floor(value), cps.length) : 0);
  let a = clamp(from);
  let b = to === undefined ? cps.length : clamp(to);
  if (a > b) [a, b] = [b, a];
  return cps.slice(a, b).join("");
}

const XPATH_FUNCTIONS = {
  "string-length": (context, text) =>
    characters(text === undefined ? contextString(context) : text.stringValue()).length,

  substring: (context, text, start, length) => {
    const cps = characters(text.stringValue());
    const from = Math.round(start.numberValue()) - 1;
    const to = length === undefined ? undefined : from + Math.round(length.numberValue());
    return substringByCharacter(cps, from, to);
  },

  translate: (context, text, from, to) => {
    const target = characters(to.stringValue());
    const map = new Map();
    characters(from.stringValue()).forEach((character, index) => {
      if (!map.has(character)) map.set(character, index < target.length ? target[index] : "");
    });
    return characters(text.stringValue())
      .map((character) => (map.has(character) ? map.get(character) : character))
      .join("");
  },
};

/** npm `xpath`'s resolver protocol: return nothing and the library's own function is used. */
function xpathFunction(name, namespace) {
  if (namespace) return undefined;
  return XPATH_FUNCTIONS[name];
}

/** Evaluate `expr` against `text` and return the §3 XPath envelope. */
export function xpath(text, expr) {
  const built = domOf(text);
  if (built.failure) return built.failure;

  let value;
  try {
    value = xpathLib.parse(expr).evaluate({ node: built.doc, functions: xpathFunction });
  } catch (error) {
    return refuse("unsupported", error.message);
  }

  if (value instanceof xpathLib.XBoolean) {
    return { ok: true, kind: "boolean", value: value.booleanValue() };
  }
  if (value instanceof xpathLib.XNumber) {
    return { ok: true, kind: "number", value: xpathNumberToString(value.numberValue()) };
  }
  if (value instanceof xpathLib.XString) {
    return { ok: true, kind: "string", value: value.stringValue() };
  }

  const nodes = value.nodeset().toArray();
  // 2 = ATTRIBUTE_NODE
  const attributes = nodes.filter((node) => node.nodeType === 2);
  if (attributes.length > 0) {
    if (attributes.length !== nodes.length) {
      return refuse("unsupported", "a node-set mixing attributes with other nodes");
    }
    return {
      ok: true,
      kind: "attributes",
      value: nodes.map((node) => [node.name, node.value]),
    };
  }
  return { ok: true, kind: "nodes", value: nodes.map(nodeEnvelope) };
}

/** Answer a whole `xpath` job. */
export function xpathJob(job) {
  return {
    results: job.cases.map((entry) => ({ id: entry.id, ...xpath(entry.xml, entry.expr) })),
  };
}

/** Answer a whole `write` job. */
export function writeJob(job) {
  return {
    results: job.cases.map((entry) => ({
      id: entry.id,
      ...write(entry.tree, entry.indent ?? ""),
    })),
  };
}
