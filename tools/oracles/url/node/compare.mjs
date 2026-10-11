// Comparator for the URL oracle.
//
//   node compare.mjs <mode> <cases.json> <mfb.txt> <node.txt> [report.json]
//
// Reads both sides' `case` lines, sorts every disagreement into a named GROUP by a rule
// (never by case index), and prints the groups with their count, class and the shortest
// example. The class of each group is declared in GROUPS below:
//   a  MFBASIC disagrees with its own spec, and the spec agrees with Node -- a bug
//   b  a difference the spec allows (MFBASIC is deliberately narrower or does not
//      normalize) -- declared, never fails the run
//   c  the spec is silent; an owner decision is needed
// Exit 0 when every disagreement is in a class-b group, 1 when any is class a / c or in a
// group GROUPS does not know, 2 when a side failed to answer every case.
import { readFileSync, writeFileSync } from "node:fs";
import { FIELDS, hx, nodeUrl, readCases, show, unhx } from "./lib.mjs";

const [mode, casesPath, mfbPath, nodePath, reportPath] = process.argv.slice(2);
const cases = JSON.parse(readFileSync(casesPath, "utf8"));
const mine = readCases(mfbPath);
const theirs = readCases(nodePath);
for (const [who, m] of [["mfb", mine], ["node", theirs]]) {
  if (m.size !== cases.length) {
    console.error(`${who} answered ${m.size} case(s) for ${mode}, expected ${cases.length}`);
    process.exit(2);
  }
}

// Group key -> [class, one-line meaning]. Keys are produced by the classify* functions.
const GROUPS = {
  // ---- parse / wpt
  "parse/out-of-scope-scheme": ["b", "WHATWG accepts a non-http(s) URL; MFBASIC supports http/https only (spec: Parsing model step 1)"],
  "parse/strict/missing-slashes": ["b", "WHATWG accepts a special URL without `//` (`http:h`, `http:/h`, `http:\\\\h`); the spec grammar requires `scheme \"://\"`"],
  "parse/strict/empty-host-after-slashes": ["b", "WHATWG skips extra slashes or backslashes before the host (`http:///h`, `http://\\h`); the spec grammar takes the authority right after `://` and fails an empty host"],
  "parse/strict/scheme-with-tab-newline-or-c0": ["b", "WHATWG strips tab/LF/CR anywhere and C0/space at the ends before parsing; the spec does no input cleanup"],
  "parse/strict/port-not-digits": ["b", "WHATWG accepts a port after stripping tab/LF/CR; MFBASIC keeps them and fails the port as non-digit (spec: no input cleanup, non-digit port fails)"],
  "parse/strict/host-tab-newline": ["b", "WHATWG strips tab/LF/CR before parsing, so it accepts a host written with one; MFBASIC keeps them and fails the host on a forbidden character (spec: no input cleanup, a forbidden host character fails; owner decision 1 on bug-979)"],
  "parse/host-keeps-tab-newline": ["a", "both accept; WHATWG strips tab/LF/CR from the input, MFBASIC stores them in `host` (`http://h\\r\\n/` -> host `h\\r\\n`): a garbage host the spec says fails (bug-873 item 5)"],
  "parse/input-cleanup": ["b", "both accept; WHATWG strips tab/LF/CR anywhere and C0/space at the ends, MFBASIC keeps them in the component (spec: no normalization)"],
  "parse/strict/backslash-authority": ["a", "WHATWG treats `\\` as `/` in an http(s) URL, so it ends the authority; MFBASIC keeps it in the authority and then fails (port or IPv6 trailing characters). Owner decision 3 on bug-979: `\\` is `/`"],
  "parse/strict/other": [null, "MFBASIC refuses a URL WHATWG accepts, for a reason no rule above explains"],
  "parse/lenient/host-forbidden-code-point": ["a", "MFBASIC accepts a host containing a WHATWG forbidden host code point (space, control, `<>^|%` ...); spec Parse failures lists \"garbage host\", owner decision on bug-873 item 5: validate hosts"],
  "parse/lenient/host-ipv4-invalid": ["a", "MFBASIC accepts a host that ends in a number but is not a valid IPv4 address (`256.0.0.1`, `1.2.3.09`, `foo.09`)"],
  "parse/lenient/host-ipv6-invalid": ["a", "MFBASIC accepts any text between `[` and `]` as an IPv6 host (`[1::2::3]`, `[g::1]`, zones, `[v1.fe]`)"],
  "parse/lenient/host-idna-invalid": ["a", "MFBASIC accepts a non-ASCII or `xn--` host that UTS #46 (WHATWG domain-to-ASCII) refuses"],
  "parse/lenient/host-percent-invalid": ["a", "MFBASIC accepts a host whose percent-decoding WHATWG refuses (`%zz`, `%00`, invalid UTF-8)"],
  "parse/lenient/port-out-of-range": ["a", "MFBASIC accepts a port WHATWG refuses"],
  "parse/lenient/backslash-authority": ["a", "the authority holds a `\\`: WHATWG ends the authority there (then refuses what is left, e.g. a non-digit port), MFBASIC keeps it inside userinfo/host. Owner decision 3 on bug-979: `\\` is `/`"],
  "parse/lenient/other": [null, "MFBASIC accepts a URL WHATWG refuses, for a reason no rule above explains"],
  "parse/split-differs/backslash": ["a", "both accept, but a `\\` in the authority moves the host: WHATWG reads `\\` as `/` (host ends there), MFBASIC keeps it in userinfo/host. `http://good.com\\@evil.com/` is host good.com to WHATWG and evil.com to MFBASIC -- a parser differential. Owner decision 3 on bug-979: `\\` is `/`"],
  "parse/split-differs": ["c", "both accept, but the URL is split into different components even after WHATWG normalization of MFBASIC's fields (backslash or tab/LF/CR handling moves text between components)"],
  "parse/host-ipv4-noncanonical": ["a", "both accept; WHATWG rewrites a numeric/short/hex/octal IPv4 host to dotted-decimal, MFBASIC keeps the text (resolver then decides what it means). Owner decision 2 on bug-979: canonicalize like WHATWG"],
  "parse/normalized-only": ["b", "both accept and agree after WHATWG normalization of MFBASIC's fields (case, IDNA, percent-encoding, dot segments, IPv6 compression); the spec says toUrl performs no normalization"],
  "parse/render-not-inverse": ["a", "toString(toUrl(x)) does not parse back to the same Url (spec: rendering is the inverse of parsing)"],
  "parse/judge-roundtrip": [null, "Node's own href did not re-parse to itself (a judge oddity, inspect)"],
  // ---- chunk-client / chunk-server (the case name picks the rule; see CHUNK_RULES)
  "chunk/data-crlf-unchecked": ["a", "the 2 bytes after a chunk's data are skipped, not checked to be CRLF (`4\\r\\nWikiXX5\\r\\n...` decodes to `Wikipedia`); spec grammar `chunk = HEX [\";\" ext] CRLF data CRLF` + \"Malformed framing fails\", llhttp refuses (HPE_STRICT)"],
  "chunk/size-whitespace": ["a", "the size line is trimmed, so whitespace before or after the hex size is accepted; spec grammar `HEX [\";\" ext] CRLF`, llhttp refuses (HPE_INVALID_CHUNK_SIZE)"],
  "chunk/extension-shape": ["c", "a chunk extension llhttp refuses is accepted: an empty `;` or BWS before `;` (RFC 9112 allows BWS; llhttp strict mode does not); the spec says only \"ignored\""],
  "chunk/last-chunk-no-final-crlf": ["c", "`0\\r\\n` then EOF is accepted as a complete body; the spec grammar ends at `last-chunk = \"0\" CRLF`, RFC 9112 and llhttp require the trailer section's final CRLF"],
  "chunk/trailer-unvalidated": ["c", "a malformed trailer line is accepted (discarded unparsed); llhttp refuses it; the spec says trailers are discarded"],
  "chunk/bytes-after-message": ["c", "bytes after the terminating chunk on a Connection: close message are ignored; llhttp refuses them (HPE_CLOSED_CONNECTION)"],
  "chunk/te-substring": ["b", "the client de-chunks whenever Transfer-Encoding contains the substring `chunked` (spec: Response Body Decoding), even when chunked is not the final coding or is part of a longer token; Node then reads the raw bytes to close"],
  "chunk/te-with-content-length": ["b", "the client accepts a response carrying both Transfer-Encoding: chunked and Content-Length (spec: the client-side parser stays lenient); llhttp refuses"],
  "chunk/other": [null, "a chunked-framing disagreement no rule explains"],
  // ---- pdecode
  "pdecode/net/signed-escape": ["a", "net::percentDecode accepts `%+X` / `%-X`-shaped escapes through toInt's sign (bug-868)"],
  "pdecode/net/other": [null, "net::percentDecode disagrees with decodeURIComponent for a reason no rule explains"],
  "pdecode/enc/other": [null, "encoding::percentDecode disagrees with decodeURIComponent"],
  "pdecode/form/strict": ["b", "encoding::formUrlDecode fails a malformed escape or invalid UTF-8 that the WHATWG form decoder passes through or replaces with U+FFFD (spec 08_encoding: decoding fails)"],
  "pdecode/form/other": [null, "encoding::formUrlDecode disagrees with the WHATWG form decoder on a value"],
  // ---- query
  "query/raw-fallback": ["b", "a component with a malformed escape or invalid UTF-8: parseQuery keeps the whole component raw (spec: tolerant raw fallback), WHATWG decodes the valid escapes and replaces bad bytes with U+FFFD"],
  "query/signed-escape": ["a", "parseQuery decodes `%+X` as an escape (bug-868, the shared decoder)"],
  "query/other": [null, "parseQuery disagrees with URLSearchParams for a reason no rule explains"],
  // ---- encode
  "encode/percent/escapes-more": ["b", "percentEncode escapes `!'()*`, which encodeURIComponent leaves alone; both are valid RFC 3986 encodings (spec: only the unreserved set passes through)"],
  "encode/form/escapes-more": ["b", "formUrlEncode escapes `*-._`, which the WHATWG serializer leaves alone (spec: every non-alphanumeric byte becomes %XX)"],
  "encode/percent/other": [null, "percentEncode output that decodeURIComponent does not decode back to the input"],
  "encode/form/other": [null, "formUrlEncode output that the WHATWG form decoder does not decode back to the input"],
};

const groups = new Map();
function note(key, i, detail) {
  if (!groups.has(key)) groups.set(key, []);
  const c = cases[i];
  groups.get(key).push({ i, input: typeof c === "string" ? c : `${c.name} ${(c.head ?? []).join("; ")} ${c.body}`, ...detail });
}

// ---------------------------------------------------------------- parse / wpt
const FORBIDDEN_HOST = /[\u0000- #%/:<>?@[\\\]^|\u007f]/;

function mfbParse(line) {
  const t = line.split(" ");
  if (t[0] === "err") return { err: t[1], message: t[2] ? unhx(t[2]) : "" };
  const u = {};
  FIELDS.forEach((f, k) => (u[f] = f === "port" ? Number(t[1 + k]) : unhx(t[1 + k])));
  u.href = unhx(t[9]);
  u.rt = t[10];
  return u;
}

function nodeParse(line) {
  const t = line.split(" ");
  if (t[0] === "err") return { err: true };
  if (t[0] === "scheme") return { scheme: unhx(t[1]), outOfScope: true };
  const u = {};
  FIELDS.forEach((f, k) => (u[f] = f === "port" ? Number(t[1 + k]) : unhx(t[1 + k])));
  u.href = unhx(t[9]);
  u.rt = t[10];
  return u;
}

const bracket = (h) => (h.includes(":") ? `[${h}]` : h);
const endsInNumber = (h) => {
  const parts = h.split(".");
  if (parts[parts.length - 1] === "" && parts.length > 1) parts.pop();
  const last = parts[parts.length - 1];
  return /^[0-9]+$/.test(last) || /^0[xX][0-9a-fA-F]*$/.test(last);
};

// Why WHATWG refuses an href MFBASIC accepted.
function lenientReason(s, m) {
  const auth = s.slice(s.indexOf("://") + 3);
  const end = auth.search(/[/?#]/);
  const authority = end < 0 ? auth : auth.slice(0, end);
  if (authority.includes("\\")) return "backslash-authority";
  if (nodeUrl(`http://h.com:${m.port}/`) === null) return "port-out-of-range";
  if (m.host.includes(":") || authority.slice(authority.lastIndexOf("@") + 1).startsWith("[")) return "host-ipv6-invalid";
  if (nodeUrl(`http://${bracket(m.host)}/`) !== null) return "other";
  if (/%/.test(m.host)) {
    let decoded = null;
    try {
      decoded = decodeURIComponent(m.host);
    } catch {}
    if (decoded === null || FORBIDDEN_HOST.test(decoded)) return "host-percent-invalid";
  }
  if (FORBIDDEN_HOST.test(m.host)) return "host-forbidden-code-point";
  if (endsInNumber(m.host.toLowerCase())) return "host-ipv4-invalid";
  if (/[^\u0000-\u007f]/.test(m.host) || /(^|\.)xn--/i.test(m.host)) return "host-idna-invalid";
  return "other";
}

// Why MFBASIC refuses an href WHATWG accepted, from its message and the input's shape.
function strictReason(s, m) {
  const msg = m.message;
  if (/[\t\n\r]/.test(s.split("://")[0] ?? "") || /^[\u0000- ]/.test(s)) return "scheme-with-tab-newline-or-c0";
  if (/missing scheme separator/.test(msg)) return "missing-slashes";
  if (/unsupported URL scheme/.test(msg)) return "scheme-with-tab-newline-or-c0";
  if (/empty host/.test(msg) && /^[a-zA-Z]+:\/\/[\\/]/.test(s)) return "empty-host-after-slashes";
  const auth = s.slice(s.indexOf("://") + 3);
  const end = auth.search(/[/?#]/);
  const authority = end < 0 ? auth : auth.slice(0, end);
  if (authority.includes("\\")) return "backslash-authority";
  if (/non-digit in port/.test(msg)) return "port-not-digits";
  if (/forbidden character in host/.test(msg) && /[\t\n\r]/.test(authority)) return "host-tab-newline";
  return "other";
}

const IPV4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/;

// WHATWG normalization of MFBASIC's fields, one at a time: what Node makes of each field
// placed alone in an otherwise fixed URL. Equal to Node's parse of the whole input means
// the two agree on how the input is split and differ only in normalization.
function normalizeFields(m) {
  const n = {};
  const at = (href, f) => {
    const u = nodeUrl(href);
    return u === null ? null : u[f];
  };
  n.scheme = m.scheme;
  n.host = at(`http://${bracket(m.host)}/`, "host");
  n.port = m.port;
  n.username = m.username === "" ? "" : at(`http://${m.username.replace(/[@/\\?#]/g, encodeURIComponent)}@h/`, "username");
  n.password = m.password === "" ? "" : at(`http://u:${m.password.replace(/[@/\\?#]/g, encodeURIComponent)}@h/`, "password");
  n.path = at(`http://h${m.path.replace(/[?#]/g, encodeURIComponent)}`, "path");
  n.query = m.query === "" ? "" : at(`${m.scheme}://h/?${m.query.replace(/#/g, "%23")}`, "query");
  n.fragment = m.fragment === "" ? "" : at(`http://h/#${m.fragment}`, "fragment");
  return n;
}

function classifyParse(i, ml, nl) {
  const s = cases[i];
  const m = mfbParse(ml);
  const n = nodeParse(nl);
  if (!m.err && m.rt !== "rt") note("parse/render-not-inverse", i, { mfb: ml, node: nl });
  if (!n.err && !n.outOfScope && n.rt !== "rt") note("parse/judge-roundtrip", i, { mfb: ml, node: nl });
  if (n.outOfScope) {
    if (m.err) return "agree-out-of-scope";
    note("parse/out-of-scope-scheme", i, { mfb: ml, node: `scheme ${n.scheme}` });
    return "diverge";
  }
  if (m.err && n.err) return "agree";
  if (m.err) {
    note(`parse/strict/${strictReason(s, m)}`, i, { mfb: `err ${m.err} ${JSON.stringify(m.message)}`, node: n.href });
    return "diverge";
  }
  if (n.err) {
    note(`parse/lenient/${lenientReason(s, m)}`, i, { mfb: fieldsText(m), node: "TypeError: Invalid URL" });
    return "diverge";
  }
  const same = FIELDS.every((f) => m[f] === n[f]) && m.href === n.href;
  if (same) return "agree";
  const nm = normalizeFields(m);
  const off = FIELDS.filter((f) => nm[f] !== n[f]);
  const detail = { mfb: fieldsText(m), node: fieldsText(n), fields: FIELDS.filter((f) => m[f] !== n[f]).join(",") };
  if (off.length > 0) {
    const cleanup = /[\t\n\r]/.test(s) || /^[\u0000-\u0020]|[\u0000-\u0020]$/.test(s);
    const key = s.includes("\\") ? "parse/split-differs/backslash" : cleanup ? "parse/input-cleanup" : "parse/split-differs";
    note(key, i, { ...detail, fields: off.join(",") });
    return "diverge";
  }
  if (FIELDS.every((f) => f === "host" || m[f] === n[f]) && IPV4.test(n.host) && n.host !== m.host.toLowerCase().replace(/\.$/, "")) {
    note("parse/host-ipv4-noncanonical", i, detail);
    return "diverge";
  }
  if (/[\t\n\r]/.test(m.host)) {
    note("parse/host-keeps-tab-newline", i, detail);
    return "diverge";
  }
  note(`parse/normalized-only/${detail.fields || "empty-query-or-fragment-marker"}`, i, detail);
  return "diverge";
}

function fieldsText(u) {
  return FIELDS.map((f) => `${f}=${JSON.stringify(u[f])}`).join(" ") + ` href=${JSON.stringify(u.href)}`;
}

// ---------------------------------------------------------------- pdecode
const okVal = (tok) => (tok.startsWith("ok:") ? unhx(tok.slice(3)) : null);
const SIGNED = /%[+-]/;

function classifyPdecode(i, ml, nl) {
  const s = cases[i];
  const [mNet, mEnc, mForm] = ml.split(" ");
  const [nUri, , nForm] = nl.split(" ");
  let verdict = "agree";
  const cmp = (tok, ref) => (tok.startsWith("err") && ref.startsWith("err")) || okVal(tok) === okVal(ref) && tok.startsWith("ok") && ref.startsWith("ok");
  if (!cmp(mNet, nUri)) {
    const key = SIGNED.test(s) ? "signed-escape" : "other";
    note(`pdecode/net/${key}`, i, { mfb: decodeShow(mNet), node: decodeShow(nUri) });
    verdict = "diverge";
  }
  if (!cmp(mEnc, nUri)) {
    note("pdecode/enc/other", i, { mfb: decodeShow(mEnc), node: decodeShow(nUri) });
    verdict = "diverge";
  }
  if (!cmp(mForm, nForm)) {
    note(mForm.startsWith("err") ? "pdecode/form/strict" : "pdecode/form/other", i, { mfb: decodeShow(mForm), node: decodeShow(nForm) });
    verdict = "diverge";
  }
  return verdict;
}

const decodeShow = (tok) => (tok.startsWith("ok:") ? "ok " + show(unhx(tok.slice(3))) : tok);

// ---------------------------------------------------------------- query
function entries(line) {
  const body = line.slice(3);
  if (body === "-") return new Map();
  return new Map(body.split(",").map((kv) => kv.split("=").map(unhx)));
}

function rawPairs(s) {
  const m = new Map();
  for (const pair of s.split("&")) {
    if (pair === "") continue;
    const eq = pair.indexOf("=");
    m.set(eq < 0 ? pair : pair.slice(0, eq), eq < 0 ? "" : pair.slice(eq + 1));
  }
  return m;
}

function classifyQuery(i, ml, nl) {
  const s = cases[i];
  const me = entries(ml);
  const ne = entries(nl);
  const fmt = (m) => [...m].sort().map(([k, v]) => `${show(k)}=${show(v)}`).join(" ");
  if (fmt(me) === fmt(ne)) return "agree";
  const detail = { mfb: fmt(me), node: fmt(ne) };
  // Every MFB entry that differs from Node must be either Node's decoding or the raw text of
  // a component (the documented fallback) for raw-fallback to explain it.
  const raws = new Set();
  for (const [k, v] of rawPairs(s)) raws.add(k).add(v);
  const keysM = [...me.keys()].filter((k) => !ne.has(k) || ne.get(k) !== me.get(k));
  const rawExplains = keysM.every((k) => raws.has(k) || ne.has(k)) && keysM.every((k) => raws.has(me.get(k)) || ne.get(k) === me.get(k));
  if (SIGNED.test(s)) note("query/signed-escape", i, detail);
  else if (rawExplains) note("query/raw-fallback", i, detail);
  else note("query/other", i, detail);
  return "diverge";
}

// ---------------------------------------------------------------- encode
function classifyEncode(i, ml, nl) {
  const s = cases[i];
  const [mPct, mForm] = ml.split(" ").map(unhx);
  const [nPct, nForm] = nl.split(" ").map(unhx);
  let verdict = "agree";
  if (mPct !== nPct) {
    let back = null;
    try {
      back = decodeURIComponent(mPct);
    } catch {}
    const extra = /^[A-Za-z0-9\-._~]*$/.test(mPct.replace(/%[0-9A-F]{2}/g, "")) && back === s;
    note(extra ? "encode/percent/escapes-more" : "encode/percent/other", i, { mfb: mPct, node: nPct });
    verdict = "diverge";
  }
  if (mForm !== nForm) {
    const back = new URLSearchParams("a=" + mForm).get("a");
    note(back === s ? "encode/form/escapes-more" : "encode/form/other", i, { mfb: mForm, node: nForm });
    verdict = "diverge";
  }
  return verdict;
}

// ---------------------------------------------------------------- chunk-client / chunk-server
const CHUNK_RULES = {
  "crlf-after-data-is-XX": "data-crlf-unchecked",
  "no-crlf-after-data": "data-crlf-unchecked",
  "zero-straight-after-data": "data-crlf-unchecked",
  "space-before-size": "size-whitespace",
  "space-after-size": "size-whitespace",
  "tab-after-size": "size-whitespace",
  "extension-bare": "extension-shape",
  "extension-space-before-semi": "extension-shape",
  "last-chunk-no-final-crlf": "last-chunk-no-final-crlf",
  "trailer-malformed": "trailer-unvalidated",
  "garbage-after-terminator": "bytes-after-message",
  "te-chunked-not-final": "te-substring",
  "te-chunkedx": "te-substring",
  "te-xchunked": "te-substring",
  "te-and-content-length": "te-with-content-length",
};

// Verdict and body of one side's line: `ok [status] x<hex>` -> body hex, anything else -> refused.
function chunkVerdict(line) {
  const t = line.split(" ");
  if (t[0] !== "ok") return { ok: false, text: line };
  const body = t[t.length - 1];
  return { ok: true, body, text: `ok ${show(Buffer.from(body.slice(1), "hex").toString("latin1"))}` };
}

function classifyChunk(i, ml, nl) {
  const m = chunkVerdict(ml);
  const n = chunkVerdict(nl);
  if (m.ok === n.ok && (!m.ok || m.body === n.body)) return "agree";
  const rule = CHUNK_RULES[cases[i].name] ?? "other";
  note(`chunk/${rule}`, i, { mfb: m.text, node: n.text });
  return "diverge";
}

// ---------------------------------------------------------------- run
const CLASSIFY = { "chunk-client": classifyChunk, "chunk-server": classifyChunk, parse: classifyParse, wpt: classifyParse, pdecode: classifyPdecode, query: classifyQuery, encode: classifyEncode };
const classify = CLASSIFY[mode];
const tally = { agree: 0, "agree-out-of-scope": 0, diverge: 0 };
for (let i = 0; i < cases.length; i++) tally[classify(i, mine.get(i), theirs.get(i))]++;

let failing = 0;
const report = [];
const keys = [...groups.keys()].sort();
for (const key of keys) {
  const list = groups.get(key);
  const [cls, meaning] = GROUPS[key] ?? GROUPS[key.split("/").slice(0, 2).join("/")] ?? [null, "UNKNOWN GROUP -- add it to GROUPS in compare.mjs"];
  const shortest = [...list].sort((a, b) => a.input.length - b.input.length || a.i - b.i);
  if (cls !== "b") failing += list.length;
  report.push({ key, class: cls ?? "?", meaning, count: list.length, examples: shortest.slice(0, 12) });
  const label = cls ? `class ${cls}` : "UNCLASSIFIED";
  console.log(`  ${mode}: ${key} [${label}] ${list.length} case(s) -- ${meaning}`);
  for (const ex of shortest.slice(0, cls === "b" ? 1 : 3)) {
    console.log(`      case ${ex.i} ${show(ex.input)}`);
    console.log(`        mfb:  ${ex.mfb}`);
    console.log(`        node: ${ex.node}`);
  }
}
const scope = tally["agree-out-of-scope"] ? `, ${tally["agree-out-of-scope"]} out of scope (both refuse as http/https)` : "";
console.log(`${mode}: ${cases.length} case(s): ${tally.agree} agree${scope}, ${tally.diverge} diverge; ${failing} divergence entr(ies) in class a / c / unclassified groups`);
if (reportPath) writeFileSync(reportPath, JSON.stringify({ mode, total: cases.length, tally, groups: report }, null, 1));
process.exit(failing > 0 ? 1 : 0);
