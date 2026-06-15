#!/bin/bash

# Phase 1 Review Script
# Processes review_list.txt items using OpenAI Codex

REVIEW_LIST="specifications/review_list.txt"
REVIEW_MISSING="specifications/review_missing.txt"
STATUS_LOG="specifications/review_status.log"
MAX_CONCURRENT="${1:-5}"
MISSING_CLEAN_STREAK_TARGET="${2:-3}"

if ! [[ "$MAX_CONCURRENT" =~ ^[1-9][0-9]*$ ]]; then
    echo "Error: concurrency must be a positive integer"
    exit 1
fi

if ! [[ "$MISSING_CLEAN_STREAK_TARGET" =~ ^[1-9][0-9]*$ ]]; then
    echo "Error: phase 2 clean streak must be a positive integer"
    exit 1
fi

# Check if review_list exists
if [ ! -f "$REVIEW_LIST" ]; then
    echo "Error: $REVIEW_LIST not found"
    exit 1
fi

: > "$STATUS_LOG"
touch "$REVIEW_MISSING"

TMP_DIR=$(mktemp -d)
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

run_review() {
    local job_key="$1"
    local item_name="$2"
    local response_file="$TMP_DIR/response.$job_key"
    local exit_file="$TMP_DIR/exit.$job_key"

    # Build the prompt for this single item
    local prompt="Review ONLY this item: $item_name

DO NOT scan repository. DO NOT review multiple items.
Focus ONLY on $item_name.

Check for inconsistencies between:
1. Manual page for $item_name
2. specifications/*
3. src/**
4. tests/**

REPORT:
- Each inconsistency found
- How to resolve it
- Untested methods/edge cases

If NO inconsistencies found, respond with ONLY: \"No inconsistencies found\"

If inconsistencies found, format as:

FIXME
- Inconsistency 1: [description]
- Inconsistency 2: [description]
[blank line]
Notes: [your resolution notes]
[blank line]
Untested: [methods/edge cases not covered]"

    codex exec "$prompt" >"$response_file" 2>&1
    echo "$?" >"$exit_file"
}

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

process_completed_review() {
    local job_key="$1"
    local line_num="$2"
    local item_name="$3"
    local response_file="$TMP_DIR/response.$job_key"
    local exit_file="$TMP_DIR/exit.$job_key"
    local review_tmp="$TMP_DIR/review_list.$job_key"
    local exit_code
    local response
    local raw_response
    local normalized_response
    local fixme_body
    local deduped_fixme_body
    local man_file

    exit_code=$(cat "$exit_file")
    raw_response=$(cat "$response_file")
    response=$(extract_codex_response "$response_file")

    if [ "$exit_code" -ne 0 ]; then
        printf '%s [codex_exit]\n' "$item_name" >> "$STATUS_LOG"
        return 1
    fi

    normalized_response=$(printf '%s' "$response" | tr -d '\r' | sed -e '1{/^[[:space:]]*$/d;}' -e '${/^[[:space:]]*$/d;}')

    # Auto-add FIXME if inconsistencies found
    if [ "$normalized_response" != "No inconsistencies found" ]; then
        fixme_body=$(printf '%s\n' "$normalized_response" | awk '
            BEGIN { capture = 0 }
            /^[[:space:]]*FIXME[[:space:]]*$/ { capture = 1; next }
            /^[[:space:]]*tokens used[[:space:]]*$/ { capture = 0 }
            capture { print }
        ')

        if [ -z "$fixme_body" ]; then
            printf '%s [malformed_response]\n' "$item_name" >> "$STATUS_LOG"
            return 1
        fi

        deduped_fixme_body=$(printf '%s\n' "$fixme_body" | dedupe_exact_duplicate_halves)

        if [ -n "$deduped_fixme_body" ]; then
            fixme_body="$deduped_fixme_body"
        fi

        if [ -f "$item_name" ]; then
            man_file="$item_name"
        else
            man_file=$(find . -path "./$item_name" -type f 2>/dev/null | head -1)
        fi

        if [ -z "$man_file" ]; then
            man_file=$(find . -name "$(basename "$item_name" .txt).txt" -type f 2>/dev/null | head -1)
        fi

        if [ -f "$man_file" ]; then
            echo "" >> "$man_file"
            echo "FIXME" >> "$man_file"
            echo "$fixme_body" >> "$man_file"
        fi

        printf '%s [fixme]\n' "$item_name" >> "$STATUS_LOG"
    else
        printf '%s [clean]\n' "$item_name" >> "$STATUS_LOG"
    fi

    # Mark as [DONE]
    awk -v target="$line_num" '
        NR == target { sub(/\[TODO\]/, "[DONE]") }
        { print }
    ' "$REVIEW_LIST" > "$review_tmp" && mv "$review_tmp" "$REVIEW_LIST"
    rm -f "$response_file" "$exit_file"
}

render_progress() {
    local active_count="$1"
    local queued_count="$2"
    local line_count="${#AGENT_LINES[@]}"
    local header="Processing $active_count manual pages. $queued_count pages left to go:"
    local index

    if [ -n "$RENDERED_PROGRESS" ]; then
        printf '\033[%dA' "$((line_count + 1))"
    fi

    printf '\r\033[K%s\n' "$header"
    for index in "${!AGENT_LINES[@]}"; do
        printf '\r\033[KAgent %d: %s\n' "$((index + 1))" "${AGENT_LINES[$index]}"
    done

    RENDERED_PROGRESS=1
}

run_missing_review() {
    local job_key="$1"
    local response_file="$TMP_DIR/response.$job_key"
    local exit_file="$TMP_DIR/exit.$job_key"
    local prompt="Read all specifications/*
Review all src/**

REPORT:
- Any specification requirements not implemented in compiler not already in specifications/review_missing.txt
- How to resolve it

If NO missing requirements found, respond with ONLY: \"No missing requirements found\"

If missing requirements found, format as:

REQUIREMENT: [title]
SPECIFICATION: [specification file] @ [specification section]
[description]
[blank line]"

    codex exec "$prompt" >"$response_file" 2>&1
    echo "$?" >"$exit_file"
}

render_phase2_progress() {
    local found_count="$1"
    local clean_streak="$2"

    if [ -n "$PHASE2_RENDERED" ]; then
        printf '\033[2A'
    fi

    printf '\r\033[KPhase 2: missing requirements scan.\n'
    printf '\r\033[KFound: %d\n' "$found_count"
    printf '\r\033[KClean streak: %d/%d' "$clean_streak" "$MISSING_CLEAN_STREAK_TARGET"
    PHASE2_RENDERED=1
}

process_missing_review() {
    local job_key="$1"
    local response_file="$TMP_DIR/response.$job_key"
    local exit_file="$TMP_DIR/exit.$job_key"
    local exit_code
    local response
    local normalized_response
    local missing_body
    local deduped_missing_body
    local existing_contents
    local appended_count

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

    deduped_missing_body=$(printf '%s\n' "$missing_body" | dedupe_exact_duplicate_halves)
    if [ -n "$deduped_missing_body" ]; then
        missing_body="$deduped_missing_body"
    fi

    appended_count=$(printf '%s\n' "$missing_body" | awk '
        /^REQUIREMENT:/ { count++ }
        END { print count + 0 }
    ')

    existing_contents=$(cat "$REVIEW_MISSING")
    if [[ "$existing_contents" != *"$missing_body"* ]]; then
        if [ -s "$REVIEW_MISSING" ]; then
            printf '\n\n' >> "$REVIEW_MISSING"
        fi
        printf '%s\n' "$missing_body" >> "$REVIEW_MISSING"
        PHASE2_FOUND_COUNT=$((PHASE2_FOUND_COUNT + appended_count))
    fi

    rm -f "$response_file" "$exit_file"
    return 3
}

RENDERED_PROGRESS=
TOTAL_TODOS=0
TODO_LINES=()

while IFS= read -r todo_line; do
    TODO_LINES+=("$todo_line")
done < <(grep -n "\[TODO\]" "$REVIEW_LIST")

TOTAL_TODOS="${#TODO_LINES[@]}"

if [ "$TOTAL_TODOS" -eq 0 ]; then
    echo "✓ Phase 1 complete: All items marked [DONE]"
else
    SLOT_COUNT="$MAX_CONCURRENT"
    if [ "$TOTAL_TODOS" -lt "$SLOT_COUNT" ]; then
        SLOT_COUNT="$TOTAL_TODOS"
    fi

    PIDS=()
    JOB_KEYS=()
    LINE_NUMS=()
    ITEM_NAMES=()
    AGENT_LINES=()
    RUNNING=()
    CLEANED_UP=
    STOP_REQUESTED=0
    NEXT_TODO_INDEX=0
    ACTIVE_COUNT=0
    COMPLETED_COUNT=0
    QUEUED_COUNT="$TOTAL_TODOS"

    launch_review_for_slot() {
        local slot="$1"
        local todo_line
        local line_num
        local item_name
        local job_key

        if [ "$STOP_REQUESTED" -eq 1 ]; then
            RUNNING[$slot]=0
            AGENT_LINES[$slot]="[stopped]"
            return
        fi

        if [ "$NEXT_TODO_INDEX" -ge "$TOTAL_TODOS" ]; then
            RUNNING[$slot]=0
            AGENT_LINES[$slot]="[idle]"
            return
        fi

        todo_line="${TODO_LINES[$NEXT_TODO_INDEX]}"
        line_num=$(echo "$todo_line" | cut -d: -f1)
        item_name=$(echo "$todo_line" | cut -d: -f2- | sed 's/\[TODO\]//' | xargs)
        job_key="slot${slot}_todo${NEXT_TODO_INDEX}"

        LINE_NUMS[$slot]="$line_num"
        ITEM_NAMES[$slot]="$item_name"
        JOB_KEYS[$slot]="$job_key"
        AGENT_LINES[$slot]="$item_name"
        RUNNING[$slot]=1
        NEXT_TODO_INDEX=$((NEXT_TODO_INDEX + 1))
        ACTIVE_COUNT=$((ACTIVE_COUNT + 1))
        QUEUED_COUNT=$((TOTAL_TODOS - NEXT_TODO_INDEX))

        run_review "$job_key" "$item_name" &
        PIDS[$slot]=$!
    }

    for ((slot = 0; slot < SLOT_COUNT; slot++)); do
        launch_review_for_slot "$slot"
    done

    render_progress "$ACTIVE_COUNT" "$QUEUED_COUNT"

    while [ "$COMPLETED_COUNT" -lt "$TOTAL_TODOS" ]; do
        if [ "$STOP_REQUESTED" -eq 1 ]; then
            stop_running_reviews
            break
        fi

        for ((slot = 0; slot < SLOT_COUNT; slot++)); do
            if [ "$STOP_REQUESTED" -eq 1 ]; then
                stop_running_reviews
                break 2
            fi

            if [ "${RUNNING[$slot]}" != "1" ]; then
                continue
            fi

            if [ -f "$TMP_DIR/exit.${JOB_KEYS[$slot]}" ]; then
                wait "${PIDS[$slot]}"

                if ! process_completed_review "${JOB_KEYS[$slot]}" "${LINE_NUMS[$slot]}" "${ITEM_NAMES[$slot]}"; then
                    AGENT_LINES[$slot]="${ITEM_NAMES[$slot]} [failed]"
                else
                    AGENT_LINES[$slot]="${ITEM_NAMES[$slot]} [done]"
                fi

                COMPLETED_COUNT=$((COMPLETED_COUNT + 1))
                ACTIVE_COUNT=$((ACTIVE_COUNT - 1))
                RUNNING[$slot]=0

                if [ "$NEXT_TODO_INDEX" -lt "$TOTAL_TODOS" ]; then
                    launch_review_for_slot "$slot"
                fi

                render_progress "$ACTIVE_COUNT" "$QUEUED_COUNT"
            fi
        done

        if [ "$COMPLETED_COUNT" -lt "$TOTAL_TODOS" ]; then
            sleep 0.2
        fi
    done

    if [ "$STOP_REQUESTED" -eq 1 ]; then
        exit 130
    fi

    echo
    echo "✓ Phase 1 complete: All items marked [DONE]"
fi

if [ "$STOP_REQUESTED" -eq 1 ]; then
    exit 130
fi

PHASE2_CLEAN_STREAK=0
PHASE2_ATTEMPT=0
PHASE2_FOUND_COUNT=0
PHASE2_RENDERED=
PIDS=()

while [ "$PHASE2_CLEAN_STREAK" -lt "$MISSING_CLEAN_STREAK_TARGET" ]; do
    if [ "$STOP_REQUESTED" -eq 1 ]; then
        stop_running_reviews
        exit 130
    fi

    PHASE2_ATTEMPT=$((PHASE2_ATTEMPT + 1))
    PIDS=()
    run_missing_review "phase2_$PHASE2_ATTEMPT" &
    PIDS[0]=$!

    render_phase2_progress "$PHASE2_FOUND_COUNT" "$PHASE2_CLEAN_STREAK"

    while [ ! -f "$TMP_DIR/exit.phase2_$PHASE2_ATTEMPT" ]; do
        if [ "$STOP_REQUESTED" -eq 1 ]; then
            stop_running_reviews
            exit 130
        fi
        sleep 0.2
    done

    wait "${PIDS[0]}"
    process_missing_review "phase2_$PHASE2_ATTEMPT"
    phase2_result="$?"

    if [ "$phase2_result" -eq 0 ]; then
        PHASE2_CLEAN_STREAK=$((PHASE2_CLEAN_STREAK + 1))
    else
        PHASE2_CLEAN_STREAK=0
    fi

    render_phase2_progress "$PHASE2_FOUND_COUNT" "$PHASE2_CLEAN_STREAK"
done

echo
echo "✓ Phase 2 complete: $MISSING_CLEAN_STREAK_TARGET consecutive clean scans"
