#!/usr/bin/env bash
# Machine paths are inputs; every gate and fixture recipe is owned by this tooling tree.
set -euo pipefail
test "$#" = 6
if test "${RUNE_SYNC_ENV_READY:-0}" != 1; then
    export RUNE_SYNC_PRIVATE_ROOT="$(dirname "$5")/private"
    export RUNE_SYNC_STATE="$(dirname "$5")/state"
    exec python3 "$(dirname "$0")/launch.py" --environment bash "$0" "$@"
fi
phase=$1
export RUNE_SYNC_CANDIDATE=$2
candidate_sha=$3
upstream=$4
evidence=$5
tools=$(cd "$(dirname "$0")" && pwd)
(cd "$tools" && sha256sum --check inputs.sha256)
RUNE_SYNC_STATE="$(dirname "$evidence")/state"
export RUNE_SYNC_STATE
export RUNE_SYNC_PRIVATE_ROOT="$(dirname "$evidence")/private"
export RUNE_SYNC_TREE="$(git -C "$RUNE_SYNC_CANDIDATE" rev-parse "$6^{tree}")"
test "$RUNE_SYNC_TREE" = "$6"
export CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_TEST_CODEGEN_UNITS=256
export CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=1
: "${RUNE_SYNC_HEAVY:?machine heavy-slot wrapper}"
: "${RUNE_SYNC_SWAY:?absolute isolated Sway executable}"
: "${RUNE_SYNC_RENDER_NODE:?explicit GPU render node}"
: "${CARGO_TARGET_DIR:?dedicated target directory}"
test "$(git -C "$RUNE_SYNC_CANDIDATE" rev-parse HEAD)" = "$candidate_sha"
state=$RUNE_SYNC_STATE
trap 'if test -f "$state/resource-failure.json"; then cp "$state/resource-failure.json" "$evidence/resource-failure.json"; fi' EXIT
features=(--no-default-features --features 'accessibility,dbus,embedded-fonts,svg')
cargo=(bash "$tools/cargo.sh")
run() {
    local label=$1 result
    shift
    printf '%s START %s\n' "$(date -u +%FT%TZ)" "$label"
    printf '%q ' "$@" > "$state/logs/$label.command"
    printf '\n' >> "$state/logs/$label.command"
    export RUNE_SYNC_LABEL="$label"
    if "$@" > "$state/logs/$label.log" 2>&1; then result=0; else result=$?; fi
    printf '%s END %s exit=%s\n' "$(date -u +%FT%TZ)" "$label" "$result"
    if test "$result" = 75 || test "$result" = 137 || grep -qF 'heavy.sh: OOM-KILLED' "$state/logs/$label.log"; then
        if test ! -e "$state/resource-failure.json"; then
            printf '{"reason":"resource stop or killed scope","label":"%s","exit":%s}\n' "$label" "$result" > "$state/resource-failure.json"
        fi
    fi
    if test "$result" != 0 && test "$phase" = native; then
        case "$label" in
            primary-native|resize-native|hosted-*)
                if python3 "$tools/native/owned.py" retryable "$state" "$label"; then
                    printf '%s\n' "$label" > "$evidence/functional-failure"
                fi
                ;;
        esac
    fi
    test "$result" = 0
}
heavy() { python3 "$tools/launch.py" "$@"; }
build() { python3 "$tools/launch.py" --build "$@"; }
cd "$RUNE_SYNC_CANDIDATE"
if test "$phase" = native || test "$phase" = motion; then
    export RUNE_SYNC_VERIFY_HOST_INPUTS=1
fi
case "$phase" in
    focused)
        heavy python3 "$tools/materialize.py" "$upstream"
        run test-ownership heavy python3 "$tools/../scripts/test-shards.py" --check "$RUNE_SYNC_CANDIDATE/crates/rune/src"
        run fixture-lock build bash "$tools/resolve-lock.sh" "$state"
        run export-all build python3 "$tools/select-focused.py" all --all-tests
        python3 "$tools/verify-focused.py" "$state/focused/all/source"
        run export-focused build python3 "$tools/select-focused.py" selected
        cmp "$state/focused/all/source/Cargo.lock" "$state/focused/selected/source/Cargo.lock"
        cd "$state/focused/all/source"
        run clippy build "${cargo[@]}" clippy --locked -p rune -p rune-ref --all-targets --features rune-ref/gpu
        run library build "${cargo[@]}" build --locked -p rune --lib "${features[@]}"
        source="$state/focused/selected/source"
        python3 "$tools/verify-focused.py" "$source"
        for group in primary virtual-primary clipboard keys policy; do
            case "$group" in
                primary) filter=runtime::keys::primary::tests;;
                virtual-primary) filter=runtime::selectable::virtual_selection_tests::primary_tests;;
                clipboard) filter=clipboard;;
                keys) filter=window::keys;;
                policy) filter=gpu::memory::tests;;
            esac
            (
                cd "$source"
                run "$group" build "${cargo[@]}" test --locked -p rune --lib --jobs 1 "${features[@]}" "$filter" -- --test-threads=1 --show-output
            )
            python3 "$tools/verify-tests.py" "$group" "$state/logs/$group.log"
        done
        run allocation build "${cargo[@]}" test --locked -p rune --test alloc "${features[@]}" primary_selection_unchanged_key_does_not_allocate_a_payload -- --test-threads=1 --show-output
        python3 "$tools/verify-tests.py" allocation "$state/logs/allocation.log"
        run workspace build "${cargo[@]}" check --locked --workspace --all-targets --jobs 1
        run workspace-tests build "${cargo[@]}" test --locked --workspace --jobs 1 --features rune-kit/markdown,rune-kit/mermaid -- --test-threads=1
        ;;
    native)
        cd "$state/focused/all/source"
        python3 "$tools/verify-focused.py" "$PWD"
        cmp "$state/provenance/lock/resolved.lock" "$state/fixtures/Cargo.lock"
        cmp "$state/provenance/lock/baseline-resolved.lock" "$state/baseline-fixtures/Cargo.lock"
        run gpu build env -u DISPLAY -u WAYLAND_DISPLAY -u SWAYSOCK -u I3SOCK -u DBUS_SESSION_BUS_ADDRESS \
            XDG_RUNTIME_DIR="$state/xdg-gpu" VK_ICD_FILENAMES="$state/intel_abs_icd.json" \
            VK_DRIVER_FILES="$state/intel_abs_icd.json" LD_LIBRARY_PATH="$state/syslibs" \
            XDG_CACHE_HOME="$state/native-cache" XDG_CONFIG_HOME="$state/native-config" \
            XDG_DATA_HOME="$state/native-data" XDG_STATE_HOME="$state/native-state" HOME="$state/native-home" FONTCONFIG_FILE=/etc/fonts/fonts.conf \
            "${cargo[@]}" test --locked -p rune --test layer_bucket_reuse "${features[@]}" -- --test-threads=1 --show-output
        run self-resize-build build "${cargo[@]}" test --locked -p rune --test self_resize "${features[@]}" --no-run --message-format=json
        run fixture-build build "${cargo[@]}" build --locked --manifest-path "$state/fixtures/Cargo.toml" --profile perf -p yazelix-venus --example primary_i --example hosted --example eon_transitions --features rune/frame-profile
        cp "$CARGO_TARGET_DIR/perf/examples/primary_i" "$state/candidates/primary"
        cp "$CARGO_TARGET_DIR/perf/examples/hosted" "$state/candidates/hosted"
        cp "$CARGO_TARGET_DIR/perf/examples/eon_transitions" "$state/candidates/motion-candidate"
        run baseline-build build "${cargo[@]}" build --locked --manifest-path "$state/baseline-fixtures/Cargo.toml" --profile perf -p yazelix-venus --example eon_transitions --features rune/frame-profile
        cp "$CARGO_TARGET_DIR/perf/examples/eon_transitions" "$state/candidates/motion-upstream"
        python3 "$tools/freeze.py" "$candidate_sha" "$upstream"
        run injector build python3 "$state/native-tools/build-injector.py"
        run primary-native heavy bash "$tools/primary.sh"
        run resize-native heavy python3 "$state/native-tools/self-resize.py" functional
        run hosted-10 heavy python3 "$state/native-tools/reproduce.py" hosted-10 "$state/candidates/hosted" --count 10
        for control in tiled override fractional; do
            run "hosted-$control" heavy python3 "$state/native-tools/reproduce.py" "hosted-$control" "$state/candidates/hosted" --control "$control"
        done
        ;;
    motion)
        test "${HEAVY_QUIET_EXEMPT:-0}" = 1
        : "${RUNE_SYNC_QUIET_START:?reserved quiet-window UTC epoch}"
        : "${RUNE_SYNC_QUIET_END:?reserved quiet-window UTC epoch}"
        : "${RUNE_SYNC_HEAVY_LEDGER:?machine slot ledger}"
        test "$(date +%s)" -ge "$RUNE_SYNC_QUIET_START"
        test "$(date +%s)" -lt "$((RUNE_SYNC_QUIET_END - 100))"
        run motion heavy bash "$tools/quiet.sh"
        ;;
    *) exit 2;;
esac
"$tools/artifacts.sh" "$state" > "$evidence/artifacts.sha256"
python3 "$tools/host_inputs.py" "$state" >> "$evidence/artifacts.sha256"
