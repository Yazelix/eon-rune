# Weekly fork sync

The maintainer owns this fork and approves every application revision bump.
Budget each weekly sync to one implementation session, including review fixes.
This tool prepares a local proposal; it never accepts, pushes, posts, schedules
work or edits an application pin. A failed sync leaves the last accepted stack.

## Inputs and prerequisites

Use a clean worktree at the accepted stack, Bash, Git with `--empty=stop`
(Git 2.56 is verified), Python 3.11 or later, GNU coreutils, and the existing Rust 1.99.1
toolchain. No compiler is vendored. Qualification is offline. Populate a separate
credential-free source seed only in an explicitly authorized, logged step using
`cargo fetch --locked` for its exact input locks, then make it read-only.
Qualification never fetches implicitly or reads the caller's Cargo home. A
missing source stops the run; obtain it through a separate approved fetch. The Linux/Wayland checks need systemd user
scopes for an 8 GiB/no-swap bound, isolated headless Sway, Intel Vulkan/Mesa,
Wayland client/xkbcommon development libraries, cc, pkg-config, dbus-run-session,
wl-copy/wl-paste, wtype, grim, pidstat and the machine's heavy-run scheduler.

The check drivers, frozen fixtures, dependency lock and source recipe are in
`fork-sync/`. `fixture-source.json` pins the owned fixture dependency to an Eon
revision and file hashes; `fixtures/hosted.patch` adapts only its hosted renderer.
Supply a read-only Git clone containing that revision, or fetch the named exact
revision from the recipe's URL into an isolated clone. No current Eon checkout
files are used as fixture source. Direct Rune checks retain the candidate's vendored
winit and checked-in lock. The matched fixtures share that winit source and use
corresponding local renderer and accessibility dependencies from each Rune tree.
Darwin and the consumer's chosen winit are qualified separately. Generated Wayland bindings retain
their source notices; the recipe inventories their XML origins and versions.

The fork-only tooling commit sits after the runtime patches, has no upstream
issue or drop-when-fixed obligation, and its size is disclosed separately from
the runtime patches. Run the trusted script/checks from accepted state,
not candidate code. Keep tooling changes in that separate commit at landing.

This tooling belongs to the Rune fork. It follows Rune's existing Python check
script conventions; it installs no Eon command or Eon repository policy. Bash
only composes Git and check processes. Maintain these helpers with the fork's
tooling commit and disclose its dependencies when proposing publication.

## Run

Read the prior private check/maintenance record. It names the accepted commit,
old upstream base, adoption date, patch inventories, prior failures and session
cost. Check current upstream fixes, licence/Linux support and the render hook.
Fetch the explicit upstream remote, then resolve its dev to one immutable SHA.
The fork remote is a recorded parameter: it need not exist yet.

```bash
git status --short
git fetch "$UPSTREAM_REMOTE" dev
UPSTREAM=$(git rev-parse FETCH_HEAD)
ACCEPTED=$(git rev-parse HEAD)
# Set OLD_BASE from the prior accepted record; never infer it from merge-base.
WORK=/mnt/build/project-agents/rune-sync-proof
mkdir -p "$WORK"
export CARGO_TARGET_DIR=/mnt/build/project-agents/rune-sync-target
export RUNE_SYNC_CARGO_SEED=/mnt/build/project-agents/approved-rune-sources
export RUNE_SYNC_TOOLCHAIN=/path/to/rust-1.99.1
export RUNE_SYNC_EON_SOURCE=/path/to/read-only-eon-clone
export RUNE_SYNC_HEAVY=/path/to/machine-heavy-wrapper
export RUNE_SYNC_HEAVY_LEDGER=/path/to/machine-heavy-ledger
export RUNE_SYNC_SWAY=/path/to/sway
export RUNE_SYNC_RENDER_NODE=/dev/dri/renderD128
export RUNE_SYNC_INTEL_DRIVER=/path/to/libvulkan_intel.so
export RUNE_SYNC_LAVAPIPE_DRIVER=/path/to/libvulkan_lvp.so
export RUNE_SYNC_LIBRARIES="$WORK/libraries.json"
python3 fork-sync/host-libraries.py "$RUNE_SYNC_INTEL_DRIVER" > "$RUNE_SYNC_LIBRARIES"
RUN="$WORK/weekly-candidate"
./sync.sh prepare "$ACCEPTED" "$OLD_BASE" "$UPSTREAM" "$FORK_REMOTE" "$RUN" "$PWD/fork-sync/checks.sh"
./sync.sh check "$RUN" focused
./sync.sh check "$RUN" native
```

Run in the existing development shell that supplies the pinned compiler and
native tools; never use `--ignore-rust-version`. Keep one Rune build at a time
under the 8 GiB/no-swap scope.
Stop without retry or cap increase on an OOM or peak above 7.9 GiB; record every
scope peak. No 6 GiB compliance is claimed. The Rune build lock
`/mnt/build/rune-build.lock` is shared across all targets
and acquired before queueing every heavy job, including native checks and motion.
The wrapper requests HEAVY_MEM=8G and reuses the
scheduler's verified 8 GiB/no-swap job scope; older uncapped schedulers receive an
explicit equivalent scope. A live watcher fences OOM/oom_kill/peak violations,
stops only its scope's PIDs and prohibits further work or retries. Child
environments use an allowlist; private HOME/XDG/CARGO_HOME replace caller state,
Git global and system config are disabled. The direct export accepts the tracked upstream
Cargo configuration by hash and rejects ancestor configuration.
Only the launcher sees the manager bus; native clients use their private bus.
Each phase records commands, exit codes, compiler identity, memory counters and hashed artifacts.
The focused phase runs Clippy before its builds and tests.
Focused and workspace checks use the same byte-verified source and lock.
Named regressions select the current Cargo test targets for their owner crates
and fail if any required name or completed result is missing. Rune's
`--lib` target is the core unit-test partition. The workspace suite runs each
package in sequence, with a separate invocation for each Rune partition and
standalone target. Those batches use up to two Cargo jobs.
Inherited lint/allocation failures remain disclosed in the private proof record.

Native checks build the exact candidate and **new upstream target** with the
same locked fixture/compiler. They exercise real primary/clipboard/security/
ownership/host routing, dead-key/Space, app identity, ten-second idle, resize at
three scales and hosted resize 10/10 plus controls. They use private displays,
XDG homes and a non-activating bus, never desktop focus or live application roots.
Hosted resize uses generated synthetic terminal text; no captured user screen
is needed. The frozen motion fixture and its scenario remain unchanged.
Self-resize requires its final post-run marker on the exact Rune build.
The close-request driver verifies real `xdg_toplevel.close` events, deferred
render and input on the same surface, programmatic bypass, and a surviving
second main window. The Vulkan gates verify Intel and lavapipe independently.

For a single unreproduced native failure, the maintainer may authorize one full
untimed rerun before acceptance. Preserve the failure and every authorized
diagnostic. Reuse the original rebased commit in a fresh run directory; do not
prepare/rebase again, since a rebase can change committer dates and commit IDs:

```bash
FAILED_RUN=$RUN
RUN="$WORK/weekly-native-retry"
./sync.sh retry "$FAILED_RUN" "$RUN"  # Only after explicit authorization.
cmp "$FAILED_RUN/candidate-sha" "$RUN/candidate-sha"
./sync.sh check "$RUN" focused
./sync.sh check "$RUN" native
./sync.sh discard "$FAILED_RUN"
```

`retry` requires a resource-clean functional native failure receipt; a resource
marker, killed supervisor or missing receipt prohibits it. It preserves the
failed phase's records, copies the exact rebased commit and permits one retry
only. It does not carry any passing phase
receipts forward. If no retry is authorized, run `discard "$RUN"` after recording
the failure. A second failure stops qualification; report both logs and the best
supported mechanism assessment. Do not retry again or start further diagnostics
without the maintainer's direction. Compile/test failures, dirty state and
conflicts are not classified as transient merely to obtain a passing run.

Request **3 minutes** in a standing machine quiet slot after builds are ready.
Only an explicit reservation from the machine's resource scheduler authorizes
the timed phase. Record its granted start and end times in the private evidence.
Run once, at the grant, with the machine-wide wrapper enabled:

```bash
export HEAVY_QUIET_EXEMPT=1
export RUNE_SYNC_QUIET_START=GRANTED_START_EPOCH
export RUNE_SYNC_QUIET_END=GRANTED_END_EPOCH
export RUNE_SYNC_STATUS=/path/to/private/STATUS.md
./sync.sh check "$RUN" motion
./sync.sh propose "$RUN"
./sync.sh discard "$RUN"
```

The unchanged no-fade gate alternates upstream/candidate/upstream/candidate:
930 frames at 120 Hz/scale 2, 870 measured, nine transition witnesses and 36
negative checks each. Both candidate runs must keep **every** measured CPU-wall
frame through submission at or below 1000/120 ms. It covers split, zoom/selection,
restore, popup show/hide and tab switch/return. Each run must also pass the stock
motion analyzer. The fixture allowance matches a full node path, finding kind,
virtual frame and detail exactly once. It accounts for the scripted cuts on
both variants. Review the allowance and analyzer hash together if either input
changes. A failed analyzer or over-budget candidate stops the sequence after
its receipt is saved. pidstat and heavy-ledger
snapshots accompany the grant and cleanup record; QUIET DONE is emitted at exit.

One timed attempt per weekly sync. Failure keeps acceptance and reports a
patch-first follow-up to the maintainer; never retry until a passing
sample. No build runs inside the quiet grant. This timing excludes GPU completion
and presentation and proves only this Linux/Intel workload, not rare-stall freedom.

`proposal.txt` names commit, tree, upstream and immutable `check-record.txt` hash.
Phase logs and artifact manifests are hashed; artifacts are reverified at proposal.
Materialization seals the selected host driver and library paths and contents.
Each native and motion job rechecks them inside the locked scope before launch.
Artifact manifests include the actual driver and library link contents, so an
update after checking invalidates proposal verification.
Mutable XDG runtime/cache directories are excluded; focused-copy identity and
registration diffs are included alongside logs, binaries and native observations.
The create-only
`refs/eon-sync/proposals/<sha>` preserves that commit; existing proposals cannot
be overwritten. Give the record and failures to the maintainer for review. The
maintainer separately approves acceptance/publication/pin changes. `discard`
removes only the owned clean candidate worktree/ref; it retains evidence and the
proposal. Inspect unexpected dirty state; never force-remove it.

## Fixture dependency resolution

`RUNE_SYNC_LOCKS` names a private JSON record with exactly `fixtures`
and `baseline-fixtures` entries, each containing an absolute build-disk `path`
and its reviewed `sha256`. Prepare these locks as an explicitly approved step;
qualification never generates or updates them. Keep private qualification locks
outside the fork. A fixture's local Rune dependency edges may differ from the
upstream baseline, so each graph uses its own lock. Every external package's
name, version, source and checksum must match across the pair.

`resolve-lock.sh` verifies those hashes and external identities, records the
input locks, seed differences and locked metadata under `state/provenance/lock/`,
and installs them only in disposable exports. Every metadata/build/test command
then uses `--offline --locked`. Workspace exports retain the candidate lock.
Native checks verify each fixture lock before building. A missing source, stale
lock, hash mismatch or external package drift
stops qualification. Do not silently change dependency versions or remove locked
checks. The checked-in fixture seed and Rune workspace lock remain unchanged.
Lock inputs, diffs and metadata are sealed by the phase artifact record and
reverified before proposal. Any fixture or lock admission policy change belongs
in the separate tooling commit and requires complete qualification.

Run `bash fork-sync/test-lock.sh EXISTING_PRIVATE_PROOF_PARENT` with the pinned
toolchain for offline stale-lock, local-edge, input-tamper and external-drift controls.

The headless Linux clipboard gate uses the `clipboard` required-test group.
`primary-unsupported-nonlinux` retains the separate regression guarded by
`cfg(not(target_os = "linux"))`; validate that group on a non-Linux test receipt.
It is not a Linux pass. Keep that platform obligation explicit in the final
proof; synthetic gate controls establish checker behavior only.

## Patch removal and exits

An upstream-equivalent patch deliberately stops the rebase. Record its old
commit/subject, exact upstream fix revision and replacement check evidence in
the private check record. After review, construct a separate disposable stack from the
old base by explicitly replaying only the named kept commits (including tooling),
recording old/revised inventories. Do not rewrite the accepted branch. Start a
new complete sync on that revised input; never let Git silently skip/drop it.

The dated maintenance ledger stays private. It records upstream fixes,
runtime production+manifest churn, separately disclosed tests/docs/tooling, and
session costs. The stack has no size cap and no upstreaming deadline; keep each
patch small and modular, and send fixes upstream when that comes naturally.
Stop and report if two consecutive weekly syncs each need a second fresh
implementation session. Also stop for a lost/broken render
hook, MIT/Linux-Vulkan support loss or real-desktop failure. Preserve the last
accepted revision; report performance failures with traces and a bounded patch
estimate rather than abandoning adoption automatically.

Set RUNE_SYNC_PRIVATE_ROOT to a fresh directory under /mnt/build, then run
`python3 fork-sync/launch.py --build python3 fork-sync/test-runtime.py EXISTING_PRIVATE_PROOF_PARENT`
for offline/cache/environment/process/resource/source-ID/shared-lock controls.
Run `python3 fork-sync/launch.py python3 fork-sync/test-gates.py EXISTING_PRIVATE_PROOF_PARENT` for missing
regression/final-verdict and source-tamper controls. Run
`bash fork-sync/test.sh EXISTING_PRIVATE_PROOF_PARENT` for bounded local Git
mechanics checks. Its fake adapters never qualify a real candidate. After real
proof, stage tooling for independent review before landing; review artifacts
stay outside this public repository.
