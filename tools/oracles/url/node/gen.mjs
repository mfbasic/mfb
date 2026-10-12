// Case generator for the URL oracle.
//
//   node gen.mjs <mode> <out-dir> [wpt-json]
//
// Writes <out-dir>/<mode>.cases.json (the case list every side reads) and, for the string
// modes, <out-dir>/<mode>.job (one line per case: the UTF-8 bytes in hex, `-` for the empty
// string) for the MFB subject. Prints the case count. Deterministic: no randomness, so a
// case index means the same input on every run.
import { writeFileSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { readCorpus } from "./lib.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CORPUS = join(HERE, "..", "corpus");
const [mode, outDir, wptPath] = process.argv.slice(2);

// Every ASCII code point, then Unicode code points chosen for a known URL-parser behaviour:
// C1 control, NBSP, soft hyphen (IDNA-mapped away), e-acute, dotted capital I, combining
// acute (joins the previous grapheme), line separator, zero-width space, ideographic space
// and full stop (an IDNA dot), BOM, fullwidth full stop, replacement character, an emoji.
const CHARS = [];
for (let c = 0; c < 0x80; c++) CHARS.push(String.fromCodePoint(c));
for (const c of [0x80, 0xa0, 0xad, 0xe9, 0x130, 0x301, 0x2028, 0x200b, 0x3000, 0x3002, 0xfeff, 0xff0e, 0xfffd, 0x1f600])
  CHARS.push(String.fromCodePoint(c));

// One template per URL position; `X` is replaced by the character under test.
const POSITIONS = [
  "httXp://h.com/",
  "Xhttp://h.com/",
  "http://uXu:p@h.com/",
  "http://u:pXp@h.com/",
  "http://hXh.com/",
  "http://h.comX/",
  "http://h.com:8X0/",
  "http://h.com/pXp",
  "http://h.com/?qXq",
  "http://h.com/#fXf",
];

function parseCases() {
  const cases = readCorpus(join(CORPUS, "parse.txt"));
  for (const t of POSITIONS) for (const c of CHARS) cases.push(t.replace("X", c));
  // Ports, both schemes.
  for (const scheme of ["http", "https"])
    for (const p of ["", "0", "1", "79", "80", "81", "443", "444", "8080", "65535", "65536", "100000", "00000", "0080", "-0", "+0"])
      cases.push(`${scheme}://h.com:${p}/`);
  // IPv4 numeric forms of a few addresses: 1- to 4-part, decimal / hex / octal / mixed.
  for (const a of [0x7f000001, 0, 0xffffffff, 0xc0a80001, 0x0a000001, 0x01020304]) {
    const b = [a >>> 24, (a >>> 16) & 255, (a >>> 8) & 255, a & 255];
    const hex = (n) => "0x" + n.toString(16);
    const oct = (n) => "0" + n.toString(8);
    const forms = [
      b.join("."),
      String(a),
      hex(a),
      oct(a),
      `${b[0]}.${(a & 0xffffff) >>> 0}`,
      `${b[0]}.${b[1]}.${(a & 0xffff) >>> 0}`,
      b.map(hex).join("."),
      b.map(oct).join("."),
      `${hex(b[0])}.${b[1]}.${oct(b[2])}.${b[3]}`,
      b.join(".") + ".",
    ];
    for (const f of forms) cases.push(`http://${f}/`);
  }
  // Long inputs.
  const r = (s, n) => s.repeat(n);
  cases.push(
    "http://h.com/" + r("a", 100000),
    "http://" + r("a", 63) + ".com/",
    "http://" + r("a", 64) + ".com/",
    "http://" + r("a.", 126) + "com/",
    "http://" + r("a", 300) + "/",
    "http://" + r("a.", 1000) + "com/",
    "http://" + r("u", 10000) + ":" + r("p", 10000) + "@h.com/",
    "http://h.com/?" + r("a=1&", 25000),
    "http://h.com/#" + r("f", 100000),
    "http://h.com/" + r("%41", 10000),
    "http://h.com:" + r("0", 1000) + "80/",
    "http://" + r("é", 1000) + ".com/",
    "http://h.com/" + r("../", 1000) + "x",
    "http://[" + r("0:", 7) + "1]/" + r("/", 1000),
  );
  return cases;
}

function pdecodeCases() {
  const cases = readCorpus(join(CORPUS, "pdecode.txt"));
  for (let b = 0; b < 256; b++) cases.push("%" + b.toString(16).toUpperCase().padStart(2, "0"));
  const ALPHA = ["0", "9", "a", "f", "A", "F", "g", "G", "+", "-", " ", "_", "x", ".", "%"];
  for (const h of ALPHA) for (const l of ALPHA) cases.push("%" + h + l);
  for (const s of readCorpus(join(CORPUS, "encode.txt"))) {
    if (/[\ud800-\udfff]/.test(s.replace(/[\ud800-\udbff][\udc00-\udfff]/g, ""))) continue;
    cases.push(encodeURIComponent(s));
  }
  return cases;
}

function queryCases() {
  const cases = readCorpus(join(CORPUS, "query.txt"));
  for (let c = 0; c < 0x80; c++) {
    const ch = String.fromCodePoint(c);
    cases.push(`k${ch}k=v${ch}v&z=1`);
  }
  return cases;
}

function encodeCases() {
  const cases = readCorpus(join(CORPUS, "encode.txt"));
  for (let c = 0; c < 0x80; c++) cases.push(String.fromCodePoint(c));
  for (const c of [0x80, 0xff, 0x100, 0x7ff, 0x800, 0xfffd, 0xffff, 0x10000, 0x10ffff]) cases.push(String.fromCodePoint(c));
  return cases;
}

// WPT urltestdata.json, when the caller has a copy (it is not vendored here): every
// object entry's `input`, parsed WITHOUT its base -- MFBASIC has no relative resolution,
// so Node is asked the same question.
function wptCases() {
  if (!wptPath) throw new Error("wpt mode needs the path to urltestdata.json");
  const data = JSON.parse(readFileSync(wptPath, "utf8"));
  const seen = new Set();
  const cases = [];
  for (const e of data) {
    if (typeof e !== "object" || seen.has(e.input)) continue;
    // A lone surrogate cannot be an MFBASIC String; WPT has a few on purpose.
    if (/[\ud800-\udfff]/.test(e.input.replace(/[\ud800-\udbff][\udc00-\udfff]/g, ""))) continue;
    seen.add(e.input);
    cases.push(e.input);
  }
  return cases;
}

function chunkedCases() {
  return readCorpus(join(CORPUS, "chunked.txt"));
}

const MODES = {
  parse: parseCases,
  wpt: wptCases,
  pdecode: pdecodeCases,
  query: queryCases,
  encode: encodeCases,
  "chunk-client": chunkedCases,
  "chunk-server": chunkedCases,
};
if (!MODES[mode]) {
  console.error(`unknown mode ${mode}`);
  process.exit(2);
}
const cases = MODES[mode]();
writeFileSync(join(outDir, `${mode}.cases.json`), JSON.stringify(cases));
if (!mode.startsWith("chunk-"))
  writeFileSync(join(outDir, `${mode}.job`), cases.map((s) => (s === "" ? "-" : Buffer.from(s, "utf8").toString("hex"))).join("\n") + "\n");
console.log(cases.length);
