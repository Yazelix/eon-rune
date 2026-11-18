#!/usr/bin/env python3
"""Disposable #372 native reproduction; all artifacts/processes stay in this task."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time
from owned import clean_env, Processes, check_resources, ResourceViolation

W = Path(os.environ["RUNE_SYNC_STATE"])
R = Path(os.environ["RUNE_SYNC_CANDIDATE"])
SWAYMSG = os.environ["RUNE_SYNC_SWAY"]+"msg"
PROOF = re.compile(r'resize-proof: frame (\d+) size \((\d+), (\d+)\) viewport \(([\d.]+), ([\d.]+)\) scale ([\d.]+) slot Some\(\(\[([^\]]+)\], ([\d.]+)\)\)')
OWNED = Processes()


def private_env():
    env = clean_env(os.environ, os.environ['RUNE_SYNC_PRIVATE_ROOT'])
    (W / 'xdg').mkdir(mode=0o700, exist_ok=True)
    for k in ('DISPLAY', 'WAYLAND_DISPLAY', 'SWAYSOCK', 'I3SOCK', 'DBUS_SESSION_BUS_ADDRESS', 'AT_SPI_BUS_ADDRESS', 'XDG_ACTIVATION_TOKEN', 'DESKTOP_STARTUP_ID'):
        env.pop(k, None)
    env['XDG_RUNTIME_DIR'] = str(W / 'xdg')
    home = W / 'native-home'
    home.mkdir(mode=0o700, exist_ok=True)
    env['HOME'] = str(home)
    env['FONTCONFIG_FILE'] = '/etc/fonts/fonts.conf'
    env['LD_LIBRARY_PATH'] = str(W / 'syslibs')
    env['VK_ICD_FILENAMES'] = str(W / 'intel_abs_icd.json')
    env['VK_DRIVER_FILES'] = str(W / 'intel_abs_icd.json')
    for kind in ('CACHE', 'CONFIG', 'DATA', 'STATE'):
        folder = W / ('native-' + kind.lower())
        folder.mkdir(mode=0o700, exist_ok=True)
        env['XDG_' + kind + '_HOME'] = str(folder)
    return env


def proc_env(pid):
    try:
        return dict(x.split('=', 1) for x in Path(f'/proc/{pid}/environ').read_bytes().decode().split('\0') if '=' in x)
    except (OSError, UnicodeError):
        return {}


def stop(process):
    OWNED.stop(process)


def frames(log):
    return [dict(frame=int(m[1]), size=[int(m[2]), int(m[3])],
                 viewport=[float(m[4]), float(m[5])], scale=float(m[6]),
                 slot=[float(x.strip()) for x in m[7].split(',')]) for m in PROOF.finditer(log)]


def buffers(log):
    sizes = {}
    attached = {}
    commits = []
    main_surface = None
    requested = False
    for line in log.splitlines():
        requested |= 'hosted: asked for 1400x800 at clock' in line
        target = re.search(r'get_xdg_surface\(new id xdg_surface#\d+, wl_surface#(\d+)\)', line)
        if target:
            main_surface = target[1]
        m = re.search(r'create_immed\(new id wl_buffer#(\d+), (\d+), (\d+),', line)
        if m:
            sizes[m[1]] = [int(m[2]), int(m[3])]
        m = re.search(r'wl_surface#(\d+)\.attach\(wl_buffer#(\d+),', line)
        if m:
            attached[m[1]] = sizes.get(m[2])
        m = re.search(r'wl_surface#(\d+)\.commit\(', line)
        if m and requested and m[1] == main_surface and attached.get(m[1]):
            commits.append(attached[m[1]])
    return commits


def check(log, rect, expected=(1400, 800), scale=1):
    fs = frames(log)
    px = [round(x * scale) for x in expected]
    good = [f for f in fs[-1:] if f['size'] == px and f['viewport'] == list(expected) and f['scale'] == scale
            and all(abs(a - b) <= 1 for a, b in zip(f['slot'], [8 * scale, 8 * scale, (expected[0] - 16) * scale, (expected[1] - 50) * scale]))]
    all_committed = buffers(log)
    return dict(request='hosted: asked for 1400x800 at clock' in log,
                hook_viewport_slot=bool(good), compositor=rect == list(expected),
                buffer_commit=bool(all_committed) and all_committed[-1] == px)


def sway(env, *args):
    return json.loads(OWNED.run([SWAYMSG, '-r', *args], env=env, capture_output=True, check=True, timeout=5).stdout)


def nodes(tree):
    yield tree
    for c in tree.get('nodes', []) + tree.get('floating_nodes', []):
        yield from nodes(c)


def geometry(env):
    ns = [n for n in nodes(sway(env, '-t', 'get_tree')) if n.get('name') == 'hosted' and n.get('pid')]
    if len(ns) != 1:
        return None
    return [ns[0]['rect']['width'], ns[0]['rect']['height']]


def snapshot_threads(pid, path):
    data = {}
    for p in Path(f'/proc/{pid}/task').glob('*'):
        data[p.name] = {}
        for name in ('comm', 'wchan', 'stack', 'syscall'):
            try:
                data[p.name][name] = (p / name).read_text()
            except OSError as e:
                data[p.name][name] = str(e)
    path.write_text(json.dumps(data, indent=2) + '\n')


def attempt(binary, label, env, out, control=None):
    tiled = control == 'tiled'
    scale = 1.25 if control == 'fractional' else 1
    expected = (1920, 1080) if tiled else (1400, 800)
    path = out / f'{label}.log'
    app_env = env | {'VK_ICD_FILENAMES': str(W / 'intel_abs_icd.json'),
                     'VK_DRIVER_FILES': str(W / 'intel_abs_icd.json'),
                     'TERMGRID_CAPTURE': str(W / 'capture.ansi'), 'TERMGRID_MODE': 'static',
                     'TERMGRID_RESIZE': '1', 'WAYLAND_DEBUG': 'client'}
    result = {'label': label, 'start_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()), 'pass': False}
    with path.open('wb', buffering=0) as log:
        app = OWNED.start([str(binary)], env=app_env, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
        result['pid'] = app.pid
        start = time.monotonic()
        requested = None
        prepared = tiled
        try:
            while time.monotonic() - start < 30:
                text = path.read_text(errors='replace')
                if app.poll() is not None:
                    result['error'] = f'early exit {app.returncode}'
                    break
                if requested is None and 'hosted: asked for 1400x800 at clock' in text:
                    requested = time.monotonic()
                rect = geometry(env)
                if not prepared and rect is not None:
                    if requested is not None:
                        result['error'] = 'request preceded restored non-tiled precondition'
                        break
                    replies = sway(env, f'[pid={app.pid}] floating enable, resize set width 1920 px height 1080 px, move position 0 px 0 px')
                    if not all(r.get('success') for r in replies):
                        raise RuntimeError(f'floating setup failed: {replies}')
                    prepared = True
                    result['initial_geometry'] = geometry(env)
                    result['setup_at_seconds'] = time.monotonic() - start
                    if result['initial_geometry'] != [1920, 1080]:
                        raise RuntimeError('initial geometry differs from stage B')
                    rect = result['initial_geometry']
                checks = check(text, rect, expected, scale)
                result['checks'] = checks
                if all(checks.values()):
                    settled = time.monotonic()
                    result['converged_seconds'] = settled - start
                    time.sleep(5)
                    stable = check(path.read_text(errors='replace'), geometry(env), expected, scale)
                    if not all(stable.values()):
                        result['error'] = 'geometry lost during five-second hold'
                        break
                    if control == 'override':
                        reply = sway(env, f'[pid={app.pid}] resize set width 1100 px height 700 px')
                        if not all(r.get('success') for r in reply):
                            raise RuntimeError(f'compositor override failed: {reply}')
                        expected = (1100, 700)
                        deadline = time.monotonic() + 5
                        while not all(check(path.read_text(errors='replace'), geometry(env), expected, scale).values()):
                            if time.monotonic() >= deadline:
                                raise RuntimeError('compositor override geometry deadline')
                            time.sleep(.1)
                        result['override_geometry'] = list(expected)
                    before = frames(path.read_text(errors='replace'))[-1]['frame']
                    result['input_at_seconds'] = time.monotonic() - start
                    OWNED.run(['wtype', '-s', '250', '-k', 'Tab', '-s', '50', 'z', '-s', '100'], env=env, stdin=subprocess.DEVNULL, check=True, timeout=4)
                    until = time.monotonic() + 5
                    while time.monotonic() < until:
                        text = path.read_text(errors='replace')
                        after = frames(text)
                        if 'hosted: on_key' in text and after and after[-1]['frame'] > before:
                            result['pass'] = all(check(text, geometry(env), expected, scale).values())
                            break
                        time.sleep(.1)
                    if not result['pass']:
                        result['error'] = 'post-convergence input/redraw witness missing'
                    if result['pass']:
                        OWNED.run(['grim', str(out / f'{label}.png')], env=env, check=True, timeout=5, stdout=subprocess.DEVNULL)
                    break
                if requested is not None and time.monotonic() - requested > 10:
                    result['error'] = 'geometry deadline after request'
                    break
                time.sleep(.1)
            else:
                result['error'] = 'outer timeout before resize convergence'
        except ResourceViolation:
            raise
        except Exception as e:
            result['error'] = f'{type(e).__name__}: {e}'
        finally:
            if not result['pass'] and not (W / 'resource-failure.json').exists():
                snapshot_threads(app.pid, out / f'{label}-threads.json')
            stop(app)
            result['app_exit'] = app.returncode
            result['seconds'] = time.monotonic() - start
    (out / f'{label}.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result), flush=True)
    return result


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('label')
    ap.add_argument('binary', type=Path)
    ap.add_argument('--count', type=int, default=1)
    ap.add_argument('--control', choices=['tiled', 'override', 'fractional'])
    args = ap.parse_args()
    out = W / args.label
    out.mkdir(exist_ok=False)
    env = private_env()
    if list((W / 'xdg').glob('wayland-*')):
        raise RuntimeError('private runtime already has a display; investigate ownership first')
    with (out / 'sway.log').open('wb', buffering=0) as log:
        compositor = OWNED.start(['dbus-run-session', '--config-file=' + str(W / 'native-dbus.conf'), '--', 'bash', str(W / 'sway-up.sh'), 'vulkan'], env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
        results = []
        try:
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                sockets = list((W / 'xdg').glob('sway-ipc.*.sock'))
                if sockets:
                    break
                if compositor.poll() is not None:
                    raise RuntimeError(f'Sway launcher exited {compositor.returncode}')
                time.sleep(.1)
            else:
                raise RuntimeError('private compositor startup timeout')
            pid = int(sockets[0].name.split('.')[-2])
            assert os.getpgid(pid) == compositor.pid, 'Sway must descend in the started process group'
            owned = proc_env(pid)
            if owned.get('XDG_RUNTIME_DIR') != str(W / 'xdg'):
                raise RuntimeError('Sway environment mismatch')
            env |= {'SWAYSOCK': str(sockets[0]), 'WAYLAND_DISPLAY': 'wayland-1', 'DBUS_SESSION_BUS_ADDRESS': owned['DBUS_SESSION_BUS_ADDRESS']}
            if args.control == 'fractional':
                replies = sway(env, 'output HEADLESS-1 resolution 2400x1350 scale 1.25')
                if not all(r.get('success') for r in replies):
                    raise RuntimeError(f'fractional output setup failed: {replies}')
            identity = {'binary': str(args.binary), 'sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                        'sway_pid': pid, 'display': env['WAYLAND_DISPLAY'], 'count': args.count, 'control': args.control,
                        'sway_config_sha256': hashlib.sha256((W / 'sway.conf').read_bytes()).hexdigest()}
            (out / 'identity.json').write_text(json.dumps(identity, indent=2) + '\n')
            for i in range(args.count):
                check_resources()
                results.append(attempt(args.binary, f'{i+1:02}', env, out, args.control))
                if not results[-1]['pass']:
                    break
        finally:
            owned_pids = OWNED.close()
            deadline = time.monotonic() + 5
            while True:
                mounts = [l for l in Path('/proc/mounts').read_text().splitlines() if str(W) in l]
                remaining = [p.pid for p in OWNED.started if p.poll() is None]
                if not mounts and not remaining or time.monotonic() >= deadline:
                    break
                time.sleep(.1)
            if remaining or mounts:
                raise RuntimeError(f'cleanup incomplete: PIDs {remaining}, mounts {mounts}')
            (out / 'summary.json').write_text(json.dumps({'results': results, 'passed': sum(r['pass'] for r in results), 'attempted': len(results), 'cleanup_pids': owned_pids, 'mounts_remaining': mounts}, indent=2) + '\n')
    return 0 if len(results) == args.count and all(r['pass'] for r in results) else 1


if __name__ == '__main__':
    raise SystemExit(main())
