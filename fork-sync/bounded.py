"""Watch the owned scope while children run; a resource failure fences the whole run."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

from native.owned import clean_env, Resources, ResourceViolation
from host_inputs import verify as verify_host_inputs


def signal_scope(scope, sig):
    """Signal only membership of this created scope, using pidfds against PID reuse."""
    prefix = '/' + str(scope.relative_to('/sys/fs/cgroup'))
    for membership in [scope / 'cgroup.procs', *scope.rglob('*/cgroup.procs')]:
        try:
            pids = list(map(int, membership.read_text().split()))
        except FileNotFoundError:
            continue
        for pid in pids:
            if pid == os.getpid():
                continue
            try:
                fd = os.pidfd_open(pid)
                try:
                    groups = Path(f'/proc/{pid}/cgroup').read_text().splitlines()
                    if not any(line == '0::' + prefix or line.startswith('0::' + prefix + '/') for line in groups):
                        continue
                    signal.pidfd_send_signal(fd, sig)
                finally:
                    os.close(fd)
            except (ProcessLookupError, FileNotFoundError):
                pass


def watch(child, guard):
    while True:
        guard.check()
        try:
            child.wait(timeout=.025)
            break
        except subprocess.TimeoutExpired:
            pass
    guard.check()


def main():
    guard = Resources()
    assert guard.scope.name.startswith(('rune-sync-', 'heavy-')), 'requires the owned scheduler/scope unit'
    guard.check()
    env = clean_env(os.environ, os.environ['RUNE_SYNC_PRIVATE_ROOT'])
    if env.get('RUNE_SYNC_VERIFY_HOST_INPUTS') == '1':
        verify_host_inputs(Path(env['RUNE_SYNC_STATE']))
    env['RUNE_SYNC_BOUNDED'] = '1'
    child = subprocess.Popen(sys.argv[1:], env=env, start_new_session=True, stdin=subprocess.DEVNULL)
    violation = False
    try:
        watch(child, guard)
    except ResourceViolation as error:
        violation = True
        print(str(error), file=sys.stderr, flush=True)
    finally:
        signal_scope(guard.scope, signal.SIGTERM)
        time.sleep(.1)
        signal_scope(guard.scope, signal.SIGKILL)
        child.wait(timeout=5)
    sample = guard.sample()
    try:
        guard.validate(sample)
    except ResourceViolation as error:
        violation = True
        print(str(error), file=sys.stderr, flush=True)
    record = dict(scope=str(guard.scope), memory_max=8589934592, swap_max=0,
                  resource_ok=not violation, child_exit=child.returncode, **sample)
    state = Path(env.get('RUNE_SYNC_STATE', env['RUNE_SYNC_PRIVATE_ROOT']))
    receipts = state / 'resources'
    receipts.mkdir(parents=True, exist_ok=True)
    label = env.get('RUNE_SYNC_LABEL', str(os.getpid()))
    assert label and all(c.isalnum() or c in '-_' for c in label)
    (receipts / (label + '-' + str(os.getpid()) + '.json')).write_text(json.dumps(record, indent=2) + '\n')
    print('RESOURCE ' + json.dumps(record), flush=True)
    return 75 if violation else child.returncode if child.returncode >= 0 else 128 - child.returncode


if __name__ == '__main__':
    raise SystemExit(main())
