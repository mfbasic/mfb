// temporal.mjs — the syntax cross-check for `ixdtf` mode, using the JavaScript
// Temporal API (node --harmony-temporal).
//
// Temporal's zone rules come from the ICU tz data bundled with Node, which is an
// OLDER release than the package's (Node 24.12 ships tz 2025b; the package is
// 2026d). So Temporal is only trusted on what the text says, not on offsets the
// two releases disagree about.
//
//   node --harmony-temporal temporal.mjs filter jobs/ixdtf.candidates
//       Reads `candidate <name> <seconds> <utoff> <text>` lines, where utoff is
//       zoneinfo's (2026d) offset. Keeps a candidate only if Temporal gives the
//       same offset for that zone at that instant, and writes it as an
//       `ixdtf <text>` job on stdout. Every skipped candidate goes to stderr
//       with its reason.
//
//   node --harmony-temporal temporal.mjs answer jobs/ixdtf.txt
//       Answers each `ixdtf <text>` job with
//       `accept <epochSeconds> <nanos> <offsetSeconds> <zone>` or `reject`.

import { readFileSync } from "node:fs";

if (typeof Temporal === "undefined") {
  console.error("temporal.mjs: Temporal is unavailable; run with node --harmony-temporal (Node >= 24)");
  process.exit(2);
}

const [mode, path] = process.argv.slice(2);
const lines = readFileSync(path, "utf8").split("\n").filter((line) => line !== "");
const out = [];

function zoneOffsetSeconds(name, seconds) {
  const instant = Temporal.Instant.fromEpochMilliseconds(seconds * 1000);
  return Number(instant.toZonedDateTimeISO(name).offsetNanoseconds / 1e9);
}

if (mode === "filter") {
  const seen = new Set();
  for (const line of lines) {
    const [, name, seconds, utoff, text] = line.split(" ");
    let temporalOffset;
    try {
      temporalOffset = zoneOffsetSeconds(name, Number(seconds));
    } catch (error) {
      console.error(`skip ${text}: Temporal does not know zone ${name} (${error.message})`);
      continue;
    }
    if (temporalOffset !== Number(utoff)) {
      console.error(`skip ${text}: Temporal's tz gives ${temporalOffset} for ${name} at ${seconds}, 2026d gives ${utoff}`);
      continue;
    }
    const job = "ixdtf " + text;
    if (!seen.has(job)) {
      seen.add(job);
      out.push(job);
    }
  }
} else if (mode === "answer") {
  for (const line of lines) {
    const text = line.slice("ixdtf ".length);
    try {
      const z = Temporal.ZonedDateTime.from(text);
      let seconds = z.epochNanoseconds / 1000000000n;
      let nanos = z.epochNanoseconds % 1000000000n;
      if (nanos < 0n) {
        seconds -= 1n;
        nanos += 1000000000n;
      }
      const zone = z.timeZoneId ?? String(z.timeZone);
      out.push(`accept ${seconds} ${nanos} ${Number(z.offsetNanoseconds / 1e9)} ${zone}`);
    } catch {
      out.push("reject");
    }
  }
} else {
  console.error("usage: temporal.mjs filter|answer <file>");
  process.exit(2);
}

process.stdout.write(out.join("\n") + (out.length ? "\n" : ""));
