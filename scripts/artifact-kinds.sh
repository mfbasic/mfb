# Shared codegen-dump artifact table for the two scripts that compare build
# artifacts against goldens:
#   - scripts/test-accept.sh   — the full acceptance harness
#   - scripts/artifact-gate.sh — the fast, execution-free codegen gate
# Sourced by both so they cannot drift about which deterministic build dumps
# exist or how each is produced.
#
# SCOPE — read before adding a kind. This table covers ONLY the artifacts that
# `mfb build -<flag>` emits as a deterministic dump WITHOUT linking, packaging,
# or running. That is the fast gate's entire contract, so anything requiring a
# further step must NOT go here:
#   - mfp / info          packaging (`mfb build` link + `mfb pkg info`)
#   - audit               `mfb audit`
#   - testrun             `mfb test`            (executes the program)
#   - covmap.json/covdata/covfail   `mfb test --coverage`   (executes)
# Those seven live only in test-accept.sh's own bespoke blocks; the fast gate
# never produces them, which is why it compares fewer kinds than the harness
# does — that is by design, not drift.
#
# Two families:
#   ARTIFACT_HOST_KINDS   — target-independent front-end dumps, built once for
#                           the host. Golden name: `<pkg>.<kind>`.
#   ARTIFACT_NATIVE_KINDS — per-target backend dumps. Golden name carries the
#                           target infix: `<pkg>.<target>.<kind>`, plus an
#                           app-mode (`mfb build -app`) variant
#                           `<pkg>.<target>.app.<kind>` for the app kinds.
ARTIFACT_HOST_KINDS="ast ir hex"
ARTIFACT_NATIVE_KINDS="nir nplan nobj ncode mir"
ARTIFACT_NATIVE_APP_KINDS="nir nplan ncode"

# The `mfb build` flag that emits a kind's dump. Every kind maps to `-<kind>`
# except hex, whose dump is the byte-render flag `-br`.
artifact_build_flag() {
  case "$1" in
    hex) printf -- '-br' ;;
    *)   printf -- '-%s' "$1" ;;
  esac
}

# ---------------------------------------------------------------------------
# Level-variance: which kinds change when the `-O` dial moves (bug-456).
#
# `scripts/test-accept.sh` accepts `MFB_OPT=<n>` (plan-100) to re-run the whole
# fixture suite at a chosen optimizer level. Goldens were all recorded at the
# default level, so a kind that the dial can rewrite mismatches on every such
# sweep — turning the sweep's real signal (a `.run`/build.log deviation) into
# noise. The predicate below is what a sweep uses to decide which goldens are
# still meaningful at a non-default level.
#
# The boundary is where the `-O`-gated passes sit in the pipeline:
# `build_nir_module` runs `optimizer::opt1::optimize_nir(module,
# active_opt_level())` (`src/target/shared/lower.rs:79`) and is the sole
# `NirModule` producer, so EVERY per-target dump — `.nir` and everything derived
# from it (`.nplan`, `.nobj`, `.ncode`, `.mir`) — is emitted downstream of the
# dial. The host kinds (`ARTIFACT_HOST_KINDS`: `.ast`, `.ir`, `.hex`) are
# produced before native lowering and are invariant, as are build.log, `.run`,
# `.testrun`, `.audit`, `.mfp`, `.info` and the coverage sidecars.
#
# So the predicate is exactly "is this a per-target native dump", NOT a list of
# the kinds observed to drift on today's fixtures. That distinction is
# load-bearing: measured at -O3 on 2026-09-06, only `.ncode`, `.mir`, and
# `macos-app-mode-term`'s `.app.nir`/`.app.nplan` actually differ — `.nobj`
# never does — but that is a property of which fixtures contain a loop for
# rotation to rewrite, not of the kinds. An earlier measurement (2026-08-31)
# saw `.nir`/`.nplan` hold still and concluded they were level-invariant; the
# fixture set changed and they moved. Keying the skip to observed drift would
# have to be re-derived every time a fixture gains a golden.
#
# `.ncodesum` is deliberately absent: it is the artifact-gate's kind, and
# `test-accept.sh` compares no `.ncodesum` on any path (the `tests/byte-identity`
# fixtures run, but nothing in the harness reads that extension). The gate has
# no `MFB_OPT` switch, so no harness compares an `.ncodesum` at a non-default
# level at all.
artifact_kind_is_level_variant() {
  case " $ARTIFACT_NATIVE_KINDS " in
    *" $1 "*) return 0 ;;
  esac
  return 1
}
