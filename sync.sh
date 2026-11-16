#!/usr/bin/env bash
# Candidate-only maintenance. Acceptance and publication belong to the maintainer.
set -euo pipefail
export LC_ALL=C
die() { printf '%s\n' "$*" >&2; exit 1; }
g() {
    git -c core.hooksPath=/dev/null -c rebase.updateRefs=false \
        -c rerere.enabled=false -c commit.gpgsign=false "$@"
}
clean() {
    test -z "$(g -C "$1" status --porcelain=v1 --untracked-files=all)" || die "dirty worktree: $1"
    for operation in rebase-merge rebase-apply MERGE_HEAD CHERRY_PICK_HEAD REVERT_HEAD BISECT_START; do
        test ! -e "$(g -C "$1" rev-parse --path-format=absolute --git-path "$operation")" || die "Git operation in progress"
    done
}
sha() { g rev-parse --verify "$1^{commit}"; }
hash() { sha256sum "$1" | cut -d ' ' -f 1; }
save() { printf '%s\n' "$2" > "$run/$1"; }
read_value() { cat "$run/$1"; }
record() { printf '%s %s\n' "$(date -u +%FT%TZ)" "$*" >> "$run/record.txt"; }
guard() {
    test -s "$run/candidate-tree" || die 'candidate preparation did not complete'
    test "$(g -C "$caller" rev-parse HEAD)" = "$(read_value caller-head)" || die 'caller HEAD changed'
    clean "$caller"
    test "$(g -C "$caller" rev-parse "$(read_value accepted)^{commit}")" = "$(read_value accepted-sha)" || die 'accepted ref changed'
    test "$(hash "$adapter")" = "$(read_value adapter-sha256)" || die 'check adapter changed'
    test "$(hash "$(realpath "$0")")" = "$(read_value workflow-sha256)" || die 'workflow changed'
    test "$(g -C "$candidate" rev-parse HEAD)" = "$(read_value candidate-sha)" || die 'candidate changed'
    test "$(g -C "$candidate" rev-parse 'HEAD^{tree}')" = "$(read_value candidate-tree)" || die 'candidate tree changed'
    clean "$candidate"
}
cleanup() {
    # No force: unexpected candidate changes need human inspection.
    clean "$candidate"
    g -C "$caller" worktree remove "$candidate"
    g -C "$caller" update-ref -d "$(read_value candidate-ref)" "$(read_value candidate-sha)"
}

test -z "${GIT_DIR:-}${GIT_WORK_TREE:-}${GIT_INDEX_FILE:-}${GIT_COMMON_DIR:-}" || die 'unset Git directory/index overrides'
command=${1:-}
shift || true
if test "$command" = prepare; then
    test "$#" = 6 || die 'prepare ACCEPTED OLD_BASE NEW_BASE FORK_REMOTE RUN_DIR CHECK_ADAPTER'
    accepted=$1; old=$(sha "$2"); upstream=$(sha "$3"); fork=$4
    caller=$(g rev-parse --show-toplevel)
    clean "$caller"
    options=$(g rebase -h 2>&1 || true)
    case "$options" in *'drop|keep|stop'*) ;; *) die 'Git must support rebase --empty=stop';; esac
    accepted_sha=$(sha "$accepted")
    test "$(sha HEAD)" = "$accepted_sha" || die 'run from the accepted stack'
    g merge-base --is-ancestor "$old" "$accepted_sha" || die 'unrelated accepted base'
    g merge-base --is-ancestor "$old" "$upstream" || die 'unrelated upstream target'
    test -z "$(g rev-list --merges "$old..$accepted_sha")" || die 'patch stack contains merges'
    adapter=$(realpath -e "$6")
    test -x "$adapter" || die 'check adapter must be executable'
    run=$(realpath -m "$5")
    case "$run/" in "$caller/"*) die 'run directory must be outside caller';; esac
    for value in "$accepted" "$fork" "$caller" "$run" "$adapter"; do
        test -n "$value" && [[ "$value" != *$'\n'* ]] || die 'empty or multiline input'
    done
    mkdir "$run" || die 'run directory must be new (parent must exist)'
    mkdir "$run/lock"
    trap 'rmdir "$run/lock"' EXIT
    candidate="$run/candidate"
    candidate_ref="refs/eon-sync/candidates/$(date -u +%Y%m%dT%H%M%SZ)-$$"
    save caller "$caller"; save caller-head "$accepted_sha"
    save accepted "$accepted"; save accepted-sha "$accepted_sha"
    save old-base "$old"; save upstream "$upstream"; save fork-remote "$fork"
    save adapter "$adapter"; save adapter-sha256 "$(hash "$adapter")"
    save workflow-sha256 "$(hash "$(realpath "$0")")"
    save candidate-ref "$candidate_ref"; save candidate-sha "$accepted_sha"
    g log --reverse --format=%s "$old..$accepted_sha" > "$run/patch-subjects.txt"
    g show-ref > "$run/refs-before.txt"
    record "prepare accepted=$accepted_sha old=$old upstream=$upstream fork=$fork"
    g update-ref "$candidate_ref" "$accepted_sha" 0000000000000000000000000000000000000000
    g worktree add --detach "$candidate" "$candidate_ref" >> "$run/rebase.log" 2>&1
    if ! g -C "$candidate" -c rebase.autoStash=false -c rebase.autoSquash=false \
        rebase --reapply-cherry-picks --empty=stop --keep-empty --no-update-refs \
        --no-autostash --no-autosquash --no-gpg-sign --no-rerere-autoupdate \
        --onto "$upstream" "$old" >> "$run/rebase.log" 2>&1; then
        record 'FAIL rebase; inspect rebase.log; no proposal'
        g -C "$candidate" status --porcelain=v1 > "$run/conflict.txt"
        if test -d "$(g -C "$candidate" rev-parse --path-format=absolute --git-path rebase-merge)" || \
            test -d "$(g -C "$candidate" rev-parse --path-format=absolute --git-path rebase-apply)"; then
            g -C "$candidate" rebase --abort
        fi
        cleanup
        exit 1
    fi
    rebased=$(g -C "$candidate" rev-parse HEAD)
    g -C "$candidate" log --reverse --format=%s "$upstream..$rebased" > "$run/rebased-subjects.txt"
    if ! cmp -s "$run/patch-subjects.txt" "$run/rebased-subjects.txt"; then
        record 'FAIL patch count/subjects changed; patch removal requires evidence'
        # Update only our ref so cleanup can compare its exact expected identity.
        g update-ref "$candidate_ref" "$rebased" "$accepted_sha"
        save candidate-sha "$rebased"
        cleanup
        exit 1
    fi
    g update-ref "$candidate_ref" "$rebased" "$accepted_sha"
    save candidate-sha "$rebased"
    save candidate-tree "$(g -C "$candidate" rev-parse 'HEAD^{tree}')"
    g range-diff "$old..$accepted_sha" "$upstream..$rebased" > "$run/range-diff.txt"
    guard
    record "PREPARED candidate=$rebased tree=$(read_value candidate-tree)"
    printf 'PREPARED %s\n' "$run"
    exit
fi

test "$#" -ge 1 || die 'check RUN_DIR focused|native|motion | retry FAILED_RUN NEW_RUN | propose RUN_DIR | discard RUN_DIR'
run=$(realpath -e "$1"); shift
mkdir "$run/lock" || die 'run is busy'
trap 'rmdir "$run/lock"' EXIT
caller=$(read_value caller); candidate="$run/candidate"; adapter=$(read_value adapter)
guard
case "$command" in
    retry)
        test "$#" = 1 || die 'retry FAILED_RUN NEW_RUN (requires maintainer authorization)'
        test -f "$run/focused.pass" && test -d "$run/native" && test ! -e "$run/native.pass" || die 'retry requires a failed native phase'
        test ! -e "$run/motion" && test ! -e "$run/retry-of" || die 'only one untimed retry is allowed'
        next=$(realpath -m "$1")
        case "$next/" in "$caller/"*|"$candidate/"*) die 'retry directory must be outside existing worktrees';; esac
        [[ "$next" != *$'\n'* ]] || die 'multiline run directory'
        test ! -e "$next" || die 'retry directory must be new'
        # Claim the single authorized retry before creating any new candidate state.
        mkdir "$run/retry-used" || die 'retry already used'
        mkdir "$next"
        mkdir "$next/lock"
        prior=$run
        trap 'rmdir "$prior/lock" "$next/lock"' EXIT
        for item in caller caller-head accepted accepted-sha old-base upstream fork-remote adapter adapter-sha256 workflow-sha256 candidate-sha candidate-tree patch-subjects.txt rebased-subjects.txt range-diff.txt; do
            cp "$run/$item" "$next/$item"
        done
        ref="refs/eon-sync/candidates/$(date -u +%Y%m%dT%H%M%SZ)-$$"
        printf '%s\n' "$ref" > "$next/candidate-ref"
        printf '%s\n' "$run" > "$next/retry-of"
        record "RETRY $next candidate=$(read_value candidate-sha)"
        cp "$run/record.txt" "$next/prior-record.txt"
        g -C "$caller" update-ref "$ref" "$(read_value candidate-sha)" 0000000000000000000000000000000000000000
        g -C "$caller" worktree add --detach "$next/candidate" "$ref" > "$next/rebase.log" 2>&1
        run=$next; candidate="$run/candidate"
        guard
        record "PREPARED retry-of=$(read_value retry-of) candidate=$(read_value candidate-sha) tree=$(read_value candidate-tree)"
        printf 'PREPARED %s\n' "$run"
        ;;
    check)
        test "$#" = 1 || die 'check RUN_DIR focused|native|motion'
        phase=$1
        case "$phase" in
            focused) ;;
            native) test -f "$run/focused.pass" || die 'focused checks must pass first';;
            motion) test -f "$run/native.pass" || die 'native checks must pass first';;
            *) die 'unknown mandatory check phase';;
        esac
        mkdir "$run/$phase" || die 'phase already attempted; preserve evidence and consult retry policy'
        record "START $phase"
        if "$adapter" "$phase" "$candidate" "$(read_value candidate-sha)" \
            "$(read_value upstream)" "$run/$phase" > "$run/$phase.log" 2>&1; then
            guard
            test -s "$run/$phase/artifacts.sha256" || die 'missing check artifacts'
            save "$phase.artifacts-sha256" "$(hash "$run/$phase/artifacts.sha256")"
            hash "$run/$phase.log" > "$run/$phase.pass"
            record "PASS $phase log-sha256=$(cat "$run/$phase.pass") artifacts-sha256=$(read_value "$phase.artifacts-sha256")"
        else
            result=$?
            record "FAIL $phase exit=$result; no proposal; retain accepted stack"
            exit "$result"
        fi
        ;;
    propose)
        test "$#" = 0 || die 'propose RUN_DIR'
        test ! -e "$run/proposal.txt" && test ! -e "$run/check-record.txt" || die 'run already proposed'
        for phase in focused native motion; do
            test -f "$run/$phase.pass" || die "missing $phase pass"
            test "$(cat "$run/$phase.pass")" = "$(hash "$run/$phase.log")" || die 'check log changed'
            test "$(read_value "$phase.artifacts-sha256")" = "$(hash "$run/$phase/artifacts.sha256")" || die 'artifact manifest changed'
            sha256sum --check --status "$run/$phase/artifacts.sha256" || die 'check artifacts changed'
        done
        proposal="refs/eon-sync/proposals/$(read_value candidate-sha)"
        g -C "$caller" update-ref "$proposal" "$(read_value candidate-sha)" 0000000000000000000000000000000000000000
        record "PROPOSED $proposal"
        cp "$run/record.txt" "$run/check-record.txt"
        {
            printf 'revision %s\ntree %s\nupstream %s\naccepted %s\n' \
                "$(read_value candidate-sha)" "$(read_value candidate-tree)" \
                "$(read_value upstream)" "$(read_value accepted-sha)"
            printf 'check-record-sha256 %s\n' "$(hash "$run/check-record.txt")"
        } > "$run/proposal.txt"
        cat "$run/proposal.txt"
        ;;
    discard)
        test "$#" = 0 || die 'discard RUN_DIR'
        cleanup
        record 'DISCARDED owned candidate; proposal/evidence retained'
        ;;
    *) die 'expected prepare, retry, check, propose or discard';;
esac
