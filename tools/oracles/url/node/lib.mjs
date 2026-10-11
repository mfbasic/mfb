// Shared helpers for the URL oracle's Node scripts. No npm dependencies.
import { readFileSync } from "node:fs";

// `x` + hex of the UTF-8 bytes -- the field encoding both the MFB subject and the judges print.
export const hx = (s) => "x" + Buffer.from(s, "utf8").toString("hex");

// Inverse of hx: `x<hex>` -> String.
export const unhx = (f) => Buffer.from(f.slice(1), "hex").toString("utf8");

// A corpus file: one JSON literal per line; `#` lines and blank lines are ignored.
export function readCorpus(path) {
  const out = [];
  readFileSync(path, "utf8")
    .split("\n")
    .forEach((line, i) => {
      const t = line.trim();
      if (t === "" || t.startsWith("#")) return;
      try {
        out.push(JSON.parse(t));
      } catch (e) {
        throw new Error(`${path}:${i + 1}: not a JSON literal: ${e.message}`);
      }
    });
  return out;
}

// Read a job/result file of `case <index> <rest>` lines into Map(index -> rest).
export function readCases(path) {
  const m = new Map();
  for (const line of readFileSync(path, "utf8").split("\n")) {
    const mm = /^case (\d+) ?(.*)$/.exec(line);
    if (mm) m.set(Number(mm[1]), mm[2]);
  }
  return m;
}

// Display form of a case input: JSON with every non-printable-ASCII code unit escaped,
// truncated for very long inputs.
export function show(s, max = 120) {
  let j = JSON.stringify(s).replace(/[\u007f-￿]/g, (c) => "\\u" + c.charCodeAt(0).toString(16).padStart(4, "0"));
  if (j.length > max) j = j.slice(0, max - 20) + `...(${s.length} chars)"`;
  return j;
}

export const defaultPort = (scheme) => (scheme === "https" ? 443 : 80);

// Node's view of an href as the eight MFBASIC `Url` fields, or null when WHATWG refuses it.
export function nodeUrl(s) {
  let u;
  try {
    u = new URL(s);
  } catch {
    return null;
  }
  const scheme = u.protocol.slice(0, -1);
  let host = u.hostname;
  if (host.startsWith("[")) host = host.slice(1, -1);
  return {
    scheme,
    username: u.username,
    password: u.password,
    host,
    port: u.port === "" ? defaultPort(scheme) : Number(u.port),
    path: u.pathname,
    query: u.search.slice(1),
    fragment: u.hash.slice(1),
    href: u.href,
  };
}

export const FIELDS = ["scheme", "username", "password", "host", "port", "path", "query", "fragment"];
