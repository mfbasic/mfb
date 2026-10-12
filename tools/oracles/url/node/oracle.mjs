// The Node judge for the URL oracle's string modes.
//
//   node oracle.mjs <mode> <cases.json>
//
// Prints one `case <index> <fields...>` line per case in the shape mfb/src/main.mfb prints
// (every String as hx(): `x` + UTF-8 hex), answered by Node's WHATWG `URL`,
// `URLSearchParams`, `decodeURIComponent` and `encodeURIComponent`.
import { readFileSync } from "node:fs";
import { hx, nodeUrl } from "./lib.mjs";

const [mode, casesPath] = process.argv.slice(2);
const cases = JSON.parse(readFileSync(casesPath, "utf8"));

// parse / wpt: `ok <8 fields> <href> <rt>`, `err` (WHATWG refuses), or `scheme <x>` (WHATWG
// accepts a URL whose scheme is not http/https -- outside MFBASIC's scope).
function parseJudge(s) {
  const u = nodeUrl(s);
  if (u === null) return "err";
  if (u.scheme !== "http" && u.scheme !== "https") return "scheme " + hx(u.scheme);
  const again = nodeUrl(u.href);
  const rt = again && again.href === u.href ? "rt" : "rtdiff";
  const f = [u.scheme, u.username, u.password, u.host].map(hx);
  return `ok ${f.join(" ")} ${u.port} ${hx(u.path)} ${hx(u.query)} ${hx(u.fragment)} ${hx(u.href)} ${rt}`;
}

function uriDecode(s) {
  try {
    return "ok:" + hx(decodeURIComponent(s));
  } catch (e) {
    return "err:" + e.name;
  }
}

// The WHATWG form-urlencoded value decoder (`+` -> space, lenient `%XX`, UTF-8 with
// replacement), reached through URLSearchParams. `&` is escaped so the value stays one pair.
function formDecode(s) {
  return "ok:" + hx(new URLSearchParams("a=" + s.replaceAll("&", "%26")).get("a"));
}

// pdecode: `<decodeURIComponent> <decodeURIComponent> <form decode>` -- the first answer
// judges net::percentDecode, the second encoding::percentDecode, the third
// encoding::formUrlDecode.
function pdecodeJudge(s) {
  const d = uriDecode(s);
  return `${d} ${d} ${formDecode(s)}`;
}

// query: URLSearchParams, duplicate keys collapsed last-wins. Handed "&" + s so the
// constructor's strip of one leading `?` never fires: an empty first pair is skipped by
// the parser, so the case string is parsed exactly as written.
function queryJudge(s) {
  const m = new Map();
  for (const [k, v] of new URLSearchParams("&" + s)) m.set(k, v);
  if (m.size === 0) return "ok -";
  return "ok " + [...m].map(([k, v]) => `${hx(k)}=${hx(v)}`).join(",");
}

// encode: `<encodeURIComponent> <URLSearchParams serialization>`.
function encodeJudge(s) {
  const form = new URLSearchParams([["", s]]).toString().slice(1);
  return `${hx(encodeURIComponent(s))} ${hx(form)}`;
}

const JUDGES = { parse: parseJudge, wpt: parseJudge, pdecode: pdecodeJudge, query: queryJudge, encode: encodeJudge };
const judge = JUDGES[mode];
if (!judge) {
  console.error(`unknown mode ${mode}`);
  process.exit(2);
}
const out = cases.map((s, i) => `case ${i} ${judge(s)}`);
process.stdout.write(out.join("\n") + "\n");
