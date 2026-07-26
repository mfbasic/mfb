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
