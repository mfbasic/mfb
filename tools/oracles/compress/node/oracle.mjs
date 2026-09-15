// Node judge for the compress oracle: built-in `node:zlib` only, no npm install.
//
// Usage: node oracle.mjs <mode> <job-path>
//        node oracle.mjs <encode-mode> <job-path> <mfb-output-path>
//
// Reads the job `python/gen.py` wrote and prints one `case <index> <fields...>` line
// per case, in the same shape the MFB probe prints. `zlib.crc32` needs Node >= 22.2
// (or 20.15).

import { readFileSync } from "node:fs";
import zlib from "node:zlib";

function* readJob(path) {
  const blob = readFileSync(path);
  const count = blob.readUInt32LE(0);
  let offset = 4 + 8 * count;
  for (let i = 0; i < count; i++) {
    const n = blob.readUInt32LE(4 + 8 * i);
    const aux = blob.readUInt32LE(8 + 8 * i);
    yield [i, blob.subarray(offset, offset + n), aux];
    offset += n;
  }
}

function outcome(fn) {
  try {
    return fn();
  } catch (e) {
    return `err ${e.code ?? e.name}: ${e.message}`;
  }
}

const MODES = {
  // How Node's zlib treats one probe stream (aux: 0 raw, 1 zlib, 2 gzip).
  probe(index, data, fmt) {
    const sync = [zlib.inflateRawSync, zlib.inflateSync, zlib.gunzipSync][fmt];
    const result = outcome(() => `ok out=${sync(data).length}`);
    return `case ${index} sync[${result}]`;
  },
  // Length and CRC-32 of Node zlib's raw DEFLATE decode, or the error.
  "decode-raw"(index, data) {
    return outcome(() => {
      const out = zlib.inflateRawSync(data);
      return `case ${index} ${out.length} ${zlib.crc32(out)}`;
    }).replace(/^err /, `case ${index} err `);
  },
  // Length and CRC-32 of Node zlib's zlib-format decode, or the error.
  "decode-zlib"(index, data) {
    return outcome(() => {
      const out = zlib.inflateSync(data);
      return `case ${index} ${out.length} ${zlib.crc32(out)}`;
    }).replace(/^err /, `case ${index} err `);
  },
  // Length and CRC-32 of Node zlib's gzip decode (every member), or the error.
  "decode-gzip"(index, data) {
    return outcome(() => {
      const out = zlib.gunzipSync(data);
      return `case ${index} ${out.length} ${zlib.crc32(out)}`;
    }).replace(/^err /, `case ${index} err `);
  },
  // Verdict only (aux: 0 raw, 1 zlib, 2 gzip) — messages are not comparable across decoders.
  mutate(index, data, fmt) {
    const sync = [zlib.inflateRawSync, zlib.inflateSync, zlib.gunzipSync][fmt];
    try {
      const out = sync(data);
      return `case ${index} ok ${out.length} ${zlib.crc32(out)}`;
    } catch {
      return `case ${index} err`;
    }
  },
  crc32(index, data, split) {
    const whole = zlib.crc32(data);
    const chained = zlib.crc32(data.subarray(split), zlib.crc32(data.subarray(0, split)));
    return `case ${index} ${whole} ${chained}`;
  },
};

// Encode modes judge the MFB probe's output file (a job of the same layout) against the payloads.
const ENCODE_MODES = {
  // Node's raw DEFLATE decode of what the probe produced must be exactly the payload.
  "encode-raw"(index, data, _level, produced) {
    try {
      const out = zlib.inflateRawSync(produced);
      if (!out.equals(data)) return `case ${index} MISMATCH ${out.length} ${zlib.crc32(out)}`;
      return `case ${index} ${data.length} ${zlib.crc32(data)}`;
    } catch (e) {
      return `case ${index} err ${e.code ?? e.name}: ${e.message}`;
    }
  },
  // Node's zlib-format decode of what the probe produced must be exactly the payload.
  "encode-zlib"(index, data, _level, produced) {
    try {
      const out = zlib.inflateSync(produced);
      if (!out.equals(data)) return `case ${index} MISMATCH ${out.length} ${zlib.crc32(out)}`;
      return `case ${index} ${data.length} ${zlib.crc32(data)}`;
    } catch (e) {
      return `case ${index} err ${e.code ?? e.name}: ${e.message}`;
    }
  },
  // Node's gzip decode of what the probe produced must be exactly the payload.
  "encode-gzip"(index, data, _level, produced) {
    try {
      const out = zlib.gunzipSync(produced);
      if (!out.equals(data)) return `case ${index} MISMATCH ${out.length} ${zlib.crc32(out)}`;
      return `case ${index} ${data.length} ${zlib.crc32(data)}`;
    } catch (e) {
      return `case ${index} err ${e.code ?? e.name}: ${e.message}`;
    }
  },
};

const [mode, path, producedPath] = process.argv.slice(2);
const lines = [];
if (ENCODE_MODES[mode] && path && producedPath) {
  const produced = [...readJob(producedPath)].map(([, data]) => data);
  for (const [index, data, aux] of readJob(path)) {
    lines.push(ENCODE_MODES[mode](index, data, aux, produced[index]));
  }
} else if (MODES[mode] && path) {
  for (const [index, data, aux] of readJob(path)) {
    lines.push(MODES[mode](index, data, aux));
  }
} else {
  console.error(`usage: node oracle.mjs <${Object.keys(MODES).join("|")}> <job-path>`);
  console.error(`       node oracle.mjs <${Object.keys(ENCODE_MODES).join("|")}> <job-path> <mfb-output-path>`);
  process.exit(2);
}
process.stdout.write(lines.join("\n") + (lines.length ? "\n" : ""));
