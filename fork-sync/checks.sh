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
features=(--no-default-features --features 'accessibility,dbus,embedded-fonts,svg,test-gpu-noop,test-gpu-counters')
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
        run export-all build python3 "$tools/select-focused.py" all --all-tests
        cd "$state/focused/all/source"
        python3 "$tools/verify-focused.py" "$PWD"
        run clippy build "${cargo[@]}" clippy --locked -p rune -p rune-ref --all-targets --features rune/test-gpu-noop,rune/test-gpu-counters,rune-ref/gpu
        run library build "${cargo[@]}" build --locked -p rune --lib "${features[@]}"
        run wayland-graph build "${cargo[@]}" tree --locked -p rune --target x86_64-unknown-linux-gnu -e normal,build "${features[@]}"
        if rg -q '(x11rb|x11-dl|arboard|sctk-adwaita) v' "$state/logs/wayland-graph.log"; then
            printf '%s\n' 'Wayland-only Rune has an unexpected native dependency' >&2
            exit 1
        fi
        run wayland-only build "${cargo[@]}" test --locked -p rune --test wayland_only "${features[@]}"
        rg -q '^wayland_only: case 1 ok$' "$state/logs/wayland-only.log"
        rg -q '^wayland_only: case 2 ok$' "$state/logs/wayland-only.log"
        for group in primary virtual-primary clipboard keys policy; do
            case "$group" in
                primary) filter=runtime::input::primary_selection::behavioral_tests; targets=(--test unit_selection_tests); count=1;;
                virtual-primary) filter=runtime::selection::selectable::virtual_selection_tests::primary_tests; targets=(--test unit_selection_tests); count=1;;
                clipboard) filter=clipboard; targets=(--lib --test unit_app_tests --test unit_input_tests --test unit_layout_tests --test unit_motion_tests --test unit_native_tests --test unit_paint_tests --test unit_selection_tests); count=8;;
                keys) filter=window::keys; targets=(--test unit_native_tests); count=1;;
                policy) filter=memory::tests; targets=(--lib); count=1;;
            esac
            if test "$group" = policy; then
                run "$group" build "${cargo[@]}" test --locked -p rune-gpu "${targets[@]}" --jobs 1 "$filter" -- --test-threads=1 --show-output
            else
                run "$group" build "${cargo[@]}" test --locked -p rune "${targets[@]}" --jobs 1 "${features[@]}" "$filter" -- --test-threads=1 --show-output
            fi
            python3 "$tools/verify-tests.py" "$group" "$state/logs/$group.log" "$count"
        done
        run allocation build "${cargo[@]}" test --locked -p rune --test alloc "${features[@]}" primary_selection_unchanged_key_does_not_allocate_a_payload -- --test-threads=1 --show-output
        python3 "$tools/verify-tests.py" allocation "$state/logs/allocation.log"
        run wasm build "${cargo[@]}" check --locked -p rune --no-default-features --target wasm32-unknown-unknown --jobs 1
        run workspace build "${cargo[@]}" check --locked --workspace --all-targets --jobs 1
        run workspace-test-targets build "${cargo[@]}" metadata --locked --no-deps --format-version 1
        python3 "$tools/test-batches.py" "$state/logs/workspace-test-targets.log" > "$state/provenance/workspace-test-batches.tsv"
        while IFS=$'\t' read -r batch package selector target; do
            args=()
            if test "$selector" != -; then args+=("$selector"); fi
            if test "$target" != -; then args+=("$target"); fi
            case "$package" in
                rune) args+=(--features test-gpu-noop,test-gpu-counters);;
                rune-kit) args+=(--features markdown,mermaid);;
            esac
            run "workspace-$batch" build "${cargo[@]}" test --locked --jobs 2 -p "$package" "${args[@]}" -- --test-threads=1
        done < "$state/provenance/workspace-test-batches.tsv"
        ;;
    native)
        cd "$state/focused/all/source"
        python3 "$tools/verify-focused.py" "$PWD"
        python3 "$tools/host_inputs.py" "$state" > "$state/provenance/native-host-inputs.sha256"
        for driver in intel lavapipe; do
            gpu=(env -u DISPLAY -u WAYLAND_DISPLAY -u SWAYSOCK -u I3SOCK -u DBUS_SESSION_BUS_ADDRESS
                XDG_RUNTIME_DIR="$state/xdg-gpu" VK_ICD_FILENAMES="$state/${driver}_abs_icd.json"
                VK_DRIVER_FILES="$state/${driver}_abs_icd.json" LD_LIBRARY_PATH="$state/syslibs"
                XDG_CACHE_HOME="$state/native-cache" XDG_CONFIG_HOME="$state/native-config"
                XDG_DATA_HOME="$state/native-data" XDG_STATE_HOME="$state/native-state" HOME="$state/native-home"
                FONTCONFIG_FILE=/etc/fonts/fonts.conf)
            run "$driver-layers" build "${gpu[@]}" "${cargo[@]}" test --locked -p rune --test layer_bucket_reuse "${features[@]}" -- --test-threads=1 --show-output
            case "$driver" in
                intel) rg -qi 'LAYER_ADAPTER .*intel' "$state/logs/$driver-layers.log";;
                lavapipe) rg -qi 'LAYER_ADAPTER .*llvmpipe' "$state/logs/$driver-layers.log";;
            esac
            run "$driver-reference" build "${gpu[@]}" "${cargo[@]}" test --locked -p rune-ref --features gpu --test glyph_abi --test reference --test differential -- --test-threads=1 --show-output
            run "$driver-atlas" build "${gpu[@]}" "${cargo[@]}" test --locked -p rune --test unit_paint_tests "${features[@]}" explicit_atlas_pixels_survive_growth_and_windfoil_promotion -- --test-threads=1 --show-output
            rg -q '^test .*explicit_atlas_pixels_survive_growth_and_windfoil_promotion \.\.\. ok$' "$state/logs/$driver-atlas.log"
            run "$driver-grain" build "${gpu[@]}" "${cargo[@]}" test --locked -p rune-fx --test integration grain_numerics -- --test-threads=1 --show-output
            rg -q '^test .*grain_numerics \.\.\. ok$' "$state/logs/$driver-grain.log"
        done
        run self-resize-build build "${cargo[@]}" test --locked -p rune --test self_resize "${features[@]}" --no-run --message-format=json
        run close-build build "${cargo[@]}" test --locked -p rune --test wayland_close_request "${features[@]}" --no-run --message-format=json
        close_binary=$(python3 "$tools/test-artifact.py" wayland_close_request "$state/logs/close-build.log")
        run injector build python3 "$state/native-tools/build-injector.py"
        run wayland-close-request heavy python3 "$state/native-tools/close-request.py" "$close_binary" "$state/native-close-request" "$state/native-tools/inject"
        run resize-native heavy python3 "$state/native-tools/self-resize.py" functional
        run native-regressions-build build "${cargo[@]}" test --locked -p rune --test unit_native_tests --test overlay_gpu_lifecycle "${features[@]}" --no-run --message-format=json
        native_binary=$(python3 "$tools/test-artifact.py" unit_native_tests "$state/logs/native-regressions-build.log")
        overlay_binary=$(python3 "$tools/test-artifact.py" overlay_gpu_lifecycle "$state/logs/native-regressions-build.log")
        run native-regressions heavy python3 "$state/native-tools/rebase-tests.py" "$native_binary" "$overlay_binary" "$state/native-regressions"
        run fixture-lock build bash "$tools/resolve-lock.sh" "$state"
        run fixture-build build "${cargo[@]}" build --locked --manifest-path "$state/fixtures/Cargo.toml" --profile perf -p yazelix-venus --example primary_i --example hosted --example eon_transitions --features rune/frame-profile
        cp "$CARGO_TARGET_DIR/perf/examples/primary_i" "$state/candidates/primary"
        cp "$CARGO_TARGET_DIR/perf/examples/hosted" "$state/candidates/hosted"
        cp "$CARGO_TARGET_DIR/perf/examples/eon_transitions" "$state/candidates/motion-candidate"
        run baseline-build build "${cargo[@]}" build --locked --manifest-path "$state/baseline-fixtures/Cargo.toml" --profile perf -p yazelix-venus --example eon_transitions --features rune/frame-profile
        cp "$CARGO_TARGET_DIR/perf/examples/eon_transitions" "$state/candidates/motion-upstream"
        python3 "$tools/freeze.py" "$candidate_sha" "$upstream"
        run primary-native heavy bash "$tools/primary.sh"
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
