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

import { readJob as nodeReadJob } from "./oracle.mjs";

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

const MODES = { corpus: runCorpus };

function main() {
  const requested = process.argv.slice(2).filter((argument) => !argument.startsWith("--"));
  const modes = requested.length > 0 ? requested : Object.keys(MODES);

  let failed = false;
  for (const mode of modes) {
    const run = MODES[mode];
    if (!run) {
      console.error(`unknown mode \`${mode}\` (have: ${Object.keys(MODES).join(", ")})`);
      process.exit(2);
    }
    const { count, failures, diverged } = run();
    const declared = diverged > 0 ? `, ${diverged} declared divergent` : "";
    if (failures.length === 0) {
      console.log(`ok   ${mode}: ${count} case(s) agreed three ways${declared}`);
      continue;
    }
    failed = true;
    console.log(`FAIL ${mode}: ${failures.length} of ${count} case(s) disagreed${declared}`);
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
