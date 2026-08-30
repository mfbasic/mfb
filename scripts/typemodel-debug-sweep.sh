#!/usr/bin/env bash
# plan-111-C Phase 2: compile every fixture project with the DEBUG binary, so
# the `#[cfg(debug_assertions)]` TypeModel key-bijection invariant actually
# fires. This is a COMPILE sweep — it diffs no goldens and regenerates nothing.
#
# `#[cfg(debug_assertions)]` guards do not fire in release, and the local gates
# are RELEASE, so a release run proves nothing about them (the CI-axis memory).
set -u
MFB="$1"
built=0
tripped=0
failed_other=0

while IFS= read -r manifest; do
  project="$(dirname "$manifest")"
  out=$("$MFB" build -q "$project" 2>&1)
  status=$?
  rm -rf "$project/build"
  # A package fixture writes its `.mfp` BESIDE its source, not into `build/`.
  # Left behind, those look like untracked work in `git status` and are easy to
  # mistake for a peer session's files.
  find "$project" -maxdepth 1 -name '*.mfp' -newer "$manifest" -delete 2>/dev/null
  # Only a real panic counts. NOT a text match on "assert": a diagnostic says
  # "a bare binding asserts the resource has no state", and a fixture path can
  # contain the word too — both matched a looser grep and cost a false alarm.
  if printf '%s' "$out" | grep -qE 'panicked at|assertion .* failed'; then
    echo "ASSERT   $project"
    printf '%s\n' "$out" | grep -A3 'panicked' | head -6 | sed 's/^/    /'
    tripped=$((tripped + 1))
  elif [ $status -ne 0 ]; then
    # A fixture that is SUPPOSED to fail to compile still built its TypeModel
    # for every module it got that far on; only a panic is a failure here.
    failed_other=$((failed_other + 1))
  fi
  built=$((built + 1))
done < <(find tests -name project.json | sort)

echo "debug-sweep: $built project(s) compiled with the debug binary — \
$tripped assertion trip(s), $failed_other expected-reject build(s)"
exit $tripped
