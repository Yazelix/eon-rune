#!/usr/bin/env bash
# Real Git failures with fake checks: this proves orchestration, never native behavior.
set -euo pipefail
if test "${RUNE_SYNC_BOUNDED:-0}" != 1; then
    export RUNE_SYNC_PRIVATE_ROOT
    RUNE_SYNC_PRIVATE_ROOT=$(mktemp -d "${1:?existing build-disk proof parent}/git-env-XXXXXX")
    exec python3 "$(dirname "$0")/launch.py" bash "$0" "$@"
fi
test "$HOME" = "$RUNE_SYNC_PRIVATE_ROOT/home"
test "$GIT_CONFIG_GLOBAL" = /dev/null
workflow=$(realpath "$(dirname "$0")/../sync.sh")
scratch=$(mktemp -d "${1:?usage: fork-sync/test.sh EXISTING_PROOF_PARENT}/git-proof-XXXXXX")
mkdir -p "$scratch/evidence/native-cache" "$scratch/evidence/logs" "$scratch/evidence/focused/selected/source"
mkdir -p "$scratch/evidence/native-home"
printf 'private home\n' > "$scratch/evidence/native-home/settings"
printf 'cache\n' > "$scratch/evidence/native-cache/index"
printf 'check\n' > "$scratch/evidence/logs/check.log"
printf 'identity\n' > "$scratch/evidence/focused/selected/identity.json"
printf 'source\n' > "$scratch/evidence/focused/selected/source/lib.rs"
"$(dirname "$workflow")/fork-sync/artifacts.sh" "$scratch/evidence" > "$scratch/artifacts.sha256"
printf 'cache changed\n' >> "$scratch/evidence/native-cache/index"
printf 'private home changed\n' >> "$scratch/evidence/native-home/settings"
sha256sum --check --status "$scratch/artifacts.sha256"
printf 'identity changed\n' >> "$scratch/evidence/focused/selected/identity.json"
if sha256sum --check --status "$scratch/artifacts.sha256"; then
    echo 'changed focused identity incorrectly accepted' >&2
    exit 1
fi
git init -q "$scratch/repo"
cd "$scratch/repo"
git config user.name 'Sync self-check'
git config user.email 'sync@example.invalid'
git config commit.gpgsign false
printf 'base\n' > value
git add value; git commit -qm base
base=$(git rev-parse HEAD)
printf 'patch\n' > 'patch'
git add patch; git commit -qm 'fork patch'
accepted=$(git rev-parse HEAD)
git branch accepted
git checkout -q --detach "$base"
printf 'upstream\n' > upstream
git add upstream; git commit -qm upstream
upstream=$(git rev-parse HEAD)
git checkout -q --detach "$accepted"
# A hostile local default must not update accepted or invoke hooks.
git config rebase.updateRefs true
git config rerere.enabled true
mkdir "$scratch/hooks"
printf '#!/bin/sh\nexit 99\n' > "$scratch/hooks/pre-rebase"
chmod +x "$scratch/hooks/pre-rebase"
git config core.hooksPath "$scratch/hooks"
cat > "$scratch/check" <<'CHECK'
#!/usr/bin/env bash
set -euo pipefail
test "$(git -C "$2" rev-parse HEAD)" = "$3"
printf 'fake check phase=%s candidate=%s upstream=%s\n' "$1" "$3" "$4"
sha256sum "$2/patch" > "$5/artifacts.sha256"
if test "${FAIL_SYNC_PHASE:-}" = "$1"; then
    printf 'synthetic functional failure\n' > "$5/functional-failure"
    exit 1
fi
CHECK
chmod +x "$scratch/check"
unchanged() {
    test "$(git rev-parse HEAD)" = "$accepted"
    test "$(git rev-parse accepted)" = "$accepted"
    test -z "$(git status --porcelain)"
}
reject() { if "$@"; then echo 'unexpected success' >&2; exit 1; fi; }
"$workflow" prepare accepted "$base" "$upstream" uncreated-fork "$scratch/success" "$scratch/check"
for phase in focused native motion; do "$workflow" check "$scratch/success" "$phase"; done
cp "$scratch/success/focused/artifacts.sha256" "$scratch/manifest-backup"
printf 'changed\n' >> "$scratch/success/focused/artifacts.sha256"
reject "$workflow" propose "$scratch/success"
test ! -e "$scratch/success/proposal.txt"
cp "$scratch/manifest-backup" "$scratch/success/focused/artifacts.sha256"
"$workflow" propose "$scratch/success"
test -s "$scratch/success/proposal.txt"
proposed=$(cat "$scratch/success/candidate-sha")
test "$proposed" != "$accepted"
test "$(git show "$proposed":patch)" = patch
test "$(git show "$proposed":upstream)" = upstream
reject "$workflow" propose "$scratch/success"
unchanged
"$workflow" discard "$scratch/success"
recorded=$(sed -n 's/^check-record-sha256 //p' "$scratch/success/proposal.txt")
test "$(sha256sum "$scratch/success/check-record.txt" | cut -d ' ' -f 1)" = "$recorded"
git update-ref -d "refs/eon-sync/proposals/$proposed" "$proposed"
reject "$workflow" prepare accepted "$base" "$upstream" fork "$scratch/success" "$scratch/check"
printf 'unsaved\n' > untracked
reject "$workflow" prepare accepted "$base" "$upstream" fork "$scratch/dirty" "$scratch/check"
test "$(cat untracked)" = unsaved
rm untracked
printf 'unsaved\n' >> value
reject "$workflow" prepare accepted "$base" "$upstream" fork "$scratch/modified" "$scratch/check"
git restore value
"$workflow" prepare accepted "$base" "$upstream" fork "$scratch/failure" "$scratch/check"
"$workflow" check "$scratch/failure" focused
reject env FAIL_SYNC_PHASE=native "$workflow" check "$scratch/failure" native
reject "$workflow" propose "$scratch/failure"
test ! -e "$scratch/failure/proposal.txt"
unchanged
# A real rebase must retain its exact commit across the single authorized retry.
printf '{}\n' > "$scratch/failure/native/resource-failure.json"
reject "$workflow" retry "$scratch/failure" "$scratch/resource-retry"
test ! -e "$scratch/failure/retry-used"
rm "$scratch/failure/native/resource-failure.json"
failed_sha=$(cat "$scratch/failure/candidate-sha")
test "$failed_sha" != "$accepted"
test "$(git rev-parse "$failed_sha^")" = "$upstream"
sleep 2
reject "$workflow" retry "$scratch/failure" "$scratch/failure/candidate/nested-retry"
test ! -e "$scratch/failure/retry-used"
"$workflow" retry "$scratch/failure" "$scratch/retry"
test "$(cat "$scratch/retry/candidate-sha")" = "$failed_sha"
test "$(git -C "$scratch/retry/candidate" rev-parse HEAD)" = "$failed_sha"
test -s "$scratch/retry/prior-record.txt"
for phase in focused native; do "$workflow" check "$scratch/retry" "$phase"; done
reject "$workflow" retry "$scratch/failure" "$scratch/retry-again"
reject "$workflow" retry "$scratch/retry" "$scratch/retry-chain"
test ! -e "$scratch/retry/motion" && test ! -e "$scratch/retry/motion.pass"
test -d "$scratch/failure/native" && test ! -e "$scratch/failure/native.pass"
unchanged
"$workflow" discard "$scratch/retry"
# A changed workflow cannot qualify a run created by the original workflow.
cp "$workflow" "$scratch/changed-workflow"
printf '\n# changed\n' >> "$scratch/changed-workflow"
reject bash "$scratch/changed-workflow" check "$scratch/failure" native
"$workflow" discard "$scratch/failure"
# The same file changed differently upstream creates a genuine rebase conflict.
git -c core.hooksPath=/dev/null checkout -q --detach "$base"
printf 'upstream patch\n' > 'patch'
git add patch; git commit -qm 'conflicting upstream'
conflict=$(git rev-parse HEAD)
git -c core.hooksPath=/dev/null checkout -q --detach "$accepted"
reject "$workflow" prepare accepted "$base" "$conflict" fork "$scratch/conflict" "$scratch/check"
test -s "$scratch/conflict/conflict.txt"
test ! -e "$scratch/conflict/candidate"
reject "$workflow" check "$scratch/conflict" focused
unchanged
# An upstream equivalent must stop rather than silently drop the carried patch.
git -c core.hooksPath=/dev/null checkout -q --detach "$base"
printf 'patch\n' > 'patch'
git add patch; git commit -qm 'upstream equivalent'
equivalent=$(git rev-parse HEAD)
git -c core.hooksPath=/dev/null checkout -q --detach "$accepted"
reject "$workflow" prepare accepted "$base" "$equivalent" fork "$scratch/empty" "$scratch/check"
unchanged
git -c core.hooksPath=/dev/null checkout -q --orphan unrelated
git rm -q --cached -r .
git -c core.hooksPath=/dev/null commit -qm unrelated --allow-empty
unrelated=$(git rev-parse HEAD)
rm patch value
git -c core.hooksPath=/dev/null checkout -q --detach "$accepted"
reject "$workflow" prepare accepted "$base" "$unrelated" fork "$scratch/unrelated" "$scratch/check"
unchanged
test -z "$(git for-each-ref --format='%(refname)' refs/eon-sync/)"
test "$(git worktree list --porcelain | sed -n '/^worktree /p' | wc -l)" = 1
git branch -D accepted unrelated > /dev/null
default_ref=$(git symbolic-ref --quiet HEAD || true)
test -z "$default_ref"  # All worktrees are detached; delete only the initial local branch.
initial_ref=$(git for-each-ref --format='%(refname)' refs/heads/)
test "$(printf '%s\n' "$initial_ref" | wc -l)" = 1
git update-ref -d "$initial_ref" "$accepted"
printf 'PASS success/conflict/check-failure/dirty/unrelated/empty/immutable-ref; records: %s\n' "$scratch"
