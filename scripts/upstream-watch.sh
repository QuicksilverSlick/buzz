#!/usr/bin/env bash
# ============================================================================
# upstream-watch.sh -- what changed upstream, and does it collide with ours?
# ============================================================================
# Usage:
#   scripts/upstream-watch.sh              # report since the last run
#   scripts/upstream-watch.sh --since HEAD # report everything not in our main
#   scripts/upstream-watch.sh --mark       # record current upstream as "seen"
#
# Answers three questions, in the order they actually matter:
#
#   1. What is new upstream since we last looked?
#   2. Does any of it touch a file WE have customized?   <- the one that bites
#   3. Would merging conflict right now, and where?
#
# Question 2 exists because a clean `git merge` is not the same as a safe one.
# Upstream can rewrite a function our fork depends on, in a file we never
# touched, and git will merge it silently. The overlap list below is the set of
# files where our own commits and upstream's commits have both landed since the
# fork point -- the places where a merge can quietly undo our work. That is
# exactly how the buzz-acp author-gate fix got demoted from warn! to debug!
# during the 2026-09 merge: the merge was clean, the behaviour was not.
#
# Read-only. Fetches, never merges, never checks out, never writes to a branch.
# The only file it writes is the state file below.
#
# Exit codes:
#   0  nothing new
#   1  new upstream commits, no overlap with our customized files
#   2  new upstream commits AND overlap or predicted conflicts -- review needed
# ============================================================================

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT" || exit 1

UPSTREAM_REMOTE="${UPSTREAM_REMOTE:-upstream}"
UPSTREAM_BRANCH="${UPSTREAM_BRANCH:-main}"
OUR_BRANCH="${OUR_BRANCH:-main}"
UPSTREAM_REF="$UPSTREAM_REMOTE/$UPSTREAM_BRANCH"
STATE_FILE="${STATE_FILE:-$REPO_ROOT/.git/upstream-watch-state}"

MARK_ONLY=0
SINCE_OVERRIDE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --mark)  MARK_ONLY=1 ;;
        --since) SINCE_OVERRIDE="${2:-}"; shift ;;
        *) echo "unknown option: $1" >&2; exit 64 ;;
    esac
    shift
done

git remote get-url "$UPSTREAM_REMOTE" >/dev/null 2>&1 || {
    echo "ERROR: no '$UPSTREAM_REMOTE' remote. Add one with:" >&2
    echo "  git remote add $UPSTREAM_REMOTE https://github.com/block/buzz.git" >&2
    exit 64
}

git fetch "$UPSTREAM_REMOTE" "$UPSTREAM_BRANCH" --quiet 2>/dev/null || {
    echo "ERROR: fetch from $UPSTREAM_REMOTE failed (offline?)." >&2
    exit 69
}

UPSTREAM_HEAD="$(git rev-parse "$UPSTREAM_REF")"

if [ "$MARK_ONLY" = "1" ]; then
    echo "$UPSTREAM_HEAD" > "$STATE_FILE"
    echo "marked $UPSTREAM_REF @ ${UPSTREAM_HEAD:0:9} as seen"
    exit 0
fi

# Baseline: what we compare against. Prefer the explicit flag, then the last
# recorded run, then the merge base -- so a first run reports the full backlog
# rather than silently reporting nothing.
if [ -n "$SINCE_OVERRIDE" ]; then
    BASE="$(git rev-parse "$SINCE_OVERRIDE")"
    BASE_LABEL="$SINCE_OVERRIDE"
elif [ -f "$STATE_FILE" ] && BASE="$(git rev-parse --verify --quiet "$(cat "$STATE_FILE")^{commit}")"; then
    BASE_LABEL="last run"
else
    BASE="$(git merge-base "$OUR_BRANCH" "$UPSTREAM_REF")"
    BASE_LABEL="fork point (no previous run recorded)"
fi

NEW_COUNT="$(git rev-list --count "$BASE..$UPSTREAM_REF")"

echo "# Upstream watch — $(git log -1 --format=%cs "$UPSTREAM_REF")"
echo
echo "- upstream: \`$UPSTREAM_REF\` @ \`${UPSTREAM_HEAD:0:9}\`"
echo "- baseline: $BASE_LABEL @ \`${BASE:0:9}\`"
echo "- ours:     \`$OUR_BRANCH\` @ \`$(git rev-parse --short=9 "$OUR_BRANCH")\`"
echo

if [ "$NEW_COUNT" = "0" ]; then
    echo "**Nothing new upstream.**"
    exit 0
fi

# Commits we have that upstream does not -- our customizations.
FORK_POINT="$(git merge-base "$OUR_BRANCH" "$UPSTREAM_REF")"
OURS_FILES="$(mktemp)"; THEIRS_FILES="$(mktemp)"; OVERLAP="$(mktemp)"
trap 'rm -f "$OURS_FILES" "$THEIRS_FILES" "$OVERLAP"' EXIT

git diff --name-only "$FORK_POINT..$OUR_BRANCH" | sort -u > "$OURS_FILES"
git diff --name-only "$BASE..$UPSTREAM_REF"     | sort -u > "$THEIRS_FILES"
comm -12 "$OURS_FILES" "$THEIRS_FILES" > "$OVERLAP"
OVERLAP_COUNT="$(wc -l < "$OVERLAP" | tr -d ' ')"

echo "## $NEW_COUNT new upstream commit(s)"
echo
git log --format='- `%h` %s' "$BASE..$UPSTREAM_REF" | head -60
[ "$NEW_COUNT" -gt 60 ] && echo "- … and $((NEW_COUNT - 60)) more"
echo

# --- The part that matters: collisions with our own work -------------------
echo "## Collision check"
echo
echo "Files we changed since the fork point: $(wc -l < "$OURS_FILES" | tr -d ' ')"
echo "Files upstream changed in this range:  $(wc -l < "$THEIRS_FILES" | tr -d ' ')"
echo
if [ "$OVERLAP_COUNT" = "0" ]; then
    echo "**No overlap.** Upstream did not touch anything we have customized."
else
    echo "**$OVERLAP_COUNT file(s) touched by BOTH sides** — review these before merging;"
    echo "a clean auto-merge here can still undo our behaviour:"
    echo
    while IFS= read -r f; do
        ours=$(git log --oneline "$FORK_POINT..$OUR_BRANCH" -- "$f" | wc -l | tr -d ' ')
        theirs=$(git log --oneline "$BASE..$UPSTREAM_REF" -- "$f" | wc -l | tr -d ' ')
        echo "- \`$f\` — ours: $ours commit(s), upstream: $theirs commit(s)"
    done < "$OVERLAP"
fi
echo

# --- Would it conflict? Non-destructive: merge-tree writes no working tree --
echo "## Merge preview"
echo
MT_OUT="$(git merge-tree --write-tree --name-only "$OUR_BRANCH" "$UPSTREAM_REF" 2>&1)"
if [ $? -eq 0 ]; then
    echo "\`git merge $UPSTREAM_REF\` applies cleanly (no textual conflicts)."
else
    CONFLICTS="$(printf '%s\n' "$MT_OUT" | grep -E "^CONFLICT" || true)"
    if [ -n "$CONFLICTS" ]; then
        echo "**Textual conflicts predicted:**"
        echo
        printf '%s\n' "$CONFLICTS" | sed 's/^/- /'
    else
        echo "merge-tree reported a problem:"
        printf '%s\n' "$MT_OUT" | head -5 | sed 's/^/    /'
    fi
fi
echo

# --- Event-kind collisions: auto-merges cleanly, breaks semantically -------
# kind.rs merges without conflict even when both sides add the same number,
# which is a wire-protocol break no test would catch.
KIND_FILE="crates/buzz-core/src/kind.rs"
if git cat-file -e "$UPSTREAM_REF:$KIND_FILE" 2>/dev/null; then
    NEW_KINDS="$(git diff "$BASE..$UPSTREAM_REF" -- "$KIND_FILE" | grep -E "^\+pub const KIND" || true)"
    if [ -n "$NEW_KINDS" ]; then
        echo "## New event kinds upstream"
        echo
        echo "Check these against ours — a numeric collision auto-merges silently:"
        echo
        printf '%s\n' "$NEW_KINDS" | sed 's/^+/- `/;s/$/`/'
        echo
    fi
fi

echo "---"
echo
echo "Record this run as seen:  \`scripts/upstream-watch.sh --mark\`"

if [ "$OVERLAP_COUNT" != "0" ] || printf '%s\n' "$MT_OUT" | grep -q "^CONFLICT"; then
    exit 2
fi
exit 1
