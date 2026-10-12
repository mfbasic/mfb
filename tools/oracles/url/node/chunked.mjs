// The chunked transfer-coding modes of the URL oracle. Everything binds 127.0.0.1 only.
//
//   node chunked.mjs client <cases.json> <mfb-exe> <mfb-out> <node-out>
//   node chunked.mjs server <cases.json> <mfb-exe> <mfb-out> <node-out>
//
// client: a raw-socket server answers GET /c/<i> with `HTTP/1.1 200 OK`, case i's head
//   lines, `Connection: close`, and case i's body bytes, then closes. The MFB subject
//   (MFB_URL_MODE=chunk-client) reads every case with http::read; the judge reads every
//   case with Node's http client (llhttp). Lines: `ok <status> x<body-hex>` / `err <code>`.
// server: a raw-socket client writes `POST /echo HTTP/1.1`, Host, case i's head lines,
//   `Connection: close`, and case i's body, then half-closes. Against the MFB subject
//   (MFB_URL_MODE=chunk-server, whose handler answers `x<hex of Request.body>`) and against
//   Node's http.createServer (the judge records the body the request handler received, or
//   the llhttp error code of `clientError`). Lines: `ok x<body-hex>` / `reject <status|code>`
//   / `noresp`.
import { spawn } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import http from "node:http";
import net from "node:net";

const [direction, casesPath, mfbExe, mfbOut, nodeOut] = process.argv.slice(2);
const cases = JSON.parse(readFileSync(casesPath, "utf8"));
const HOST = "127.0.0.1";
const CASE_TIMEOUT_MS = 5000;
const head = (c) => c.head ?? ["Transfer-Encoding: chunked"];
const bytes = (s) => Buffer.from(s, "latin1");

function listen(server) {
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, HOST, () => resolve(server.address().port));
  });
}

// Run the MFB subject to completion (or kill it at the deadline); resolve with its stdout.
function runMfb(env, deadlineMs, onLine) {
  return new Promise((resolve, reject) => {
    const child = spawn(mfbExe, [], { env: { ...process.env, ...env }, stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    child.stdout.on("data", (d) => {
      out += d;
      if (onLine) onLine(out);
    });
    child.stderr.on("data", (d) => (err += d));
    const timer = setTimeout(() => child.kill("SIGKILL"), deadlineMs);
    child.on("error", reject);
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      resolve({ out, err, code, signal, child });
    });
    if (onLine) onLine(out, child);
  });
}

// ------------------------------------------------------------------ client direction
async function clientMode() {
  const raw = net.createServer((sock) => {
    let req = Buffer.alloc(0);
    sock.on("data", (d) => {
      req = Buffer.concat([req, d]);
      const end = req.indexOf("\r\n\r\n");
      if (end < 0) return;
      const m = /^GET \/c\/(\d+) /.exec(req.toString("latin1"));
      const c = m ? cases[Number(m[1])] : null;
      if (!c) return sock.destroy();
      const lines = ["HTTP/1.1 200 OK", ...head(c), "Connection: close"].join("\r\n") + "\r\n\r\n";
      sock.end(Buffer.concat([bytes(lines), bytes(c.body)]));
    });
    sock.on("error", () => {});
  });
  const port = await listen(raw);

  const judge = [];
  for (let i = 0; i < cases.length; i++) judge.push(`case ${i} ${await nodeGet(port, i)}`);
  writeFileSync(nodeOut, judge.join("\n") + "\n");

  const r = await runMfb({ MFB_URL_MODE: "chunk-client", MFB_URL_PORT: String(port), MFB_URL_COUNT: String(cases.length) }, cases.length * CASE_TIMEOUT_MS + 30000);
  writeFileSync(mfbOut, r.out);
  raw.close();
  if (r.code !== 0) {
    process.stderr.write(r.err);
    console.error(`the MFB subject exited ${r.code ?? r.signal} in chunk-client`);
    process.exit(2);
  }
}

function nodeGet(port, i) {
  return new Promise((resolve) => {
    let settled = false;
    const done = (v) => {
      if (!settled) {
        settled = true;
        resolve(v);
      }
    };
    const req = http.get({ host: HOST, port, path: `/c/${i}`, agent: false }, (res) => {
      const parts = [];
      res.on("data", (d) => parts.push(d));
      res.on("aborted", () => done("err aborted"));
      res.on("error", (e) => done(`err ${e.code ?? e.message}`));
      res.on("end", () => done(`ok ${res.statusCode} x${Buffer.concat(parts).toString("hex")}`));
    });
    req.setTimeout(CASE_TIMEOUT_MS, () => {
      done("err timeout");
      req.destroy();
    });
    req.on("error", (e) => done(`err ${e.code ?? e.message}`));
  });
}

// ------------------------------------------------------------------ server direction
function rawRequest(port, c) {
  return new Promise((resolve) => {
    const sock = net.connect(port, HOST);
    const parts = [];
    let settled = false;
    const done = () => {
      if (settled) return;
      settled = true;
      sock.destroy();
      const resp = Buffer.concat(parts);
      if (resp.length === 0) return resolve("noresp");
      const text = resp.toString("latin1");
      const status = Number(/^HTTP\/1\.[01] (\d{3})/.exec(text)?.[1] ?? 0);
      const body = text.slice(text.indexOf("\r\n\r\n") + 4);
      resolve(status === 200 ? `ok ${body.trim()}` : `reject ${status}`);
    };
    sock.on("connect", () => {
      const lines = ["POST /echo HTTP/1.1", `Host: ${HOST}:${port}`, ...head(c), "Connection: close"].join("\r\n") + "\r\n\r\n";
      sock.end(Buffer.concat([bytes(lines), bytes(c.body)]));
    });
    sock.on("data", (d) => parts.push(d));
    sock.on("close", done);
    sock.on("error", done);
    sock.setTimeout(CASE_TIMEOUT_MS, done);
  });
}

async function freePort() {
  const s = net.createServer();
  const port = await listen(s);
  await new Promise((r) => s.close(r));
  return port;
}

async function serverMode() {
  // The judge: Node's own HTTP server.
  let current = null;
  const node = http.createServer((req, res) => {
    const parts = [];
    req.on("data", (d) => parts.push(d));
    req.on("aborted", () => (current = current ?? "reject aborted"));
    req.on("end", () => {
      current = current ?? `ok x${Buffer.concat(parts).toString("hex")}`;
      res.end("ok");
    });
  });
  node.on("clientError", (e, sock) => {
    current = current ?? `reject ${e.code ?? e.message}`;
    sock.destroy();
  });
  const nodePort = await listen(node);
  const judge = [];
  for (let i = 0; i < cases.length; i++) {
    current = null;
    await rawRequest(nodePort, cases[i]);
    await new Promise((r) => setTimeout(r, 20));
    judge.push(`case ${i} ${current ?? "noresp"}`);
  }
  node.close();
  writeFileSync(nodeOut, judge.join("\n") + "\n");

  // The subject: MFB's http::server, one handleRequest per case.
  const port = await freePort();
  const child = spawn(mfbExe, [], {
    env: { ...process.env, MFB_URL_MODE: "chunk-server", MFB_URL_PORT: String(port), MFB_URL_COUNT: String(cases.length) },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let err = "";
  child.stderr.on("data", (d) => (err += d));
  const exited = new Promise((r) => child.on("close", (code, signal) => r(code ?? signal)));
  const killer = setTimeout(() => child.kill("SIGKILL"), cases.length * CASE_TIMEOUT_MS + 30000);
  await new Promise((resolve, reject) => {
    let out = "";
    child.stdout.on("data", (d) => {
      out += d;
      if (out.includes("listening")) resolve();
    });
    exited.then(() => reject(new Error(`the MFB server exited before listening: ${err}`)));
  });
  const lines = [];
  for (let i = 0; i < cases.length; i++) lines.push(`case ${i} ${await rawRequest(port, cases[i])}`);
  const code = await exited;
  clearTimeout(killer);
  writeFileSync(mfbOut, lines.join("\n") + "\n");
  if (code !== 0) {
    process.stderr.write(err);
    console.error(`the MFB server exited ${code} in chunk-server`);
    process.exit(2);
  }
}

if (direction === "client") await clientMode();
else if (direction === "server") await serverMode();
else {
  console.error("usage: chunked.mjs client|server <cases.json> <mfb-exe> <mfb-out> <node-out>");
  process.exit(2);
}
