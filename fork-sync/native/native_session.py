"""Own one private compositor and clean up only processes carrying its XDG root."""
from contextlib import contextmanager
import json
from pathlib import Path
import subprocess
import time

from reproduce import W, private_env, proc_env, stop


@contextmanager
def session(output):
    env = private_env()
    if list((W / 'xdg').glob('wayland-*')):
        raise RuntimeError('private display already exists; inspect ownership first')
    with (output / 'sway.log').open('wb', buffering=0) as log:
        compositor = subprocess.Popen(['dbus-run-session', '--config-file=' + str(W / 'native-dbus.conf'), '--', 'bash', str(W / 'sway-up.sh'), 'vulkan'],
            env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 15
            while not (sockets := list((W / 'xdg').glob('sway-ipc.*.sock'))):
                if time.monotonic() >= deadline or compositor.poll() is not None:
                    raise RuntimeError('private compositor startup failed')
                time.sleep(.1)
            pid = int(sockets[0].name.split('.')[-2])
            owned = proc_env(pid)
            assert owned.get('XDG_RUNTIME_DIR') == str(W / 'xdg')
            env.update(SWAYSOCK=str(sockets[0]), WAYLAND_DISPLAY='wayland-1',
                       DBUS_SESSION_BUS_ADDRESS=owned['DBUS_SESSION_BUS_ADDRESS'])
            identity = {'sway_pid': pid, 'display': env['WAYLAND_DISPLAY'], 'socket': env['SWAYSOCK'],
                        'xdg': env['XDG_RUNTIME_DIR'], 'bus': env['DBUS_SESSION_BUS_ADDRESS']}
            identity_file = output / 'session.json'
            identity_file.write_text(json.dumps(identity, indent=2) + '\n')
            identity_file.chmod(0o600)
            yield env
        finally:
            pids = [int(p.name) for p in Path('/proc').iterdir() if p.name.isdigit()
                    and proc_env(p.name).get('XDG_RUNTIME_DIR') == str(W / 'xdg')]
            for pid in pids:
                try:
                    stop(pid)
                except ProcessLookupError:
                    pass
            compositor.wait(timeout=8)
            deadline = time.monotonic() + 5
            while True:
                live = [pid for pid in pids if proc_env(pid).get('XDG_RUNTIME_DIR') == str(W / 'xdg')]
                mounts = [line for line in Path('/proc/mounts').read_text().splitlines() if str(W) in line]
                if not (live or mounts) or time.monotonic() >= deadline:
                    break
                time.sleep(.1)
            (output / 'cleanup.json').write_text(json.dumps({'owned_pids': pids, 'remaining': live, 'mounts': mounts}, indent=2) + '\n')
            assert not live and not mounts, 'private session cleanup incomplete'
