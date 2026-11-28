"""Run the fork's native regression witnesses in a private compositor."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from native_session import session
from reproduce import OWNED, W

native, overlay, output = map(Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=False)
cases = (
    ('window::host::hooks::attention_tests::native_attention_completes_before_returning', 'RUNE_ATTENTION_NATIVE_TEST', 'ATTENTION EXIT PASS'),
    ('window::host::hooks::geometry_tests::native_bound_focus_close_dispatch', 'RUNE_GEOMETRY_TEST', 'BOUND FIFO PASS'),
    ('window::host::hooks::geometry_tests::native_geometry_idle_close_exits_without_wakes', 'RUNE_GEOMETRY_TEST', 'IDLE CLOSE PASS'),
    ('window::host::hooks::geometry_tests::native_shared_creation_forces_geometry_before_draw_and_recreation', 'RUNE_GEOMETRY_TEST', 'SHARED CREATION PASS'),
    ('window::host::hooks::skip_reason_native::native_frame_skips_report_their_reason', 'RUNE_SKIP_REASON_NATIVE', 'SKIP REASON PASS'),
    ('window::host::hooks::gpu_failure_tests::native_terminal_window_and_same_device_surface_control', 'RUNE_GPU_FAILURE_NATIVE', 'terminal-gpu-loss-native: pass'),
    ('window::host::hooks::gpu_failure_tests::linux_cpu_window_handles_failed_gpu_start_by_policy', 'RUNE_GPU_FAILURE_NATIVE', 'terminal-gpu-cpu-first: pass'),
    ('overlay::gpu_failure::tests::terminal_overlay_unpark_releases_its_window', 'RUNE_GPU_FAILURE_NATIVE', 'terminal-overlay-unpark: pass'),
    ('clipboard::wayland_tests::live::live_wayland_round_trip', 'RUNE_CLIPBOARD_LIVE_TEST', None),
    ('clipboard::wayland_tests::live::live_wayland_secret_expiry_and_shutdown', 'RUNE_CLIPBOARD_LIVE_TEST', None),
    ('clipboard::wayland_tests::live::live_wayland_stalled_provider_is_bounded', 'RUNE_CLIPBOARD_LIVE_TEST', None),
)
gpu_tests = {'RUNE_GPU_FAILURE_NATIVE', 'RUNE_SKIP_REASON_NATIVE'}
results = []
with session(output) as env:
    env['RUNE_CPU_FIRST'] = '0'
    for label, driver in (('intel', 'intel'), ('lavapipe', 'lavapipe')):
        icd = str(W / f'{driver}_abs_icd.json')
        gpu = dict(env, VK_ICD_FILENAMES=icd, VK_DRIVER_FILES=icd)
        for name, flag, marker in cases:
            if label != 'intel' and flag not in gpu_tests:
                continue
            path = output / f'{label}-{name.rsplit("::", 1)[1]}.log'
            branch = 'only' if marker == 'terminal-gpu-cpu-first: pass' else '0'
            with path.open('wb') as log:
                run = OWNED.run([str(native), '--ignored', '--exact', name, '--nocapture'],
                                env=gpu | {flag: '1', 'RUNE_CPU_FIRST': branch}, stdout=log, stderr=subprocess.STDOUT, timeout=75)
            text = path.read_text()
            assert run.returncode == 0 and 'test result: ok. 1 passed; 0 failed;' in text, text
            assert marker is None or marker in text, text
            results.append({'test': name, 'driver': label})
        path = output / f'{label}-overlay-lifecycle.log'
        with path.open('wb') as log:
            run = OWNED.run([str(overlay)], env=gpu | {'RUNE_GUI_TESTS': '1'},
                            stdout=log, stderr=subprocess.STDOUT, timeout=125)
        text = path.read_text()
        assert run.returncode == 0 and 'overlay_gpu_lifecycle: ok' in text, text
        results.append({'test': 'overlay_gpu_lifecycle', 'driver': label})
(output / 'result.json').write_text(json.dumps({
    'tests_sha256': hashlib.sha256(native.read_bytes()).hexdigest(),
    'overlay_sha256': hashlib.sha256(overlay.read_bytes()).hexdigest(),
    'cases': results,
}, indent=2) + '\n')
