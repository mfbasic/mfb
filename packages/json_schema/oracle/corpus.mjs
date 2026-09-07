// corpus.mjs — hand-written cases covering the whole supported surface.
//
// The package's own TESTING blocks (`mfb test packages/json_schema`) pin the
// behaviour this project decided on. These cases do the other half: every one
// is read by BOTH implementations, so a shared misreading of the specification
// has somewhere to show up. They are deliberately ordinary — the kind of schema
// a real document uses — because that is where a silent disagreement costs the
// most.
//
// A case may carry `divergence: "<key>"`, naming an entry in KNOWN_DIVERGENCES
// in oracle.mjs. That is a claim that the two implementations are SUPPOSED to
// disagree here, with the reason written down; anything else that disagrees is
// a failure.

export const CORPUS = [
  // --- boolean schemas -----------------------------------------------------
  { schema: true, instances: [null, 1, "x", [], {}] },
  { schema: false, instances: [null, 1, "x", [], {}] },
  { schema: {}, instances: [null, 1, {}] },

  // --- type ----------------------------------------------------------------
  { schema: { type: "string" }, instances: ["", "a", 1, null, [], {}, true] },
  { schema: { type: "number" }, instances: [1, 1.5, -0.0, 1e300, "1", true, null] },
  { schema: { type: "integer" }, instances: [1, 1.0, 1.5, -0, 2 ** 53, "1", true] },
  { schema: { type: "boolean" }, instances: [true, false, 0, 1, "true", null] },
  { schema: { type: "null" }, instances: [null, 0, "", false, [], {}] },
  { schema: { type: "array" }, instances: [[], [1], {}, "x"] },
  { schema: { type: "object" }, instances: [{}, { a: 1 }, [], null] },
  { schema: { type: ["string", "null"] }, instances: ["a", null, 1, []] },
  { schema: { type: ["integer", "boolean"] }, instances: [1, 1.5, true, "1"] },

  // --- const and enum ------------------------------------------------------
  { schema: { const: 1 }, instances: [1, 1.0, 2, "1", true, null] },
  { schema: { const: true }, instances: [true, 1, "true", false] },
  { schema: { const: null }, instances: [null, 0, false, ""] },
  { schema: { const: { a: [1, 2] } }, instances: [{ a: [1, 2] }, { a: [2, 1] }, { a: [1, 2], b: 1 }, [1, 2]] },
  { schema: { const: [1, { b: 2 }] }, instances: [[1, { b: 2 }], [1, { b: 3 }], [1]] },
  { schema: { enum: [1, "1", true, null, [1], { a: 1 }] }, instances: [1, "1", true, null, [1], { a: 1 }, 2, [2], { a: 2 }] },
  // The 2020-12 metaschema puts no minimum length on `enum`, so an empty one
  // is a schema that accepts nothing. ajv refuses to compile it.
  { schema: { enum: [] }, instances: [1, null], divergence: "ajv-enum-nonempty" },
  // 1 and 1.0 are one JSON value, so a `const` of either matches both.
  { schema: { const: 1.0 }, instances: [1, 1.0] },

  // --- numbers -------------------------------------------------------------
  // 1e300 is exactly 2 x 5e299 and every double that large is an even
  // integer, so it IS a multiple of 2. ajv reads the quotient with parseInt
  // and gets 5.
  { schema: { multipleOf: 2 }, instances: [0, 2, 3, -4, 2.5, 1e300, "2"], divergence: "ajv-multipleof-parseint" },
  { schema: { multipleOf: 0.5 }, instances: [1, 1.5, 1.25, 0] },
  { schema: { multipleOf: 0.0001 }, instances: [0.0075, 0.00751, 1] },
  { schema: { maximum: 10 }, instances: [9, 10, 11, 10.0001, -1e300] },
  { schema: { exclusiveMaximum: 10 }, instances: [9, 10, 11] },
  { schema: { minimum: -1.5 }, instances: [-2, -1.5, 0] },
  { schema: { exclusiveMinimum: -1.5 }, instances: [-1.5, -1.4999, -2] },
  { schema: { minimum: 0, maximum: 0 }, instances: [0, -0.0, 1] },
  { schema: { type: "integer", minimum: 1, maximum: 3 }, instances: [0, 1, 2, 3, 4, 2.5] },

  // --- strings -------------------------------------------------------------
  { schema: { minLength: 2 }, instances: ["", "a", "ab", "abc", 1] },
  { schema: { maxLength: 2 }, instances: ["", "ab", "abc"] },
  // Length is counted in code points, so one astral character is ONE.
  { schema: { maxLength: 1 }, instances: ["\u{1F600}", "ab", "e\u0301"] },
  { schema: { minLength: 1 }, instances: ["\u{1F600}", ""] },
  { schema: { pattern: "^a" }, instances: ["a", "ab", "ba", ""] },
  { schema: { pattern: "a" }, instances: ["cat", "dog"] },
  { schema: { pattern: "^[0-9]{3}-[0-9]{4}$" }, instances: ["555-1234", "5551234", "555-12345"] },
  { schema: { pattern: "\\d+" }, instances: ["12", "ab", "\u0660"] },
  { schema: { pattern: "\\w" }, instances: ["a", "_", "-", "\u00e9"] },
  { schema: { pattern: "\\s" }, instances: [" ", "\t", "\u00a0", "\u2028", "x"] },
  { schema: { pattern: "." }, instances: ["a", "\n", "\r", "\u2028", ""] },
  { schema: { pattern: "[]" }, instances: ["", "a"] },
  { schema: { pattern: "[^]" }, instances: ["", "a", "\n"] },
  { schema: { pattern: "a|b" }, instances: ["a", "b", "c"] },
  { schema: { pattern: "(ab)+" }, instances: ["abab", "ba"] },
  { schema: { pattern: "^\\u0041$" }, instances: ["A", "a"] },
  { schema: { pattern: "^\\u{1F600}$" }, instances: ["\u{1F600}", "x"] },

  // --- arrays --------------------------------------------------------------
  { schema: { minItems: 2 }, instances: [[], [1], [1, 2], "ab"] },
  { schema: { maxItems: 2 }, instances: [[], [1, 2], [1, 2, 3]] },
  { schema: { uniqueItems: true }, instances: [[], [1], [1, 2], [1, 1], [1, 1.0], [{ a: 1 }, { a: 1 }], [[1], [1]], [0, false], [null, null]] },
  { schema: { uniqueItems: false }, instances: [[1, 1]] },
  { schema: { items: { type: "integer" } }, instances: [[], [1], [1, "a"], "x"] },
  { schema: { prefixItems: [{ type: "integer" }, { type: "string" }] }, instances: [[], [1], [1, "a"], [1, 2], [1, "a", true]] },
  { schema: { prefixItems: [{ type: "integer" }], items: false }, instances: [[1], [1, 2], []] },
  { schema: { contains: { type: "integer" } }, instances: [[], ["a"], ["a", 1], [1, 2]] },
  { schema: { contains: { type: "integer" }, minContains: 2 }, instances: [[1], [1, 2], ["a"]] },
  { schema: { contains: { type: "integer" }, minContains: 0 }, instances: [[], ["a"]] },
  { schema: { contains: { type: "integer" }, maxContains: 1 }, instances: [[1], [1, 2], ["a"]] },
  { schema: { contains: { type: "integer" }, minContains: 0, maxContains: 0 }, instances: [[], [1]] },

  // --- objects -------------------------------------------------------------
  { schema: { required: ["a", "b"] }, instances: [{}, { a: 1 }, { a: 1, b: 2 }, { a: 1, b: 2, c: 3 }, []] },
  { schema: { minProperties: 1 }, instances: [{}, { a: 1 }] },
  { schema: { maxProperties: 1 }, instances: [{}, { a: 1 }, { a: 1, b: 2 }] },
  { schema: { properties: { a: { type: "integer" } } }, instances: [{}, { a: 1 }, { a: "x" }, { b: "x" }] },
  { schema: { properties: { a: { type: "integer" } }, additionalProperties: false }, instances: [{ a: 1 }, { a: 1, b: 2 }] },
  { schema: { patternProperties: { "^x": { type: "integer" } } }, instances: [{ xa: 1 }, { xa: "s" }, { ya: "s" }] },
  { schema: { patternProperties: { "^x": { type: "integer" } }, additionalProperties: false }, instances: [{ xa: 1 }, { ya: 1 }] },
  { schema: { propertyNames: { maxLength: 2 } }, instances: [{}, { ab: 1 }, { abc: 1 }] },
  { schema: { propertyNames: { pattern: "^[a-z]+$" } }, instances: [{ ab: 1 }, { A: 1 }] },
  { schema: { dependentRequired: { a: ["b"] } }, instances: [{}, { a: 1 }, { a: 1, b: 2 }, { b: 2 }] },
  { schema: { dependentSchemas: { a: { required: ["b"] } } }, instances: [{}, { a: 1 }, { a: 1, b: 2 }] },
  // An empty key and a key needing JSON Pointer escaping.
  { schema: { properties: { "": { type: "integer" }, "a/b": { type: "string" }, "c~d": { type: "boolean" } }, additionalProperties: false }, instances: [{ "": 1, "a/b": "s", "c~d": true }, { "": "no" }] },

  // --- applicators ---------------------------------------------------------
  { schema: { allOf: [{ type: "integer" }, { minimum: 2 }] }, instances: [1, 2, "x"] },
  { schema: { anyOf: [{ type: "integer" }, { type: "string" }] }, instances: [1, "x", true] },
  { schema: { oneOf: [{ multipleOf: 2 }, { multipleOf: 3 }] }, instances: [2, 3, 6, 5] },
  { schema: { not: { type: "integer" } }, instances: [1, "x"] },
  { schema: { not: {} }, instances: [1, null] },
  { schema: { if: { type: "integer" }, then: { minimum: 5 }, else: { type: "string" } }, instances: [7, 3, "x", true] },
  { schema: { if: { const: 1 }, then: false }, instances: [1, 2] },
  { schema: { if: { const: 1 }, else: false }, instances: [1, 2] },
  { schema: { then: { type: "string" } }, instances: [1] },
  { schema: { else: { type: "string" } }, instances: [1] },

  // --- unevaluated ---------------------------------------------------------
  { schema: { properties: { a: true }, unevaluatedProperties: false }, instances: [{ a: 1 }, { a: 1, b: 2 }] },
  { schema: { allOf: [{ properties: { a: true } }], unevaluatedProperties: false }, instances: [{ a: 1 }, { a: 1, b: 2 }] },
  { schema: { anyOf: [{ properties: { a: true } }, { properties: { b: true } }], unevaluatedProperties: false }, instances: [{ a: 1 }, { a: 1, b: 2 }, { c: 3 }] },
  { schema: { oneOf: [{ properties: { a: true }, required: ["a"] }, { properties: { b: true }, required: ["b"] }], unevaluatedProperties: false }, instances: [{ a: 1 }, { b: 1 }, { a: 1, b: 1 }, { c: 1 }] },
  { schema: { if: { properties: { a: true }, required: ["a"] }, then: { properties: { b: true } }, unevaluatedProperties: false }, instances: [{ a: 1, b: 2 }, { b: 2 }] },
  { schema: { not: { properties: { a: true } }, unevaluatedProperties: false }, instances: [{ a: 1 }, {}] },
  { schema: { prefixItems: [true], unevaluatedItems: false }, instances: [[1], [1, 2]] },
  { schema: { items: true, unevaluatedItems: false }, instances: [[1, 2, 3]] },
  // `contains` annotates the items it MATCHED, so an unmatched one is still
  // unevaluated. Checked against the official suite, which agrees with this;
  // ajv marks every item evaluated once `contains` passes.
  { schema: { contains: { type: "integer" }, unevaluatedItems: false }, instances: [[1], [1, "a"], ["a", 1]], divergence: "ajv-contains-annotation" },
  { schema: { allOf: [{ prefixItems: [true, true] }], unevaluatedItems: false }, instances: [[1, 2], [1, 2, 3]] },
  { schema: { unevaluatedProperties: { type: "integer" } }, instances: [{ a: 1 }, { a: "x" }] },
  { schema: { unevaluatedItems: { type: "integer" } }, instances: [[1], ["x"]] },
  // `unevaluatedProperties` does NOT see the parent's annotations from inside
  // an in-place applicator: the inner schema is evaluated on its own.
  { schema: { properties: { a: true }, allOf: [{ unevaluatedProperties: false }] }, instances: [{ a: 1 }] },

  // --- references ----------------------------------------------------------
  { schema: { $defs: { pos: { type: "integer", minimum: 1 } }, $ref: "#/$defs/pos" }, instances: [1, 0, "x"] },
  { schema: { $defs: { s: { type: "string" } }, properties: { a: { $ref: "#/$defs/s" } } }, instances: [{ a: "x" }, { a: 1 }] },
  { schema: { $defs: { node: { type: "object", properties: { child: { $ref: "#/$defs/node" } } } }, $ref: "#/$defs/node" }, instances: [{}, { child: {} }, { child: { child: {} } }, { child: 1 }] },
  { schema: { $id: "https://example.com/root", $defs: { a: { $anchor: "thing", type: "integer" } }, $ref: "#thing" }, instances: [1, "x"] },
  { schema: { $id: "https://example.com/root", $defs: { a: { $id: "child", type: "integer" } }, $ref: "child" }, instances: [1, "x"] },
  { schema: { $id: "https://example.com/a/b/root", $defs: { a: { $id: "../c/leaf", type: "string" } }, $ref: "https://example.com/a/c/leaf" }, instances: ["x", 1] },
  // A `$ref` with siblings: in 2020-12 the siblings still apply.
  { schema: { $defs: { s: { type: "string" } }, $ref: "#/$defs/s", minLength: 2 }, instances: ["ab", "a", 1] },
  // A pointer through an escaped key.
  { schema: { $defs: { "a/b": { type: "integer" } }, $ref: "#/$defs/a~1b" }, instances: [1, "x"] },
  { schema: { $defs: { "a~b": { type: "integer" } }, $ref: "#/$defs/a~0b" }, instances: [1, "x"] },
  // A pointer into an array-valued keyword.
  { schema: { allOf: [{ type: "integer" }], properties: { a: { $ref: "#/allOf/0" } } }, instances: [{ a: 1 }, { a: "x" }] },

  // A definition that refers to itself. Compiling one is fine -- it is only
  // ENTERING the loop that cannot terminate -- and both implementations agree
  // on both halves, the second by each refusing in its own way.
  { schema: { $defs: { t: { $ref: "#/$defs/t" } }, type: "integer" }, instances: [1, "x"] },
  { schema: { $defs: { t: { $ref: "#/$defs/t" } }, $ref: "#/$defs/t" }, instances: [1] },

  // --- annotations are ignored --------------------------------------------
  { schema: { title: "t", description: "d", default: 5, examples: [1], deprecated: true, readOnly: true, writeOnly: false, $comment: "c" }, instances: [1, "x"] },
  { schema: { format: "email" }, instances: ["not-an-email", "a@b.co", 1] },
  { schema: { format: "date-time" }, instances: ["nope", "2020-01-01T00:00:00Z"] },
  { schema: { contentEncoding: "base64", contentMediaType: "application/json" }, instances: ["!!!", "e30="] },

  // --- deep nesting --------------------------------------------------------
  { schema: { properties: { a: { properties: { b: { properties: { c: { type: "integer" } } } } } } }, instances: [{ a: { b: { c: 1 } } }, { a: { b: { c: "x" } } }] },
  { schema: { allOf: [{ anyOf: [{ oneOf: [{ not: { type: "string" } }] }] }] }, instances: [1, "x"] },
];

// Cases where the two implementations are SUPPOSED to disagree, each with the
// reason. Anything not listed here that disagrees is a failure.
export const DIVERGENT = [
  {
    key: "unsupported-dynamic",
    schema: { $defs: { a: { $dynamicAnchor: "T", type: "integer" } }, $dynamicRef: "#T" },
    instances: [1, "x"],
  },
  {
    key: "unsupported-dynamic",
    schema: { $recursiveAnchor: true, type: "integer" },
    instances: [1],
  },
  {
    key: "unsupported-regex",
    schema: { pattern: "a(?=b)" },
    instances: ["ab", "ac"],
  },
  {
    key: "unsupported-regex",
    schema: { pattern: "(a)\\1" },
    instances: ["aa", "ab"],
  },
  {
    key: "unsupported-regex",
    schema: { pattern: "\\p{L}" },
    instances: ["a", "1"],
  },
  {
    key: "regex-word-boundary",
    schema: { pattern: "\\bfoo" },
    instances: ["\u00e9foo"],
  },
  {
    key: "no-retrieval",
    schema: { $ref: "https://example.com/nowhere" },
    instances: [1],
  },
];
