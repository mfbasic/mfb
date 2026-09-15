#!/usr/bin/env node
// diff.mjs — the runner: ask all three sides the same questions and compare.
//
// The package, the Node oracle and the Rust oracle are EQUAL PEERS. A case
// passes only when all three agree. Where the two oracles disagree with each
// OTHER, the case fails as an oracle disagreement until the specification or
// the W3C suite settles it and the decision is recorded in divergences.json --
// never by majority vote, which would hide an oracle bug behind two votes.
//
//   node diff.mjs                 # every mode
//   node diff.mjs corpus          # one mode
//   node diff.mjs corpus --seed 7 --count 2000
//
// Exit status is 0 iff every case agreed, or diverged for a declared reason.

import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, writeFileSync, existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { readJob as nodeReadJob, writeJob as nodeWriteJob } from "./oracle.mjs";
import { trees, STYLES, rng, write as writeStyled } from "./generate.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "../../..");
const PROBE = join(HERE, "probe/build/xmlprobe.out");
const RUST = join(HERE, "rust/target/release/xmloracle");
const CORPUS = join(HERE, "corpus");
const DIVERGENCES = join(HERE, "divergences.json");

const WORK = mkdtempSync(join(tmpdir(), "xmloracle-"));

/** Run one side over a whole job file and return its `results` array. */
function runSide(command, operation, job, label) {
  const path = join(WORK, `${label}.json`);
  writeFileSync(path, JSON.stringify(job));
  let output;
  try {
    output = execFileSync(command, [operation, path], {
      encoding: "utf8",
      maxBuffer: 1 << 28,
    });
  } catch (error) {
    throw new Error(`${label} exited non-zero: ${error.stderr || error.message}`);
  }
  let parsed;
  try {
    parsed = JSON.parse(output);
  } catch {
    throw new Error(`${label} wrote something that is not JSON: ${output.slice(0, 200)}`);
  }
  if (!Array.isArray(parsed.results)) {
    throw new Error(`${label} wrote no results array`);
  }
  return parsed.results;
}

const asked = {
  probe: (operation, job) => runSide(PROBE, operation, job, "probe"),
  rust: (operation, job) => runSide(RUST, operation, job, "rust"),
  node: (operation, job) => nodeReadJob(job).results,
};

/**
 * Compare one case across the three sides.
 *
 * Two refusals agree without comparing kind or reason: three implementations
 * have three vocabularies for "no", and demanding the same words would make the
 * harness fail on wording rather than on meaning. What is compared is
 * accept-vs-refuse, and content.
 */
export function compare(id, results) {
  const [probe, node, rust] = results;
  const accepted = (result) => Boolean(result && result.ok);
  const shape = [accepted(probe), accepted(node), accepted(rust)];

  if (!shape[0] && !shape[1] && !shape[2]) return null; // all refused: agreement
  if (shape[0] !== shape[1] || shape[1] !== shape[2]) {
    const say = (name, result) =>
      `${name}=${accepted(result) ? "accepted" : `refused(${result?.kind}: ${result?.reason})`}`;
    return `${id}: accept/refuse disagreement — ${say("package", probe)}, ${say("node", node)}, ${say("rust", rust)}`;
  }

  const keys = [probe, node, rust].map((result) => JSON.stringify(result.content));
  if (keys[0] === keys[1] && keys[1] === keys[2]) return null;
  const which =
    keys[1] === keys[2]
      ? "the package disagrees with both oracles"
      : keys[0] === keys[1]
        ? "the Rust oracle disagrees"
        : keys[0] === keys[2]
          ? "the Node oracle disagrees"
          : "all three disagree";
  return [
    `${id}: content disagreement — ${which}`,
    `  package: ${keys[0]}`,
    `  node:    ${keys[1]}`,
    `  rust:    ${keys[2]}`,
  ].join("\n");
}

function loadDivergences() {
  if (!existsSync(DIVERGENCES)) return {};
  return JSON.parse(readFileSync(DIVERGENCES, "utf8")).cases ?? {};
}

/** corpus mode: every document in corpus/, compared three ways. */
function runCorpus() {
  const declared = loadDivergences();
  const names = readdirSync(CORPUS)
    .filter((name) => name.endsWith(".xml"))
    .sort();
  if (names.length === 0) throw new Error("corpus/ holds no .xml files");

  const job = {
    cases: names.map((name) => ({
      id: name,
      xml: readFileSync(join(CORPUS, name), "utf8"),
    })),
  };

  const probe = asked.probe("read", job);
  const rust = asked.rust("read", job);
  const node = asked.node("read", job);

  const failures = [];
  let diverged = 0;
  for (let index = 0; index < job.cases.length; index += 1) {
    const id = job.cases[index].id;
    const problem = compare(id, [probe[index], node[index], rust[index]]);
    if (!problem) {
      if (declared[id]) {
        failures.push(`${id}: declared divergent in divergences.json, but all three now agree — remove the entry`);
      }
      continue;
    }
    if (declared[id]) {
      diverged += 1;
      continue;
    }
    failures.push(problem);
  }
  return { count: job.cases.length, failures, diverged };
}

// ---------------------------------------------------------------------------
// xmlconf: the W3C XML Conformance Test Suite, under this package's policy.
// ---------------------------------------------------------------------------

const XMLCONF = join(HERE, "xmlconf/xmlconf");

/** One attribute of a TEST tag, in either quote style. */
function attribute(tag, name) {
  const double = new RegExp(`\\b${name}\\s*=\\s*"([^"]*)"`).exec(tag);
  if (double) return double[1];
  const single = new RegExp(`\\b${name}\\s*=\\s*'([^']*)'`).exec(tag);
  return single ? single[1] : null;
}

/** Every catalog file that actually holds TEST entries, walked recursively. */
function catalogFiles(directory) {
  const out = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      out.push(...catalogFiles(path));
      continue;
    }
    if (!entry.name.endsWith(".xml")) continue;
    const text = readFileSync(path, "latin1");
    if (text.includes("<TEST ")) out.push(path);
  }
  return out;
}

/**
 * Read the suite's catalogs into test descriptors.
 *
 * Each URI resolves against its own catalog's directory, which is why xml:base
 * never needs handling: it appears only in eduni/xmlconf.xml, a wrapper that
 * includes sub-catalogs rather than holding tests itself.
 */
export function loadSuite() {
  const tests = [];
  for (const catalog of catalogFiles(XMLCONF)) {
    const text = readFileSync(catalog, "utf8");
    const base = dirname(catalog);
    for (const match of text.matchAll(/<TEST\b[\s\S]*?>/g)) {
      const tag = match[0];
      const uri = attribute(tag, "URI");
      if (!uri) continue;
      tests.push({
        id: attribute(tag, "ID") ?? uri,
        type: attribute(tag, "TYPE"),
        namespace: attribute(tag, "NAMESPACE"),
        recommendation: attribute(tag, "RECOMMENDATION"),
        edition: attribute(tag, "EDITION"),
        version: attribute(tag, "VERSION"),
        path: resolve(base, uri),
      });
    }
  }
  return tests;
}

/**
 * What this package's policy says about one test, before anyone parses it.
 *
 * Returns "accept", "refuse", or a skip reason. The rules are §4's:
 *   - XML 1.1 / Namespaces 1.1 tests: refused, by the version policy;
 *   - any file holding a DOCTYPE: refused, by the no-DTD policy, whatever the
 *     suite says the file is;
 *   - not-wf: refused;
 *   - valid/invalid without a DOCTYPE: accepted, with equal content;
 *   - error: reported only, never asserted.
 */
export function expectationFor(test, text) {
  if (test.namespace === "no") return { skip: "NAMESPACE=no" };
  if (test.type === "error") return { skip: "TYPE=error" };

  // A test that names the editions it applies to, and does not name the fifth,
  // is testing a rule this reader does not implement. XML 1.0 Fifth Edition
  // adopted XML 1.1's name characters, so `rmt-016` (a Byzantine Musical Symbol
  // in a name, "illegal in XML 1.0") is legal HERE: U+1D032 falls in
  // [#x10000-#xEFFFF]. Refusing it would mean implementing the 4th edition.
  if (test.edition && !test.edition.split(/\s+/).includes("5")) {
    return { skip: "EDITION excludes 5" };
  }
  // Namespaces 1.1 adds prefix undeclaration; this reader implements 1.0.
  if (test.recommendation === "NS1.1") return { skip: "NS1.1" };

  // The version policy keys off what the DOCUMENT declares, not off which
  // recommendation the test was written for: an XML1.1 test whose file declares
  // version="1.0" is read as 1.0 and must be treated as such.
  if (/<\?xml[^>]*\bversion\s*=\s*["']1\.1["']/.test(text)) return { expect: "refuse" };

  if (hasDoctype(text)) return { expect: "refuse" };
  if (test.type === "not-wf") return { expect: "refuse" };
  return { expect: "accept" };
}

/**
 * Does this document actually have a DOCTYPE declaration?
 *
 * A raw substring scan is wrong, and the suite proves it: `o-p15pass1`,
 * `o-p16pass1` and `o-p18pass1` hold the text `<!DOCTYPE` inside a comment, a
 * processing instruction and a CDATA section respectively, and are perfectly
 * well-formed DTD-less documents. Those constructs are removed before looking.
 */
export function hasDoctype(text) {
  const stripped = text
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/<!\[CDATA\[[\s\S]*?\]\]>/g, "")
    .replace(/<\?[\s\S]*?\?>/g, "");
  return stripped.includes("<!DOCTYPE");
}

function runXmlconf() {
  if (!existsSync(XMLCONF)) {
    throw new Error(`no suite at ${XMLCONF} — run packages/xml/oracle/fetch-xmlconf.sh first`);
  }
  const declared = loadDivergences();
  const suite = loadSuite();

  const skipped = { "NAMESPACE=no": 0, "TYPE=error": 0, "not UTF-8": 0 };
  const cases = [];
  const expectations = new Map();
  for (const test of suite) {
    const raw = readFileSync(test.path);
    // A UTF-16 or otherwise non-UTF-8 file cannot cross the JSON job boundary,
    // and MFBASIC's String cannot hold one. Skipped with a count, never
    // silently counted as agreement.
    if (
      (raw[0] === 0xff && raw[1] === 0xfe) ||
      (raw[0] === 0xfe && raw[1] === 0xff) ||
      !isValidUtf8(raw)
    ) {
      skipped["not UTF-8"] += 1;
      continue;
    }
    const text = raw.toString("utf8");
    const decision = expectationFor(test, text);
    if (decision.skip) {
      skipped[decision.skip] += 1;
      continue;
    }
    expectations.set(test.id, decision.expect);
    cases.push({ id: test.id, xml: text });
  }

  const job = { cases };
  const probe = asked.probe("read", job);
  const rust = asked.rust("read", job);
  const node = asked.node("read", job);

  const failures = [];
  let diverged = 0;
  let accepted = 0;
  for (let index = 0; index < cases.length; index += 1) {
    const id = cases[index].id;
    const results = [probe[index], node[index], rust[index]];
    const problem = compare(id, results);
    const wanted = expectations.get(id);

    if (problem) {
      if (declared[id]) diverged += 1;
      else failures.push(problem);
      continue;
    }
    if (declared[id]) {
      failures.push(`${id}: declared divergent in divergences.json, but all three now agree — remove the entry`);
      continue;
    }
    // They agree with each other; do they agree with the policy?
    const got = results[0].ok ? "accept" : "refuse";
    if (got !== wanted) {
      failures.push(`${id}: all three ${got}ed, but this package's policy says ${wanted}`);
      continue;
    }
    if (got === "accept") accepted += 1;
  }
  const note =
    `${accepted} accepted, ${cases.length - accepted} refused by policy; ` +
    `skipped ${skipped["NAMESPACE=no"]} NAMESPACE=no, ${skipped["TYPE=error"]} TYPE=error, ` +
    `${skipped["not UTF-8"]} not UTF-8`;
  return { count: cases.length, failures, diverged, note };
}

/** Node's Buffer has no "is this valid UTF-8" predicate; round-tripping tells. */
function isValidUtf8(raw) {
  return Buffer.compare(Buffer.from(raw.toString("utf8"), "utf8"), raw) === 0;
}

// ---------------------------------------------------------------------------
// The fuzz modes: random trees, both directions.
// ---------------------------------------------------------------------------

/**
 * fuzz-read: write each random tree out in several STYLES and check that every
 * side reads back the content the tree started with.
 *
 * The expected content is computed from the tree by the generator's own
 * projection, so this does not merely check that three readers agree — it
 * checks they agree with what was written.
 */
function runFuzzRead(options) {
  const cases = [];
  const expected = new Map();
  for (const { id, tree } of trees(options.seed, options.count)) {
    for (const style of STYLES) {
      const caseId = `${id}/${style.name}`;
      cases.push({ id: caseId, xml: writeStyled(tree, style) });
      expected.set(caseId, JSON.stringify(projectTree(tree)));
    }
  }
  return compareAll("read", cases, expected, options);
}

/** fuzz-write: each side writes the tree, and both oracles read it back. */
function runFuzzWrite(options) {
  const cases = [];
  const expected = new Map();
  for (const { id, tree } of trees(options.seed, options.count)) {
    for (const indent of ["", "  ", "\t"]) {
      const caseId = `${id}/indent${indent.length}`;
      cases.push({ id: caseId, tree, indent });
      expected.set(caseId, JSON.stringify(projectTree(tree)));
    }
  }

  const job = { cases };
  const written = {
    probe: asked.probe("write", job),
    rust: asked.rust("write", job),
    node: nodeWriteJob(job).results,
  };

  // Everything each side wrote must read back with the tree's own content, on
  // both readers. That is what makes this a WRITE test rather than a second
  // read test: the three writers are compared through two independent readers.
  const readBack = { cases: [] };
  const origin = [];
  for (const side of ["probe", "rust", "node"]) {
    for (let index = 0; index < cases.length; index += 1) {
      const result = written[side][index];
      if (!result.ok) continue;
      readBack.cases.push({ id: `${side}:${cases[index].id}`, xml: result.xml });
      origin.push({ side, id: cases[index].id });
    }
  }

  const failures = [];
  for (const side of ["probe", "rust", "node"]) {
    for (let index = 0; index < cases.length; index += 1) {
      const result = written[side][index];
      if (!result.ok) {
        failures.push(`${cases[index].id}: ${side} refused to write it — ${result.kind}: ${result.reason}`);
      }
    }
  }

  const probe = asked.probe("read", readBack);
  const rust = asked.rust("read", readBack);
  const node = asked.node("read", readBack);
  for (let index = 0; index < readBack.cases.length; index += 1) {
    const id = readBack.cases[index].id;
    const problem = compare(id, [probe[index], node[index], rust[index]]);
    if (problem) {
      failures.push(problem);
      continue;
    }
    if (!probe[index].ok) {
      failures.push(`${id}: every side refused to read back what ${origin[index].side} wrote`);
      continue;
    }
    const want = expected.get(origin[index].id);
    const got = JSON.stringify(probe[index].content);
    if (got !== want) {
      failures.push([`${id}: ${origin[index].side}'s output lost content`, `  wrote:  ${got}`, `  tree:   ${want}`].join("\n"));
    }
  }
  return { count: readBack.cases.length, failures, diverged: 0 };
}

/** roundtrip: content(read(write(read(x)))) = content(read(x)), on the package. */
function runRoundtrip(options) {
  const documents = [];
  for (const name of readdirSync(CORPUS).filter((file) => file.endsWith(".xml")).sort()) {
    documents.push({ id: name, xml: readFileSync(join(CORPUS, name), "utf8") });
  }
  for (const { id, tree } of trees(options.seed, Math.max(20, options.count))) {
    documents.push({ id, xml: writeStyled(tree, STYLES[0]) });
  }

  const first = asked.probe("read", { cases: documents });
  const writable = [];
  const firstContent = new Map();
  for (let index = 0; index < documents.length; index += 1) {
    if (!first[index].ok) continue; // a refusal round-trips trivially
    firstContent.set(documents[index].id, JSON.stringify(first[index].content));
    for (const indent of ["", "  "]) {
      writable.push({ id: `${documents[index].id}/indent${indent.length}`, tree: first[index].content, indent });
    }
  }

  const rewritten = asked.probe("write", { cases: writable });
  const again = { cases: [] };
  for (let index = 0; index < writable.length; index += 1) {
    if (!rewritten[index].ok) continue;
    again.cases.push({ id: writable[index].id, xml: rewritten[index].xml });
  }
  const reread = asked.probe("read", again);

  const failures = [];
  for (let index = 0; index < writable.length; index += 1) {
    if (!rewritten[index].ok) {
      failures.push(`${writable[index].id}: the package refused to write back what it read — ${rewritten[index].reason}`);
    }
  }
  for (let index = 0; index < again.cases.length; index += 1) {
    const id = again.cases[index].id;
    const source = id.slice(0, id.lastIndexOf("/"));
    if (!reread[index].ok) {
      failures.push(`${id}: the package could not read back its own output — ${reread[index].reason}`);
      continue;
    }
    const got = JSON.stringify(reread[index].content);
    const want = firstContent.get(source);
    if (got !== want) {
      failures.push([`${id}: content changed across a round trip`, `  after: ${got}`, `  before: ${want}`].join("\n"));
    }
  }
  return { count: again.cases.length, failures, diverged: 0 };
}

/** Ask all three sides one job of read cases and compare, with expectations. */
function compareAll(operation, cases, expected, options) {
  const job = { cases };
  const probe = asked.probe(operation, job);
  const rust = asked.rust(operation, job);
  const node = asked.node(operation, job);

  const failures = [];
  for (let index = 0; index < cases.length; index += 1) {
    const id = cases[index].id;
    const problem = compare(id, [probe[index], node[index], rust[index]]);
    if (problem) {
      failures.push(`${problem}\n     replay: --seed ${options.seed} --count ${options.count}`);
      continue;
    }
    if (!probe[index].ok) {
      failures.push(`${id}: every side refused a document the generator produced — ${probe[index].reason}`);
      continue;
    }
    const want = expected.get(id);
    if (want !== undefined && JSON.stringify(probe[index].content) !== want) {
      failures.push(
        [
          `${id}: all three agree, but not with the tree that was written`,
          `  read:    ${JSON.stringify(probe[index].content)}`,
          `  written: ${want}`,
          `  replay: --seed ${options.seed} --count ${options.count}`,
        ].join("\n"),
      );
    }
  }
  return { count: cases.length, failures, diverged: 0 };
}

/** The generator's own content projection, for comparing against the tree. */
function projectTree(tree) {
  const [, children] = tree;
  return ["doc", projectChildren(children)];
}

function projectChildren(children) {
  const hasElement = children.some(([kind]) => kind === "e");
  const out = [];
  let pending = "";
  const flush = () => {
    if (pending === "") return;
    const layout = hasElement && /^[ \t\r\n]+$/.test(pending);
    if (!layout) out.push(["t", pending]);
    pending = "";
  };
  for (const child of children) {
    const [kind] = child;
    if (kind === "t") {
      pending += child[1];
      continue;
    }
    if (kind !== "e") continue;
    flush();
    const attributes = child[2]
      .slice()
      .sort((left, right) => (left[0] < right[0] ? -1 : left[0] > right[0] ? 1 : 0));
    out.push(["e", child[1], attributes, projectChildren(child[3])]);
  }
  flush();
  return out;
}

// ---------------------------------------------------------------------------
// mutate: is the probe ROBUST? Not "is it right" — damaged input has no right
// answer. What is asserted is that it always answers a well-formed envelope,
// exits 0, and finishes; agreement is reported as information only.
// ---------------------------------------------------------------------------

const DAMAGE = ["<", "&", "]]>", "\r", "\0", ">", "<![CDATA[", "&#", "</", "�"];

function damage(raw, next) {
  const bytes = Buffer.from(raw);
  if (bytes.length === 0) return bytes;
  const at = Math.floor(next() * bytes.length);
  const choice = next();
  if (choice < 0.3) {
    // Flip a bit — this is the one that can produce invalid UTF-8.
    const copy = Buffer.from(bytes);
    copy[at] ^= 1 << Math.floor(next() * 8);
    return copy;
  }
  if (choice < 0.5) {
    return Buffer.concat([bytes.subarray(0, at), bytes.subarray(at + 1)]);
  }
  if (choice < 0.7) {
    const insert = Buffer.from(DAMAGE[Math.floor(next() * DAMAGE.length)]);
    return Buffer.concat([bytes.subarray(0, at), insert, bytes.subarray(at)]);
  }
  if (choice < 0.85) {
    // Truncate: an unclosed everything.
    return bytes.subarray(0, at);
  }
  // A raw byte that cannot start a UTF-8 sequence.
  const copy = Buffer.from(bytes);
  copy[at] = 0xff;
  return copy;
}

function runMutate(options) {
  const next = rng(options.seed);
  const sources = readdirSync(CORPUS)
    .filter((name) => name.endsWith(".xml"))
    .sort()
    .map((name) => ({ name, raw: readFileSync(join(CORPUS, name)) }));

  const cases = [];
  let notUtf8 = 0;
  for (let index = 0; index < options.count; index += 1) {
    const source = sources[Math.floor(next() * sources.length)];
    const mutated = damage(source.raw, next);
    // Invalid UTF-8 cannot cross the JSON job boundary, and MFBASIC's String
    // cannot hold it — the same wall xmlconf hit. Counted, never silently
    // dropped.
    if (Buffer.compare(Buffer.from(mutated.toString("utf8"), "utf8"), mutated) !== 0) {
      notUtf8 += 1;
      continue;
    }
    cases.push({ id: `${source.name}#${index}`, xml: mutated.toString("utf8") });
  }

  const started = process.hrtime.bigint();
  const probe = asked.probe("read", { cases });
  const seconds = Number(process.hrtime.bigint() - started) / 1e9;

  const failures = [];
  if (probe.length !== cases.length) {
    failures.push(`the probe answered ${probe.length} of ${cases.length} cases`);
  }
  for (let index = 0; index < probe.length; index += 1) {
    const result = probe[index];
    if (!result || typeof result.ok !== "boolean") {
      failures.push(`${cases[index].id}: the probe's answer is not a well-formed envelope`);
      continue;
    }
    if (result.ok && !Array.isArray(result.content)) {
      failures.push(`${cases[index].id}: the probe said ok but wrote no content`);
    }
    if (!result.ok && typeof result.reason !== "string") {
      failures.push(`${cases[index].id}: the probe refused without a reason`);
    }
  }
  if (seconds > 30) {
    failures.push(`the probe took ${seconds.toFixed(1)} s over ${cases.length} damaged documents (limit 30 s)`);
  }

  // Agreement is information here, not a verdict: damaged input may legitimately
  // be read differently by different implementations.
  const rust = asked.rust("read", { cases });
  const node = asked.node("read", { cases });
  let agreed = 0;
  for (let index = 0; index < cases.length; index += 1) {
    if (!compare(cases[index].id, [probe[index], node[index], rust[index]])) agreed += 1;
  }
  const accepted = probe.filter((result) => result && result.ok).length;
  const note =
    `${accepted} still parsed, ${cases.length - accepted} refused; ` +
    `${agreed}/${cases.length} agreed three ways (information only); ` +
    `${notUtf8} skipped as not UTF-8; probe took ${seconds.toFixed(1)} s`;
  return { count: cases.length, failures, diverged: 0, note };
}

// ---------------------------------------------------------------------------
// perf: the 100k-node shapes, through the package, under the plan's budget.
// ---------------------------------------------------------------------------

function shape(kind, nodes) {
  const parts = ["<root>"];
  if (kind === "flat") {
    const items = Math.floor((nodes - 1) / 2);
    for (let index = 0; index < items; index += 1) {
      parts.push(`<item id="${index}">value ${index}</item>`);
    }
  } else if (kind === "deep") {
    let built = 1;
    while (built < nodes) {
      const levels = Math.min(255, nodes - built);
      parts.push("<d>".repeat(levels), "</d>".repeat(levels));
      built += levels;
    }
  } else {
    for (let index = 1; index < nodes; index += 1) parts.push("<i/>");
  }
  parts.push("</root>");
  return parts.join("");
}

function runPerf() {
  const failures = [];
  const notes = [];
  for (const kind of ["flat", "deep", "wide"]) {
    const xml = shape(kind, 100000);
    const started = process.hrtime.bigint();
    const results = asked.probe("read", { cases: [{ id: kind, xml }] });
    const seconds = Number(process.hrtime.bigint() - started) / 1e9;
    if (!results[0] || !results[0].ok) {
      failures.push(`${kind}: the package refused a 100k-node document — ${results[0]?.reason}`);
      continue;
    }
    notes.push(`${kind} ${(xml.length / 1024).toFixed(0)} KiB in ${seconds.toFixed(2)} s`);
    if (seconds > 3) {
      failures.push(`${kind}: ${seconds.toFixed(2)} s for 100,000 nodes (budget 3.00 s)`);
    }
  }
  return { count: 3, failures, diverged: 0, note: notes.join("; ") };
}

const MODES = {
  corpus: runCorpus,
  xmlconf: runXmlconf,
  "fuzz-read": runFuzzRead,
  "fuzz-write": runFuzzWrite,
  roundtrip: runRoundtrip,
  mutate: runMutate,
  perf: runPerf,
};

/** `--seed 7 --count 2000`, so a fuzz failure replays exactly. */
function parseOptions(argv) {
  const options = { seed: 1, count: 200 };
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] === "--seed") options.seed = Number(argv[index + 1]);
    if (argv[index] === "--count") options.count = Number(argv[index + 1]);
  }
  return options;
}

function main() {
  const argv = process.argv.slice(2);
  const options = parseOptions(argv);
  const flagValues = new Set();
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] === "--seed" || argv[index] === "--count") flagValues.add(index + 1);
  }
  const requested = argv.filter(
    (argument, index) => !argument.startsWith("--") && !flagValues.has(index),
  );
  const modes = requested.length > 0 ? requested : Object.keys(MODES);

  let failed = false;
  for (const mode of modes) {
    const run = MODES[mode];
    if (!run) {
      console.error(`unknown mode \`${mode}\` (have: ${Object.keys(MODES).join(", ")})`);
      process.exit(2);
    }
    const { count, failures, diverged, note } = run(options);
    const declared = diverged > 0 ? `, ${diverged} declared divergent` : "";
    // The mode's own counts — for xmlconf these include the skip counts the
    // acceptance check reads, so they print on success as well as on failure.
    const detail = note ? `\n     ${note}` : "";
    if (failures.length === 0) {
      console.log(`ok   ${mode}: ${count} case(s) agreed three ways${declared}${detail}`);
      continue;
    }
    failed = true;
    console.log(`FAIL ${mode}: ${failures.length} of ${count} case(s) disagreed${declared}${detail}`);
    for (const failure of failures) console.log(failure.replace(/^/gm, "     "));
  }
  process.exit(failed ? 1 : 0);
}

// Only when run as a program. Importing this module must not launch a run:
// the comparator itself is worth testing, and a module that executes on import
// cannot be.
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
