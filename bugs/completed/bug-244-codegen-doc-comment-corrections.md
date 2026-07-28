# bug-244: stale/duplicated doc comments in fma_fusion, module_analysis, and audio_specs

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: docs

Status: Fixed (2026-07-15) — fma_fusion.rs adds the commuted c + a*b -> fmadd_d row; module_analysis.rs drops the duplicated doc paragraphs on module_uses_migrated / module_drops_resource_union_close; audio_specs.rs reworded to reference the capabilities.runtime_calls pre-emit gate. Documentation only.

Three documentation corrections in the codegen/runtime layer:

- `src/target/shared/code/fma_fusion.rs:12` — module header says "The four sign
  combinations map to the neutral fused ops" but the table (`:15-19`) lists only
  three rows, omitting the commuted `c + a*b` (product-on-right add) case the code
  actually handles (`:145-147`); the intro block-diagram shows only `fmadd_d`.
  Fix: add the `c + a*b → fmadd_d` row (or reword "four sign combinations").
- `src/target/shared/code/module_analysis.rs:96-116` — the doc-comment paragraph
  for `module_uses_migrated` is duplicated verbatim (`:96-98` then `:100-103`), as
  is the one for `module_drops_resource_union_close` (`:109-111` then `:113-115`);
  both merge onto the same item. Fix: delete the first (orphaned) copy of each.
- `src/target/shared/runtime/audio_specs.rs:5-15` — the module header claims the
  spec rows make an `audio::` program "fail to build with the precise 'does not
  emit runtime helper' error until a backend lands," but because the specs carry
  full metadata `spec_for_symbol` succeeds, so that error cannot trigger; the real
  pre-backend gate is `capabilities.runtime_calls`. (Moot now that macOS/Linux
  audio backends have landed.) Fix: reword to reference the `runtime_calls` gate.
