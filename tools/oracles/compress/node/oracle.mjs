// Node judge for the compress oracle: built-in `node:zlib` only, no npm install.
//
// Usage: node oracle.mjs <mode> <job-path>
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

const MODES = {
  crc32(index, data, split) {
    const whole = zlib.crc32(data);
    const chained = zlib.crc32(data.subarray(split), zlib.crc32(data.subarray(0, split)));
    return `case ${index} ${whole} ${chained}`;
  },
};

const [mode, path] = process.argv.slice(2);
if (!MODES[mode] || !path) {
  console.error(`usage: node oracle.mjs <${Object.keys(MODES).join("|")}> <job-path>`);
  process.exit(2);
}
const lines = [];
for (const [index, data, aux] of readJob(path)) {
  lines.push(MODES[mode](index, data, aux));
}
process.stdout.write(lines.join("\n") + (lines.length ? "\n" : ""));
