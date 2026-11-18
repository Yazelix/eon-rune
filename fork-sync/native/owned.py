"""Private child environments, live resource fencing and explicitly started processes."""
import json
import os
from pathlib import Path
import signal
import subprocess
import time
import sys
import re

LIMIT = 8482560409  # Largest whole byte count at or below 7.9 GiB.
ENV_KEYS = (
    'PATH', 'NIX_CC', 'NIX_BINTOOLS', 'NIX_CFLAGS_COMPILE', 'NIX_LDFLAGS',
    'PKG_CONFIG_PATH', 'PKG_CONFIG_LIBDIR', 'PKG_CONFIG_SYSTEM_INCLUDE_PATH',
    'PKG_CONFIG_SYSTEM_LIBRARY_PATH', 'RUNE_SYNC_PRIVATE_ROOT', 'RUNE_SYNC_CARGO_SEED',
    'RUNE_SYNC_STATE', 'RUNE_SYNC_CANDIDATE', 'RUNE_SYNC_TREE', 'RUNE_SYNC_LABEL', 'RUNE_SYNC_LOCKS',
    'RUNE_SYNC_TOOLCHAIN', 'RUNE_SYNC_HEAVY', 'RUNE_SYNC_HEAVY_LEDGER',
    'RUNE_SYNC_SWAY', 'RUNE_SYNC_RENDER_NODE', 'RUNE_SYNC_INTEL_DRIVER',
    'RUNE_SYNC_LIBRARIES', 'RUNE_SYNC_EON_SOURCE', 'RUNE_SYNC_STATUS', 'RUNE_SYNC_VERIFY_HOST_INPUTS',
    'RUNE_SYNC_QUIET_START', 'RUNE_SYNC_QUIET_END', 'HEAVY_QUIET_EXEMPT',
    'HEAVY_DIR', 'HEAVY_LOCAL', 'HEAVY_LOCAL_RUNNER', 'CARGO_TARGET_DIR',
    'RUNE_SYNC_BUILD_HELD',
)


def clean_env(values, root):
    root = Path(root).resolve(strict=True)
    assert root.is_relative_to('/mnt/build'), 'private run root must be on the build disk'
    assert root.stat().st_uid == os.getuid(), 'private run root must be owned by this user'
    env = {key: values[key] for key in ENV_KEYS if key in values}
    if 'RUNE_SYNC_STATE' in env:
        state = Path(env['RUNE_SYNC_STATE'])
        if state.name != 'state' or state.parent != root.parent:
            env.pop('RUNE_SYNC_STATE')
    env.update(RUNE_SYNC_PRIVATE_ROOT=str(root), LANG='C.UTF-8', LC_ALL='C.UTF-8', HOME=str(root / 'home'),
               CARGO_HOME=str(root / 'cargo'), TMPDIR=str(root / 'tmp'),
               CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='1', RUST_TEST_THREADS='1',
               CARGO_INCREMENTAL='0', CARGO_PROFILE_TEST_DEBUG='0',
               CARGO_PROFILE_TEST_CODEGEN_UNITS='256', FONTCONFIG_FILE='/etc/fonts/fonts.conf',
               GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_SYSTEM='/dev/null',
               GIT_CONFIG_NOSYSTEM='1', GIT_TERMINAL_PROMPT='0', GIT_OPTIONAL_LOCKS='0',
               PYTHONDONTWRITEBYTECODE='1')
    for kind in ('CACHE', 'CONFIG', 'DATA', 'STATE', 'RUNTIME'):
        key = 'XDG_RUNTIME_DIR' if kind == 'RUNTIME' else 'XDG_' + kind + '_HOME'
        env[key] = str(root / kind.lower())
    for key in ('HOME', 'CARGO_HOME', 'TMPDIR', 'XDG_CACHE_HOME', 'XDG_CONFIG_HOME',
                'XDG_DATA_HOME', 'XDG_STATE_HOME', 'XDG_RUNTIME_DIR'):
        assert not Path(env[key]).is_symlink(), 'private directories cannot redirect to shared state'
        Path(env[key]).mkdir(mode=0o700, exist_ok=True)
    if 'RUNE_SYNC_STATE' in env:
        libraries = Path(env['RUNE_SYNC_STATE']) / 'syslibs'
        if libraries.is_dir():
            env['LD_LIBRARY_PATH'] = str(libraries)
    return env


def sealed_tree(repo, tree):
    assert re.fullmatch(r'[0-9a-f]{40}', tree), 'requires the sealed tree ID'
    env = clean_env(os.environ, os.environ['RUNE_SYNC_PRIVATE_ROOT'])
    actual = subprocess.check_output(['git', '--no-optional-locks', '-C', str(repo),
                                      'rev-parse', tree + '^{tree}'], env=env, text=True).strip()
    assert actual == tree
    return actual


def cargo_dir(manifest):
    directory = Path(manifest).resolve(strict=True).parent
    assert directory.is_relative_to('/mnt/build'), 'Cargo must run from a private build-disk export'
    for parent in (directory, *directory.parents):
        for name in ('config', 'config.toml'):
            assert not (parent / '.cargo' / name).exists(), 'unreviewed ancestor Cargo configuration'
    return directory


class ResourceViolation(RuntimeError):
    pass


def resource_failed(sample):
    return sample['peak'] > LIMIT or sample['events']['oom'] != 0 or sample['events']['oom_kill'] != 0


class Resources:
    def __init__(self, scope=None, marker=None):
        if scope is None:
            group = next(line[3:] for line in Path('/proc/self/cgroup').read_text().splitlines() if line.startswith('0::'))
            scope = Path('/sys/fs/cgroup') / group.lstrip('/')
        self.scope = Path(scope)
        root = Path(os.environ.get('RUNE_SYNC_STATE', os.environ.get('RUNE_SYNC_PRIVATE_ROOT', '.')))
        self.marker = Path(marker) if marker is not None else root / 'resource-failure.json'

    def sample(self):
        return dict(peak=int((self.scope / 'memory.peak').read_text()),
                    events={k: int(v) for k, v in (line.split() for line in (self.scope / 'memory.events').read_text().splitlines())})

    def check(self):
        if self.marker.exists():
            raise ResourceViolation('resource failure already recorded: ' + str(self.marker))
        assert (self.scope / 'memory.max').read_text().strip() == '8589934592'
        assert (self.scope / 'memory.swap.max').read_text().strip() == '0'
        sample = self.sample()
        return self.validate(sample)

    def validate(self, sample):
        if resource_failed(sample):
            self.marker.parent.mkdir(parents=True, exist_ok=True)
            try:
                with self.marker.open('x') as output:
                    json.dump(dict(scope=str(self.scope), **sample), output)
            except FileExistsError:
                pass
            raise ResourceViolation('resource violation; stop without retry: ' + str(self.marker))
        return sample


def check_resources():
    return Resources().check()


def retryable(state, label):
    state = Path(state)
    rows = list((state / 'resources').glob(label + '-*.json'))
    if (state / 'resource-failure.json').exists() or len(rows) != 1:
        return False
    row = json.loads(rows[0].read_text())
    try:
        return row.get('resource_ok') is True and row.get('child_exit', -1) >= 0 and not resource_failed(row)
    except (KeyError, TypeError):
        return False


class Processes:
    def __init__(self):
        self.started = []
        self.finished = []

    def start(self, args, *, env, **kwargs):
        check_resources()
        assert 'start_new_session' not in kwargs
        kwargs.setdefault('stdin', subprocess.DEVNULL)
        process = subprocess.Popen(args, env=env, start_new_session=True, **kwargs)
        self.started.append(process)
        return process

    def run(self, args, *, env, check=False, capture_output=False, timeout=None, **kwargs):
        if capture_output:
            kwargs.update(stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        process = self.start(args, env=env, **kwargs)
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except BaseException:
            self.stop(process)
            self.started.remove(process)
            self.finished.append(process.pid)
            raise
        result = subprocess.CompletedProcess(args, process.returncode, stdout, stderr)
        self.stop(process)
        self.started.remove(process)
        self.finished.append(process.pid)
        check_resources()
        if check:
            result.check_returncode()
        return result

    def stop(self, process):
        assert any(process is item for item in self.started), 'process was not started here'
        # Every process starts a fresh group; descendants remain ours even if its leader exits.
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(process.pid, sig)
            except ProcessLookupError:
                break
            try:
                process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                continue
            if sig == signal.SIGTERM:
                time.sleep(.05)
        process.wait(timeout=2)

    def close(self):
        for process in reversed(self.started):
            self.stop(process)
        return self.finished + [process.pid for process in self.started]


if __name__ == '__main__':
    if sys.argv[1] == 'guard':
        check_resources()
    elif sys.argv[1] == 'cargo-dir':
        print(cargo_dir(sys.argv[2]))
    else:
        assert sys.argv[1] == 'retryable'
        raise SystemExit(0 if retryable(sys.argv[2], sys.argv[3]) else 1)
