#!/usr/bin/env node
//
// oracle.mjs - differential-test `packages/json_schema` against ajv.
//
// The package's own TESTING blocks (`mfb test packages/json_schema`) pin the
// behaviour this project decided on. This script does the other half: it checks
// that behaviour against INDEPENDENT implementations, so a shared misreading of
// the specification has somewhere to show up. It is not part of any gate - it
// needs Node and ajv installed - but it is the evidence behind the package's
// correctness claims.
//
//     npm install                      # in this directory
//     ./run.sh                         # build everything, then run every mode
//     node oracle.mjs corpus           # one mode
//     node oracle.mjs fuzz --seed 7    # reproducible random testing
//     node oracle.mjs suite --suite <checkout>
//
// Modes:
//
//   corpus   hand-written schemas covering the whole supported subset,
//            read by both implementations
//   pattern  every `pattern` in the corpus, plus a generated set, compared
//            against Node's own RegExp under the `u` flag
//   fuzz     random schemas built from the supported subset, with random
//            instances, compared verdict for verdict
//   suite    the official JSON-Schema-Test-Suite, which has GROUND TRUTH:
//            each case says whether the instance is valid, so this mode can
//            tell "we are wrong" from "we merely disagree"
//
// Exit status is 0 iff every disagreement is one of the KNOWN ones below.
//
// ajv is configured with `strict: false` because ajv's strict mode is ajv's own
// policy, not the specification's: it refuses schemas the specification allows
// (an unknown keyword, `if` without `then`). Everything else is left at its
// default, in particular `unicodeRegExp: true`, which compiles a `pattern` with
// the ECMA-262 `u` flag - the mode `ecma.mfb` translates against.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import AjvModule from "ajv/dist/2020.js";

import { CORPUS, DIVERGENT } from "./corpus.mjs";

const Ajv = AjvModule.default ?? AjvModule;
const HERE = path.dirname(fileURLToPath(import.meta.url));
const DRIVER = process.env.JSVALIDATE ?? defaultDriver();

// Inputs the two implementations are SUPPOSED to disagree about, each with the
// reason. Anything not in here that disagrees is a failure.
const KNOWN_DIVERGENCES = {
  "unsupported-dynamic":
    "$dynamicRef/$dynamicAnchor/$recursiveRef/$recursiveAnchor are outside the documented subset and are refused at compile time",
  "unsupported-regex":
    "a lookaround, a backreference, a Script_Extensions or binary property the engine does not implement, or a negated shorthand inside a character class cannot be translated into the regex dialect exactly, so it is refused rather than approximated",
  "regex-word-boundary":
    "\\b and \\B test word-ness with Unicode general categories here and with [A-Za-z0-9_] in ECMA-262; a boundary beside a non-ASCII letter is where they differ",
  "no-retrieval":
    "a $ref naming a resource outside the compiled document is refused; this validator retrieves nothing",
  "custom-dialect":
    "a `$schema` naming a metaschema other than Draft 2020-12 selects a different vocabulary set; this validator implements one dialect and refuses the others rather than guessing which keywords still apply",
  "schema-shape":
    "a schema keyword holding the wrong JSON type is refused at compile time rather than ignored",
  "ajv-enum-nonempty":
    "ajv refuses to compile {enum: []}, but the 2020-12 metaschema types enum as {type: array, items: true} with no minimum length, so it is a valid schema that accepts nothing",
  "ajv-multipleof-parseint":
    "ajv tests `multipleOf` with `division !== parseInt(division)`, and parseInt reads a quotient in exponent notation as its leading digits: parseInt(5e299) is 5, so ajv calls 1e300 not a multiple of 2 when exact arithmetic says it is",
  "ajv-prefixitems-false-contains":
    "ajv skips `contains` on an EMPTY array when a sibling `prefixItems` holds a `false` schema: it calls {prefixItems:[false],contains:false} valid for [], while {contains:false} alone it correctly calls invalid, and the official suite (contains.json) says an empty array is invalid",
  "ajv-contains-annotation":
    "ajv treats a passing `contains` as having evaluated EVERY item; the specification and the official suite annotate only the MATCHING ones (unevaluatedItems.json, 'unevaluatedItems depends on adjacent contains': [1,2,'foo'] is invalid and ajv calls it valid)",
};

function defaultDriver() {
  const built = path.join(HERE, "driver", "build");
  for (const name of ["jsvalidate.out", "jsvalidate.exe", "jsvalidate"]) {
    const candidate = path.join(built, name);
    if (fs.existsSync(candidate)) return candidate;
  }
  return path.join(built, "jsvalidate.out");
}

// ---------------------------------------------------------------------------
// Talking to the driver.
// ---------------------------------------------------------------------------

// Ask the driver a whole job at once. Starting a process per question would
// cost more than answering it, and a fuzzing run asks tens of thousands.
function runDriver(job) {
  if (!fs.existsSync(DRIVER)) {
    fail(
      `no driver at ${DRIVER}\n` +
        "build it first:  mfb build packages/json_schema/oracle/driver\n" +
        "or point $JSVALIDATE at one.",
    );
  }
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "json-schema-oracle-"));
  const jobPath = path.join(dir, "job.json");
  try {
    fs.writeFileSync(jobPath, JSON.stringify(job));
    const run = spawnSync(DRIVER, ["--batch", jobPath], {
      encoding: "utf8",
      maxBuffer: 1 << 28,
    });
    if (run.error) fail(`could not run ${DRIVER}: ${run.error.message}`);
    if (run.status !== 0) {
      fail(`${DRIVER} --batch exited ${run.status}\n${run.stderr || run.stdout}`);
    }
    return JSON.parse(run.stdout);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

function fail(message) {
  console.error(`oracle: ${message}`);
  process.exit(2);
}

// ---------------------------------------------------------------------------
// Asking ajv.
// ---------------------------------------------------------------------------

function newAjv() {
  return new Ajv({ strict: false, allErrors: true });
}

// ajv's verdict, in the driver's own vocabulary, so the two are comparable
// without either side knowing about the other.
function ajvVerdict(schema, instance) {
  let validate;
  try {
    validate = newAjv().compile(schema);
  } catch (error) {
    return { status: "compile-error", message: error.message };
  }
  try {
    return validate(instance)
      ? { status: "valid" }
      : { status: "invalid", message: describeAjvErrors(validate.errors) };
  } catch (error) {
    return { status: "validate-error", message: error.message };
  }
}

function describeAjvErrors(errors) {
  return (errors ?? [])
    .map((e) => `${e.instancePath || "<root>"} ${e.message} (${e.keyword})`)
    .join("; ");
}

// The two verdicts agree when they agree about VALIDITY. Which errors each
// reports, and how many, is each implementation's own business.
function verdictOf(result) {
  if (result.status === "valid" || result.status === "invalid") return result.status;
  return "error";
}

// ---------------------------------------------------------------------------
// Reporting.
// ---------------------------------------------------------------------------

class Report {
  constructor(mode) {
    this.mode = mode;
    this.checked = 0;
    this.agreed = 0;
    this.expected = new Map();
    this.failures = [];
  }

  agree() {
    this.checked += 1;
    this.agreed += 1;
  }

  known(key) {
    this.checked += 1;
    this.expected.set(key, (this.expected.get(key) ?? 0) + 1);
  }

  disagree(detail) {
    this.checked += 1;
    this.failures.push(detail);
  }

  print() {
    const width = 9;
    console.log(`\n== ${this.mode} ==`);
    console.log(`${String(this.checked).padStart(width)} compared`);
    console.log(`${String(this.agreed).padStart(width)} agreed`);
    for (const [key, count] of [...this.expected].sort()) {
      console.log(
        `${String(count).padStart(width)} expected divergence: ${key}`,
      );
      console.log(`${" ".repeat(width + 1)}${KNOWN_DIVERGENCES[key]}`);
    }
    if (this.failures.length === 0) return;
    console.log(`${String(this.failures.length).padStart(width)} DISAGREED`);
    const cap = Number(process.env.ORACLE_MAX_FAILURES ?? 40);
    for (const failure of this.failures.slice(0, cap)) {
      console.log("\n  " + failure.split("\n").join("\n  "));
    }
    if (this.failures.length > cap) {
      console.log(`\n  ... and ${this.failures.length - cap} more`);
    }
  }
}

// Failure detail is truncated so a fuzzing report stays readable; set
// ORACLE_FULL=1 to print the whole schema, which is what you need the moment a
// truncated one turns out to be the interesting part.
const SHOW_LIMIT = process.env.ORACLE_FULL ? Infinity : 240;

function show(value) {
  const text = JSON.stringify(value);
  if (text === undefined) return "undefined";
  return text.length > SHOW_LIMIT ? text.slice(0, SHOW_LIMIT - 3) + "..." : text;
}

// ---------------------------------------------------------------------------
// corpus
// ---------------------------------------------------------------------------

function expandCases(entries) {
  const cases = [];
  entries.forEach((entry, groupIndex) => {
    entry.instances.forEach((instance, instanceIndex) => {
      cases.push({
        id: `${groupIndex}.${instanceIndex}`,
        schema: entry.schema,
        instance,
        divergence: entry.key ?? entry.divergence ?? null,
      });
    });
  });
  return cases;
}

// Does any subschema in `schema` satisfy `predicate`? Generated cases carry no
// hand-written divergence tag, so a known ajv limitation has to be recognised
// from the SHAPE of the schema instead. Each rule below is narrow and names the
// limitation it stands for.
function anySubschema(schema, predicate) {
  if (schema === null || typeof schema !== "object") return false;
  if (Array.isArray(schema)) return schema.some((item) => anySubschema(item, predicate));
  if (predicate(schema)) return true;
  return Object.values(schema).some((value) => anySubschema(value, predicate));
}

// Every number in an instance, so a rule can be stated about the VALUES rather
// than about the mere presence of a keyword. A rule that fires on "the schema
// mentions multipleOf" would absorb any real multipleOf bug along with the
// known one.
function collectNumbers(value, into) {
  if (typeof value === "number") into.push(value);
  else if (Array.isArray(value)) for (const item of value) collectNumbers(item, into);
  else if (value && typeof value === "object") {
    for (const member of Object.values(value)) collectNumbers(member, into);
  }
  return into;
}

function classifyGenerated(schema, instance) {
  // ajv's parseInt quotient bug bites only once the quotient reaches exponent
  // notation, which JavaScript switches to at 1e21.
  const factors = [];
  anySubschema(schema, (node) => {
    if (typeof node.multipleOf === "number") factors.push(node.multipleOf);
    return false;
  });
  if (factors.length > 0) {
    const numbers = collectNumbers(instance, []);
    const overflows = numbers.some((value) =>
      factors.some((factor) => Math.abs(value / factor) >= 1e21),
    );
    if (overflows) return "ajv-multipleof-parseint";
  }
  if (anySubschema(schema, (node) => Array.isArray(node.enum) && node.enum.length === 0)) {
    return "ajv-enum-nonempty";
  }
  if (
    anySubschema(
      schema,
      (node) =>
        "contains" in node &&
        Array.isArray(node.prefixItems) &&
        node.prefixItems.includes(false),
    )
  ) {
    return "ajv-prefixitems-false-contains";
  }
  if (
    anySubschema(schema, (node) => "contains" in node) &&
    anySubschema(schema, (node) => "unevaluatedItems" in node)
  ) {
    return "ajv-contains-annotation";
  }
  return null;
}

function compareCases(report, cases) {
  const answers = runDriver({
    cases: cases.map(({ id, schema, instance }) => ({ id, schema, instance })),
  });
  const byId = new Map(answers.cases.map((answer) => [answer.id, answer]));

  for (const testCase of cases) {
    const mine = byId.get(testCase.id);
    if (!mine) {
      report.disagree(`no answer for case ${testCase.id}`);
      continue;
    }
    const theirs = ajvVerdict(testCase.schema, testCase.instance);
    if (verdictOf(mine) === verdictOf(theirs)) {
      // Both refusing counts as agreement only in the loose sense - but a
      // schema neither will compile is a schema neither will misvalidate,
      // which is the property being checked.
      report.agree();
      continue;
    }
    const divergence = testCase.divergence ?? (testCase.generated ? classifyGenerated(testCase.schema, testCase.instance) : null);
    if (divergence) {
      report.known(divergence);
      continue;
    }
    report.disagree(
      [
        `schema:   ${show(testCase.schema)}`,
        `instance: ${show(testCase.instance)}`,
        `json_schema: ${mine.status}${mine.message ? " - " + mine.message : ""}`,
        `ajv:         ${theirs.status}${theirs.message ? " - " + theirs.message : ""}`,
      ].join("\n"),
    );
  }
}

function modeCorpus() {
  const report = new Report("corpus");
  compareCases(report, expandCases(CORPUS));
  compareCases(report, expandCases(DIVERGENT));
  return report;
}

// ---------------------------------------------------------------------------
// pattern
//
// `pattern` is where the two engines are furthest apart, because JSON Schema
// borrows ECMA-262's grammar and the `regex` package has a dialect of its own.
// `ecma.mfb` translates between them; this mode checks that the translation
// preserves meaning, asked without a schema in the way.
// ---------------------------------------------------------------------------

const PATTERN_SUBJECTS = [
  "", "a", "A", "ab", "abc", "0", "007", "a1", "_", "-", ".", "/", "?",
  " ", "\t", "\n", "\r", " ", " ", " ", " ", "　", "﻿",
  "é", "Ω", "٠", "１", "\u{1F600}", "a\u{1F600}b", "ß",
  "foo bar", "foo\nbar", "éfoo", "foo-bar", "a.b", "a/b", "aa", "aaa", "ab ab",
  "2020-01-01", "user@example.com", "555-1234", "[x]", "{2}", "a&&b", "[:alpha:]",
];

const PATTERNS = [
  "", "a", "^a", "a$", "^a$", "^$", ".", ".*", ".+", "a.b", "a|b", "(a|b)c",
  "a*", "a+", "a?", "a{2}", "a{2,}", "a{1,3}", "a*?", "a+?", "(ab)+", "(?:ab)+",
  "\\d", "\\D", "\\w", "\\W", "\\s", "\\S", "\\d+", "\\w+", "\\s+",
  "[abc]", "[^abc]", "[a-z]", "[^a-z]", "[a-zA-Z0-9_]", "[\\d]", "[\\w]", "[\\s]",
  "[\\-]", "[a\\-z]", "[.]", "[\\]]", "[\\\\]", "[]", "[^]", "[\\b]",
  "\\.", "\\/", "\\$", "\\^", "\\*", "\\+", "\\?", "\\(", "\\)", "\\[", "\\{", "\\|",
  "\\n", "\\r", "\\t", "\\f", "\\v", "\\0", "\\x41", "\\u0041", "\\u{1F600}", "\\cA",
  "^[0-9]{3}-[0-9]{4}$", "^[a-z]+@[a-z]+\\.[a-z]{2,}$",
  "^\\d{4}-\\d{2}-\\d{2}$", "[-a-z]", "[a-]", "^(?:[A-Z][a-z]*)+$",
  "\\bfoo", "foo\\b", "\\Bfoo", "(a)(b)(c)", "(?<name>a)b", "\\u00e9",
  "\\p{L}", "\\p{Letter}", "\\p{Lu}", "\\p{Ll}", "\\p{N}", "\\p{Nd}", "\\p{Decimal_Number}",
  "\\p{P}", "\\p{Z}", "\\p{Zs}", "\\p{C}", "\\p{S}", "\\p{M}", "\\p{Uppercase_Letter}",
  "\\p{White_Space}", "\\p{Alphabetic}", "\\p{gc=Nd}", "\\p{General_Category=Letter}",
  "\\p{Script=Greek}", "\\p{sc=Latin}", "\\P{L}", "\\P{Nd}", "[\\p{L}]", "[\\p{L}\\d]",
  "^\\p{Letter}+$", "\\p{Cased_Letter}", "\\p{ASCII}", "\\p{Script_Extensions=Greek}", "\\pL",
  // Refused rather than translated - each is listed in KNOWN_DIVERGENCES.
  "a(?=b)", "a(?!b)", "(?<=a)b", "(?<!a)b", "(a)\\1", "\\p{L}", "\\P{L}",
  "\\k<n>", "\\a", "\\e", "\\A", "\\z", "a}", "a]", "a{", "\\01", "(?i)a",
];

function collectPatterns(schema, into) {
  if (schema === null || typeof schema !== "object") return;
  if (Array.isArray(schema)) {
    for (const item of schema) collectPatterns(item, into);
    return;
  }
  for (const [key, value] of Object.entries(schema)) {
    if (key === "pattern" && typeof value === "string") into.add(value);
    if (key === "patternProperties" && value && typeof value === "object") {
      for (const name of Object.keys(value)) into.add(name);
    }
    collectPatterns(value, into);
  }
}

function modePattern() {
  const report = new Report("pattern");
  const questions = [];
  const meta = new Map();

  const patterns = new Set(PATTERNS);
  for (const entry of [...CORPUS, ...DIVERGENT]) collectPatterns(entry.schema, patterns);

  let index = 0;
  for (const pattern of patterns) {
    let node = null;
    let compileError = null;
    try {
      node = new RegExp(pattern, "u");
    } catch (error) {
      compileError = error.message;
    }
    for (const text of PATTERN_SUBJECTS) {
      const id = `p${index++}`;
      questions.push({ id, pattern, text });
      meta.set(id, { pattern, text, node, compileError });
    }
  }

  const answers = runDriver({ patterns: questions });
  const byId = new Map(answers.patterns.map((answer) => [answer.id, answer]));

  for (const [id, question] of meta) {
    const mine = byId.get(id);
    if (!mine) {
      report.disagree(`no answer for pattern question ${id}`);
      continue;
    }

    if (question.compileError !== null) {
      // ECMA-262 refuses the pattern. So must this validator, or a schema
      // nobody else accepts would validate here.
      if (mine.status === "pattern-error") {
        report.agree();
      } else {
        report.disagree(
          [
            `pattern:  ${show(question.pattern)}`,
            `node:     SyntaxError - ${question.compileError}`,
            `json_schema: accepted it, translated to ${show(mine.translated)}`,
          ].join("\n"),
        );
      }
      continue;
    }

    if (mine.status === "pattern-error") {
      report.known(mine.code === 77050007 ? "unsupported-regex" : "schema-shape");
      continue;
    }

    const theirs = question.node.test(question.text);
    if (mine.matched === theirs) {
      report.agree();
      continue;
    }
    if (/\\[bB]/.test(question.pattern)) {
      report.known("regex-word-boundary");
      continue;
    }
    report.disagree(
      [
        `pattern:  ${show(question.pattern)}`,
        `subject:  ${show(question.text)}`,
        `translated: ${show(mine.translated)}`,
        `json_schema: ${mine.matched}   node: ${theirs}`,
      ].join("\n"),
    );
  }
  return report;
}

// ---------------------------------------------------------------------------
// fuzz
//
// A generator over the SUPPORTED subset only: every schema it builds is one
// both implementations should compile, so a compile failure is a finding
// rather than an expected refusal.
// ---------------------------------------------------------------------------

function mulberry32(seed) {
  let a = seed >>> 0;
  return function random() {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function shuffle(list, random) {
  const out = [...list];
  for (let i = out.length - 1; i > 0; i -= 1) {
    const j = Math.floor(random() * (i + 1));
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}

function makeGenerator(random) {
  const pick = (list) => list[Math.floor(random() * list.length)];
  // Definitions are collected here and installed at the ROOT of the generated
  // schema. A `$defs` nested inside a subschema is not wrong, but a
  // `$ref: "#/$defs/t"` beside it is: the fragment is a JSON Pointer into the
  // schema RESOURCE, so it names the root's `$defs` and not the neighbour's.
  // Generating that shape only tested how each implementation reports a
  // dangling reference.
  const defs = {};
  const chance = (p) => random() < p;
  const count = (max) => Math.floor(random() * (max + 1));

  const SCALARS = [
    null, true, false, 0, 1, -1, 1.5, -0.5, 2, 3, 6, 1e21, 5e-324,
    "", "a", "ab", "abc", "A", "0", "  ", "é", "\u{1F600}",
  ];

  function value(depth) {
    if (depth <= 0 || chance(0.55)) return pick(SCALARS);
    if (chance(0.5)) return Array.from({ length: count(3) }, () => value(depth - 1));
    const object = {};
    for (const name of ["a", "b", "c", "x"].slice(0, count(4))) {
      object[name] = value(depth - 1);
    }
    return object;
  }

  function schema(depth) {
    if (depth <= 0) {
      return pick([true, false, {}, { type: "integer" }, { type: "string" }]);
    }
    const node = {};
    const parts = 1 + count(2);
    for (let i = 0; i < parts; i += 1) Object.assign(node, keyword(depth, true));
    return node;
  }

  function keyword(depth, allowRef) {
    const kind = pick([
      "type", "const", "enum", "number", "string", "array", "object",
      "properties", "items", "contains", "applicator", "conditional",
      "unevaluated", allowRef ? "ref" : "type",
    ]);
    switch (kind) {
      case "type":
        return chance(0.7)
          ? { type: pick(["null", "boolean", "object", "array", "number", "string", "integer"]) }
          : { type: shuffle(["null", "boolean", "number", "string"], random).slice(0, 2) };
      case "const":
        return { const: value(2) };
      case "enum":
        return { enum: Array.from({ length: 1 + count(3) }, () => value(1)) };
      case "number": {
        const node = {};
        if (chance(0.5)) node.minimum = pick([-1, 0, 1, 2.5]);
        if (chance(0.5)) node.maximum = pick([0, 1, 10, 2.5]);
        if (chance(0.3)) node.exclusiveMinimum = pick([0, 1]);
        if (chance(0.3)) node.exclusiveMaximum = pick([1, 10]);
        if (chance(0.4)) node.multipleOf = pick([1, 2, 0.5, 3, 0.0001]);
        return node;
      }
      case "string": {
        const node = {};
        if (chance(0.5)) node.minLength = count(3);
        if (chance(0.5)) node.maxLength = count(4);
        if (chance(0.5)) node.pattern = pick(["^a", "a$", "[0-9]+", "^[a-z]*$", "\\d", "\\w+", "."]);
        return node;
      }
      case "array": {
        const node = {};
        if (chance(0.5)) node.minItems = count(3);
        if (chance(0.5)) node.maxItems = count(4);
        if (chance(0.5)) node.uniqueItems = chance(0.7);
        return node;
      }
      case "object": {
        const node = {};
        if (chance(0.6)) node.required = shuffle(["a", "b", "c"], random).slice(0, count(3));
        if (chance(0.4)) node.minProperties = count(2);
        if (chance(0.4)) node.maxProperties = count(3);
        if (chance(0.4)) node.dependentRequired = { a: ["b"] };
        return node;
      }
      case "properties": {
        const node = { properties: {} };
        for (const name of shuffle(["a", "b", "c"], random).slice(0, 1 + count(2))) {
          node.properties[name] = schema(depth - 1);
        }
        if (chance(0.4)) node.patternProperties = { "^x": schema(depth - 1) };
        if (chance(0.4)) node.additionalProperties = schema(depth - 1);
        if (chance(0.2)) node.propertyNames = { maxLength: 1 + count(2) };
        return node;
      }
      case "items": {
        const node = {};
        if (chance(0.5)) {
          node.prefixItems = Array.from({ length: 1 + count(2) }, () => schema(depth - 1));
        }
        if (chance(0.7)) node.items = schema(depth - 1);
        return node;
      }
      case "contains": {
        const node = { contains: schema(depth - 1) };
        if (chance(0.4)) node.minContains = count(2);
        if (chance(0.4)) node.maxContains = 1 + count(2);
        return node;
      }
      case "applicator": {
        const branches = Array.from({ length: 1 + count(2) }, () => schema(depth - 1));
        return pick([
          { allOf: branches },
          { anyOf: branches },
          { oneOf: branches },
          { not: schema(depth - 1) },
        ]);
      }
      case "conditional": {
        const node = { if: schema(depth - 1) };
        if (chance(0.8)) node.then = schema(depth - 1);
        if (chance(0.6)) node.else = schema(depth - 1);
        if (chance(0.3)) node.dependentSchemas = { a: schema(depth - 1) };
        return node;
      }
      case "unevaluated":
        return chance(0.5)
          ? { unevaluatedProperties: schema(depth - 1) }
          : { unevaluatedItems: schema(depth - 1) };
      case "ref":
      default: {
        const name = `t${Object.keys(defs).length}`;
        // Reserved BEFORE the body is generated, and generated with references
        // switched off: without the reservation the body's own reference names
        // the slot being filled, and `{$defs:{t:{$ref:"#/$defs/t"}}}` is a
        // self-reference rather than a reference. It is a real difference
        // between the two implementations -- it overflows ajv's stack at
        // compile time -- but it is one case, and generating it constantly
        // drowned everything else.
        defs[name] = true;
        defs[name] = keyword(1, false);
        return { $ref: `#/$defs/${name}` };
      }
    }
  }

  return { schema, value, defs };
}

function modeFuzz(options) {
  const report = new Report(`fuzz (seed ${options.seed}, ${options.rounds} schemas)`);
  const random = mulberry32(options.seed);
  const generator = makeGenerator(random);
  const cases = [];
  for (let i = 0; i < options.rounds; i += 1) {
    const body = generator.schema(3);
    const schema =
      Object.keys(generator.defs).length > 0
        ? { ...body, $defs: { ...generator.defs } }
        : body;
    for (let j = 0; j < options.instances; j += 1) {
      cases.push({
        id: `f${i}.${j}`,
        schema,
        instance: generator.value(3),
        generated: true,
      });
    }
  }
  compareCases(report, cases);
  return report;
}

// ---------------------------------------------------------------------------
// suite
//
// The official JSON-Schema-Test-Suite is a stronger oracle than ajv, because
// every case states whether the instance is valid. That turns "the two
// disagree" into "which one is wrong", and it is the only mode here that can
// find a bug both implementations share.
// ---------------------------------------------------------------------------

// Suite files this validator does not claim to pass in full, each with the
// reason. A case in one of these is allowed to end in a REFUSAL; a wrong
// verdict is still a failure.
const SUITE_EXPECTED = {
  "dynamicRef.json": "unsupported-dynamic",
  "unevaluatedItems.json": "unsupported-dynamic",
  "unevaluatedProperties.json": "unsupported-dynamic",
  "refRemote.json": "no-retrieval",
  "ref.json": "no-retrieval",
  "defs.json": "no-retrieval",
  "vocabulary.json": "custom-dialect",
};

function findSuite(explicit) {
  const candidates = [
    explicit,
    process.env.JSON_SCHEMA_TEST_SUITE,
    path.join(HERE, "JSON-Schema-Test-Suite"),
    path.join(HERE, "..", "..", "..", "third_party", "JSON-Schema-Test-Suite"),
  ].filter(Boolean);
  for (const candidate of candidates) {
    if (fs.existsSync(path.join(candidate, "tests", "draft2020-12"))) return candidate;
  }
  return null;
}

function modeSuite(options) {
  const root = findSuite(options.suite);
  if (!root) {
    console.log("\n== suite ==");
    console.log("  skipped: no JSON-Schema-Test-Suite checkout found.");
    console.log("  get one with ./fetch-suite.sh, or pass --suite <dir>.");
    return null;
  }
  const report = new Report(`suite (${path.relative(HERE, root) || root})`);
  const dir = path.join(root, "tests", "draft2020-12");
  const files = fs.readdirSync(dir).filter((name) => name.endsWith(".json")).sort();

  const cases = [];
  const meta = new Map();
  let index = 0;
  for (const file of files) {
    const groups = JSON.parse(fs.readFileSync(path.join(dir, file), "utf8"));
    for (const group of groups) {
      for (const test of group.tests) {
        const id = `s${index++}`;
        cases.push({ id, schema: group.schema, instance: test.data });
        meta.set(id, {
          file,
          group: group.description,
          test: test.description,
          valid: test.valid,
          schema: group.schema,
          instance: test.data,
        });
      }
    }
  }

  const answers = runDriver({ cases });
  const byId = new Map(answers.cases.map((answer) => [answer.id, answer]));

  for (const [id, question] of meta) {
    const mine = byId.get(id) ?? { status: "missing" };
    const truth = question.valid ? "valid" : "invalid";
    if (verdictOf(mine) === truth) {
      report.agree();
      continue;
    }
    // ORACLE_NO_EXPECTED=1 drops the allowances, which is how the list below
    // is kept honest: a file-wide entry can hide a wrong verdict in the same
    // file, so it has to be re-derived whenever the subset changes.
    const expected = process.env.ORACLE_NO_EXPECTED ? null : SUITE_EXPECTED[question.file];
    if (expected && verdictOf(mine) === "error") {
      report.known(expected);
      continue;
    }
    const theirs = ajvVerdict(question.schema, question.instance);
    report.disagree(
      [
        `${question.file} :: ${question.group} :: ${question.test}`,
        `schema:   ${show(question.schema)}`,
        `instance: ${show(question.instance)}`,
        `suite says:  ${truth}`,
        `json_schema: ${mine.status}${mine.message ? " - " + mine.message : ""}`,
        `ajv:         ${theirs.status}${theirs.message ? " - " + theirs.message : ""}`,
      ].join("\n"),
    );
  }
  return report;
}

// ---------------------------------------------------------------------------
// Command line.
// ---------------------------------------------------------------------------

function parseArguments(argv) {
  const options = { modes: [], seed: 1, rounds: 400, instances: 6, suite: null };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--seed") options.seed = Number(argv[++i]);
    else if (arg === "--rounds") options.rounds = Number(argv[++i]);
    else if (arg === "--instances") options.instances = Number(argv[++i]);
    else if (arg === "--suite") options.suite = argv[++i];
    else if (arg === "--help" || arg === "-h") options.help = true;
    else if (arg.startsWith("-")) fail(`unknown option ${arg}`);
    else options.modes.push(arg);
  }
  if (options.modes.length === 0) options.modes = ["corpus", "pattern", "fuzz", "suite"];
  return options;
}

function main() {
  const options = parseArguments(process.argv.slice(2));
  if (options.help) {
    console.log(
      "usage: node oracle.mjs [corpus|pattern|fuzz|suite ...]\n" +
        "       [--seed N] [--rounds N] [--instances N] [--suite <dir>]",
    );
    return 0;
  }

  const reports = [];
  for (const mode of options.modes) {
    switch (mode) {
      case "corpus": reports.push(modeCorpus()); break;
      case "pattern": reports.push(modePattern()); break;
      case "fuzz": reports.push(modeFuzz(options)); break;
      case "suite": reports.push(modeSuite(options)); break;
      default: fail(`unknown mode ${mode}`);
    }
  }

  let failures = 0;
  for (const report of reports) {
    if (!report) continue;
    report.print();
    failures += report.failures.length;
  }
  console.log("");
  if (failures === 0) {
    console.log("every disagreement was an expected one");
    return 0;
  }
  console.log(`${failures} unexplained disagreement(s)`);
  return 1;
}

process.exit(main());
