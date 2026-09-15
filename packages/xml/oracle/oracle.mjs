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
