#!/usr/bin/env node
// MCP server exposing the MFBASIC built-in help (`mfb man`) and the embedded
// language specification (`mfb spec`) to MCP-aware assistants. Each tool shells
// out to the `mfb` binary and returns its rendered text output.

import { spawn } from "node:child_process";
import { appendFileSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

const HERE = dirname(fileURLToPath(import.meta.url));
// tools/mcp -> repo root is two levels up.
const REPO_ROOT = resolve(HERE, "..", "..");

// Resolve the `mfb` binary to spawn, in priority order:
//   1. $MFB_BIN (explicit override)
//   2. a release/debug build inside this checkout
//   3. `mfb` on PATH
function resolveMfbBin() {
  const fromEnv = process.env.MFB_BIN;
  if (fromEnv && fromEnv.trim().length > 0) {
    return fromEnv;
  }
  const candidates = [
    join(REPO_ROOT, "target", "release", "mfb"),
    join(REPO_ROOT, "target", "debug", "mfb"),
  ];
  for (const candidate of candidates) {
    if (existsSync(candidate)) {
      return candidate;
    }
  }
  return "mfb";
}

const MFB_BIN = resolveMfbBin();

// Clients (e.g. Claude Code) capture the server's stderr into their own opaque
// logs, so logging there is invisible in practice. Write our own log file the
// user can `tail -f`. Path is overridable via $MFB_MCP_LOG.
const LOG_FILE = process.env.MFB_MCP_LOG || join(HERE, "mcp.log");

function log(message) {
  const line = `${new Date().toISOString()} ${message}\n`;
  // Mirror to stderr (for clients that do surface it) and to the log file.
  process.stderr.write(line);
  try {
    appendFileSync(LOG_FILE, line);
  } catch {
    // Never let logging break the server.
  }
}

// Spawn `mfb <args>` and capture its output. The CLI exits non-zero on bad
// input (unknown package/topic) and prints the reason to stderr; we surface that
// to the caller as an error result rather than throwing.
function runMfb(args) {
  // Command only, no result.
  log(`run: mfb ${args.join(" ")}`);
  return new Promise((resolvePromise) => {
    let child;
    try {
      child = spawn(MFB_BIN, args, {
        cwd: REPO_ROOT,
        // Force deterministic, terminal-free rendering for the spec output.
        env: { ...process.env, NO_COLOR: "1", COLUMNS: process.env.MFB_MCP_WIDTH || "100" },
      });
    } catch (err) {
      resolvePromise({ ok: false, output: `failed to launch \`${MFB_BIN}\`: ${err.message}` });
      return;
    }

    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", (err) => {
      resolvePromise({ ok: false, output: `failed to launch \`${MFB_BIN}\`: ${err.message}` });
    });
    child.on("close", (code) => {
      if (code === 0) {
        resolvePromise({ ok: true, output: stdout.length > 0 ? stdout : stderr });
      } else {
        const message = (stderr || stdout).trim();
        resolvePromise({
          ok: false,
          output: message.length > 0 ? message : `mfb exited with code ${code}`,
        });
      }
    });
  });
}

function textResult({ ok, output }) {
  return {
    isError: !ok,
    content: [{ type: "text", text: output.trimEnd() || "(no output)" }],
  };
}

const server = new McpServer({
  name: "mfbasic",
  version: "0.1.0",
});

server.tool(
  "mfb_man",
  "Show built-in MFBASIC package and function help. Omit both arguments for the package index; pass a package for its function list; pass a package and function for full details.",
  {
    package: z
      .string()
      .optional()
      .describe("Built-in package name, e.g. `io`, `strings`, `collections`. Omit to list all packages."),
    function: z
      .string()
      .optional()
      .describe("Function (or topic) within the package, e.g. `print`. Requires `package`."),
  },
  async ({ package: pkg, function: fn }) => {
    if (fn && !pkg) {
      return textResult({
        ok: false,
        output: "`function` requires `package` to also be set.",
      });
    }
    const args = ["man"];
    if (pkg) args.push(pkg);
    if (fn) args.push(fn);
    return textResult(await runMfb(args));
  },
);

server.tool(
  "mfb_spec",
  "Show the MFBASIC language specification. Omit arguments for the index; pass a topic for its section list; pass a topic and subtopic for one section; set `all` to render an entire topic.",
  {
    topic: z
      .string()
      .optional()
      .describe("Specification topic, e.g. `language`, `diagnostics`. Omit to list all topics."),
    subtopic: z
      .string()
      .optional()
      .describe("Section within the topic. Requires `topic`. Cannot combine with `all`."),
    all: z
      .boolean()
      .optional()
      .describe("Render the full text of the topic instead of its section list. Requires `topic`, excludes `subtopic`."),
  },
  async ({ topic, subtopic, all }) => {
    if (subtopic && !topic) {
      return textResult({ ok: false, output: "`subtopic` requires `topic` to also be set." });
    }
    if (all && !topic) {
      return textResult({ ok: false, output: "`all` requires `topic` to also be set." });
    }
    if (all && subtopic) {
      return textResult({ ok: false, output: "`all` cannot be combined with `subtopic`." });
    }
    const args = ["spec", "--no-color"];
    if (topic) args.push(topic);
    if (subtopic) args.push(subtopic);
    if (all) args.push("--all");
    return textResult(await runMfb(args));
  },
);

async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
  // log(`ready; logging to ${LOG_FILE}`);
  // Stays alive until the transport closes (stdin EOF / client disconnect).
}

main().catch((err) => {
  // The transport owns stdout; diagnostics must go to stderr.
  console.error(`mfb-mcp-server fatal: ${err?.stack || err}`);
  process.exit(1);
});
