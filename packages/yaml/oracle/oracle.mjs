#!/usr/bin/env node
// oracle.mjs — read a YAML file with `yaml` (eemeli/yaml) and report the result
// as one line of JSON, in exactly the shape `probe/` reports it.
//
//     node oracle.mjs <file.yaml>
//
//     {"ok":true,"documents":[ ... ]}
//     {"ok":false,"kind":"parse","reason":"..."}
//     {"ok":false,"kind":"unrepresentable","reason":"..."}
//
// Why THIS module: `yaml` implements YAML **1.2** with a selectable schema, so
// it can be pinned to the same language `packages/yaml` documents — the 1.2 Core
// Schema. That matters more than it sounds. The other easily available oracle,
// PyYAML, is YAML **1.1**, where `yes` is a boolean, `12:30` is 750, and `0o17`
// and `1e3` are strings; every one of those is a difference in the SPEC VERSION
// rather than in either implementation, so a 1.1 oracle buries the real signal
// under a list of expected-to-differ inputs. Pinned to 1.2 Core, this oracle
// agrees with `packages/yaml` on every input except the handful the package
// deliberately refuses — which is a comparison worth running.
//
// The options below are the whole of the pinning, and each one is load-bearing:
//
//   version: '1.2'   the language. Without it the module follows the document's
//                    own %YAML directive and defaults to 1.2 only for new docs.
//   schema: 'core'   the scalar typing rules. `core` is the schema the package
//                    names in its documentation; the module's default is
//                    'core' for 1.2 already, but naming it means a future
//                    default change cannot silently re-type every scalar.
//   uniqueKeys: true a duplicate mapping key is an error, as the package treats
//                    it. Left at the default this is also true, but this is a
//                    documented decision on the package side, so it is pinned.
//   merge: false     `<<` is NOT a merge key. Merge is a 1.1 extension; the
//                    package rejects `<<` outright, so the oracle must at least
//                    not silently merge.
//   mapAsMap: true   mapping keys arrive as real values instead of being
//                    stringified into an object. This is what lets the walk
//                    below SEE a non-string key (`12: x`) and report it, rather
//                    than silently agreeing on `{"12":"x"}` for a document the
//                    package refuses.

import { readFileSync } from 'node:fs'
import { parseAllDocuments } from 'yaml'

export const OPTIONS = {
  version: '1.2',
  schema: 'core',
  uniqueKeys: true,
  merge: false,
  mapAsMap: true,
  prettyErrors: false,
}

/** A value the oracle read but JSON cannot hold, with the reason. */
class Unrepresentable extends Error {}

/**
 * Convert one `toJS()` result into a JSON-safe value, refusing the three things
 * JSON cannot carry. Each refusal is reported rather than coerced, because
 * every one of them is a case `packages/yaml` also refuses — comparing a
 * coerced `null` against a refusal would look like agreement.
 *
 * `seen` is the ancestor chain, so a cycle introduced by an alias is caught
 * where it closes instead of recursing forever.
 */
function toJsonSafe(value, seen = new Set()) {
  if (value === null || value === undefined) return null
  if (typeof value === 'boolean' || typeof value === 'string') return value
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) {
      throw new Unrepresentable(
        `the number ${value} has no JSON form (.inf/.nan)`,
      )
    }
    return value
  }
  if (typeof value === 'bigint') return Number(value)
  if (value instanceof Date) {
    // 1.2 Core has no timestamp type, so this should be unreachable; if a
    // future schema change makes it reachable, say so rather than guess.
    throw new Unrepresentable('a timestamp value has no 1.2 Core Schema form')
  }

  if (seen.has(value)) {
    throw new Unrepresentable(
      'the document contains a cycle, which no JSON tree can hold',
    )
  }
  const nested = new Set(seen).add(value)

  if (Array.isArray(value)) return value.map((item) => toJsonSafe(item, nested))

  if (value instanceof Map) {
    const object = {}
    for (const [key, entry] of value) {
      if (typeof key !== 'string') {
        throw new Unrepresentable(
          `mapping key ${JSON.stringify(String(key))} is a ${key === null ? 'null' : typeof key}, and a JSON object key must be a string`,
        )
      }
      object[key] = toJsonSafe(entry, nested)
    }
    return object
  }

  if (value instanceof Set) {
    throw new Unrepresentable('a `!!set` value has no JSON form')
  }

  throw new Unrepresentable(`a ${typeof value} value has no JSON form`)
}

/**
 * Read a whole YAML stream. Returns the same envelope shape the MFBASIC probe
 * prints, so the runner compares two objects rather than two formats.
 */
export function readYaml(source) {
  let documents
  try {
    documents = parseAllDocuments(source, OPTIONS)
  } catch (problem) {
    return { ok: false, kind: 'parse', reason: String(problem.message ?? problem) }
  }

  for (const document of documents) {
    if (document.errors.length > 0) {
      const first = document.errors[0]
      return { ok: false, kind: 'parse', reason: `${first.code}: ${first.message}` }
    }
  }

  const values = []
  for (const document of documents) {
    let value
    try {
      value = document.toJS(OPTIONS)
    } catch (problem) {
      // `toJS` throws on a self-referential alias graph before the walk below
      // ever sees it.
      return { ok: false, kind: 'unrepresentable', reason: String(problem.message ?? problem) }
    }
    try {
      values.push(toJsonSafe(value))
    } catch (problem) {
      if (problem instanceof Unrepresentable) {
        return { ok: false, kind: 'unrepresentable', reason: problem.message }
      }
      throw problem
    }
  }
  return { ok: true, documents: values }
}

function main(argv) {
  if (argv.length !== 1) {
    process.stderr.write('usage: node oracle.mjs <file.yaml>\n')
    return 2
  }
  const source = readFileSync(argv[0], 'utf8')
  process.stdout.write(JSON.stringify(readYaml(source)) + '\n')
  return 0
}

if (import.meta.url === `file://${process.argv[1]}`) {
  process.exit(main(process.argv.slice(2)))
}
