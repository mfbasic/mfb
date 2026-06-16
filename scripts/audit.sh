#!/bin/bash

REVIEW_issues="specifications/review_issues.txt"
MISSING_CLEAN_STREAK_TARGET="${1:-3}"

if ! [[ "$MISSING_CLEAN_STREAK_TARGET" =~ ^[1-9][0-9]*$ ]]; then
    echo "Error: clean streak must be a positive integer"
    exit 1
fi

touch "$REVIEW_issues"

TMP_DIR=$(mktemp -d)
STOP_REQUESTED=0
CLEANED_UP=

request_stop() {
    STOP_REQUESTED=1
}

stop_running_reviews() {
    local pid

    for pid in "${PIDS[@]}"; do
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
        fi
    done

    sleep 0.2

    for pid in "${PIDS[@]}"; do
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            kill -9 "$pid" 2>/dev/null || true
        fi
    done
}

cleanup() {
    local pid

    if [ -n "$CLEANED_UP" ]; then
        return
    fi

    CLEANED_UP=1

    stop_running_reviews

    for pid in "${PIDS[@]}"; do
        if [ -n "$pid" ]; then
            wait "$pid" 2>/dev/null || true
        fi
    done

    rm -rf "$TMP_DIR"
}

trap cleanup EXIT
trap request_stop INT TERM

extract_codex_response() {
    local response_file="$1"

    awk '
        BEGIN { last = 0 }
        { lines[NR] = $0 }
        tolower($0) ~ /codex/ { last = NR }
        END {
            if (last == 0) {
                for (i = 1; i <= NR; i++) {
                    print lines[i]
                }
            } else {
                for (i = last + 1; i <= NR; i++) {
                    print lines[i]
                }
            }
        }
    ' "$response_file"
}

dedupe_exact_duplicate_halves() {
    awk '
        {
            lines[++count] = $0
        }
        END {
            if (count % 2 == 0) {
                half = count / 2
                same = 1
                for (i = 1; i <= half; i++) {
                    if (lines[i] != lines[i + half]) {
                        same = 0
                        break
                    }
                }
                if (same) {
                    for (i = 1; i <= half; i++) {
                        print lines[i]
                    }
                    exit
                }
            }

            for (i = 1; i <= count; i++) {
                print lines[i]
            }
        }
    '
}

run_issues_review() {
    local job_key="$1"
    local response_file="$TMP_DIR/response.$job_key"
    local exit_file="$TMP_DIR/exit.$job_key"
    local prompt="Read all specifications/*
Review all src/**

REPORT:
- Any specification requirements not implemented in compiler not already in specifications/review_issues.txt
- How to resolve it

If NO missing requirements found, respond with ONLY: \"No missing requirements found\"

If missing requirements found, format as:

REQUIREMENT: [title]
SPECIFICATION: [specification file] @ [specification section]
GROUP: [group — pick exactly one from the list below]
RECOMMENDED: [SPEC CHANGE | CODE CHANGE — pick one based on the overall language feel: use SPEC CHANGE if the spec appears to be in error or conflicts with the language's design intent; use CODE CHANGE if the spec is clearly correct and the implementation is simply missing or wrong]
[description]
[blank line]

Groups (pick the most specific match):
  json        Built-in JSON package (standard_package.md §12)
  filesystem  Built-in filesystem package (standard_package.md §8)
  math        Built-in math package (standard_package.md §10)
  io          Built-in IO package (standard_package.md §7)
  regex       Built-in regex package (standard_package.md §6)
  net         Built-in net/TLS package (standard_package.md §11)
  stdlib      General standard library: toString, typeName, isNumeric, toFixed, collection helpers (standard_package.md §3)
  typesystem  Type system: primitives, records, unions, enums, Result, Error, type inference, comparable types, templates
  language    Language: lexer, grammar, parser, control flow, operators, scope, bindings, functions, closures, modules, imports, memory semantics, resources, USING, FAIL, TRAP, PROPAGATE
  threads     Threading: Thread type, thread lifecycle, queue semantics, sendability
  registry    Package registry, repository, account model, publishing, signing, lockfile
  tooling     CLI commands (build, init, pkg, fmt, test, lsp, audit, check-abi), project configuration, project manifest validation
  package     Package format (.mfp), bytecode sections, linker, ABI hashing, native helper dispatch, package bytecode lowering"

    codex exec "$prompt" >"$response_file" 2>&1
    echo "$?" >"$exit_file"
}

render_progress() {
    local found_count="$1"
    local clean_streak="$2"

    if [ -n "$RENDERED" ]; then
        printf '\033[2A'
    fi

    printf '\r\033[KMissing requirements scan.\n'
    printf '\r\033[KFound: %d\n' "$found_count"
    printf '\r\033[KClean streak: %d/%d' "$clean_streak" "$MISSING_CLEAN_STREAK_TARGET"
    RENDERED=1
}

add_to_section() {
    local group="$1"
    local block="$2"
    local file="$3"
    local header="=== $group ==="
    local title tmp blk_file
    title=$(printf '%s\n' "$block" | grep '^REQUIREMENT:' | head -1)
    tmp="${file}.tmp.$$"
    blk_file="${file}.blk.$$"

    printf '%s\n' "$block" > "$blk_file"

    # Skip if this requirement title is already in the file
    if [ -s "$file" ] && grep -qF "$title" "$file"; then
        rm -f "$blk_file"
        return 1
    fi

    if ! grep -qxF "$header" "$file" 2>/dev/null; then
        # Section doesn't exist yet — append it
        {
            [ -s "$file" ] && printf '\n\n'
            printf '%s\n\n' "$header"
            cat "$blk_file"
        } >> "$file"
        rm -f "$blk_file"
        return 0
    fi

    # Section exists — insert block at the end of that section.
    # Block is passed via a temp file to avoid awk's -v newline restriction.
    awk -v hdr="$header" -v blk_file="$blk_file" '
        { lines[NR] = $0 }
        END {
            sec_start = 0
            next_sec = NR + 1
            for (i = 1; i <= NR; i++) {
                if (lines[i] == hdr) sec_start = i
                else if (sec_start > 0 && lines[i] ~ /^=== .+ ===$/) {
                    next_sec = i
                    break
                }
            }
            last_content = sec_start
            for (i = sec_start + 1; i < next_sec; i++) {
                if (lines[i] !~ /^[[:space:]]*$/) last_content = i
            }
            for (i = 1; i <= last_content; i++) print lines[i]
            printf "\n"
            while ((getline line < blk_file) > 0) print line
            if (next_sec <= NR) printf "\n\n"
            for (i = next_sec; i <= NR; i++) print lines[i]
        }
    ' "$file" > "$tmp" && mv "$tmp" "$file"
    rm -f "$blk_file"
    return 0
}

trim_blank_lines() {
    awk 'NF { found=1; for (i=1;i<=b;i++) print ""; b=0; print; next } found { b++ }'
}

process_issues_review() {
    local job_key="$1"
    local response_file="$TMP_DIR/response.$job_key"
    local exit_file="$TMP_DIR/exit.$job_key"
    local exit_code response normalized_response missing_body deduped_issues_body

    exit_code=$(cat "$exit_file")
    response=$(extract_codex_response "$response_file")

    if [ "$exit_code" -ne 0 ]; then
        rm -f "$response_file" "$exit_file"
        return 1
    fi

    normalized_response=$(printf '%s' "$response" | tr -d '\r' | sed -e '1{/^[[:space:]]*$/d;}' -e '${/^[[:space:]]*$/d;}')

    if [ "$normalized_response" = "No missing requirements found" ]; then
        rm -f "$response_file" "$exit_file"
        return 0
    fi

    missing_body=$(printf '%s\n' "$normalized_response" | awk '
        BEGIN { capture = 0 }
        /^REQUIREMENT:/ { capture = 1 }
        /^[[:space:]]*tokens used[[:space:]]*$/ { capture = 0 }
        capture { print }
    ')

    if [ -z "$missing_body" ]; then
        rm -f "$response_file" "$exit_file"
        return 2
    fi

    deduped_issues_body=$(printf '%s\n' "$missing_body" | dedupe_exact_duplicate_halves)
    if [ -n "$deduped_issues_body" ]; then
        missing_body="$deduped_issues_body"
    fi

    local added=0

    # Split into individual blocks (null-delimited) and route each to its section
    while IFS= read -r -d '' raw_block; do
        local block group
        block=$(printf '%s\n' "$raw_block" | trim_blank_lines)
        [ -z "$block" ] && continue

        group=$(printf '%s\n' "$block" | awk '/^GROUP:/ { gsub(/^GROUP:[[:space:]]*/, ""); print; exit }')
        [ -z "$group" ] && group="language"

        if add_to_section "$group" "$block" "$REVIEW_issues"; then
            added=$((added + 1))
        fi
    done < <(printf '%s\n' "$missing_body" | awk '
        /^REQUIREMENT:/ && buf != "" { printf "%s%c", buf, 0; buf = "" }
        { buf = buf $0 "\n" }
        END { if (buf != "") printf "%s%c", buf, 0 }
    ')

    rm -f "$response_file" "$exit_file"

    if [ "$added" -gt 0 ]; then
        FOUND_COUNT=$((FOUND_COUNT + added))
        return 3
    fi
    return 0
}

CLEAN_STREAK=0
ATTEMPT=0
FOUND_COUNT=0
RENDERED=
PIDS=()

while [ "$CLEAN_STREAK" -lt "$MISSING_CLEAN_STREAK_TARGET" ]; do
    if [ "$STOP_REQUESTED" -eq 1 ]; then
        stop_running_reviews
        exit 130
    fi

    ATTEMPT=$((ATTEMPT + 1))
    PIDS=()
    run_issues_review "attempt_$ATTEMPT" &
    PIDS[0]=$!

    render_progress "$FOUND_COUNT" "$CLEAN_STREAK"

    while [ ! -f "$TMP_DIR/exit.attempt_$ATTEMPT" ]; do
        if [ "$STOP_REQUESTED" -eq 1 ]; then
            stop_running_reviews
            exit 130
        fi
        sleep 0.2
    done

    wait "${PIDS[0]}"
    process_issues_review "attempt_$ATTEMPT"
    result="$?"

    if [ "$result" -eq 0 ]; then
        CLEAN_STREAK=$((CLEAN_STREAK + 1))
    else
        CLEAN_STREAK=0
    fi

    render_progress "$FOUND_COUNT" "$CLEAN_STREAK"
done

echo
echo "✓ Done: $MISSING_CLEAN_STREAK_TARGET consecutive clean scans"
