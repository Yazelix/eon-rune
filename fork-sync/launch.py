"""Serialize Rune jobs before acquiring a machine slot, then enter the bounded scope."""
import os
from pathlib import Path
import sys
sys.dont_write_bytecode = True
import json
import subprocess

from native.owned import clean_env


def command(env, arguments):
    arguments = list(arguments)
    env.pop('RUNE_SYNC_BUILD_HELD', None)
    build = arguments[:1] == ['--build']
    if build:
        arguments = arguments[1:]
        env['RUNE_SYNC_BUILD_HELD'] = '1'
    tools = Path(__file__).resolve().parent
    return ['flock', '--exclusive', '/mnt/build/rune-build.lock',
            'bash', env['RUNE_SYNC_HEAVY'],
            'env', 'CARGO_TARGET_DIR=' + env['CARGO_TARGET_DIR'], 'CARGO_BUILD_JOBS=1',
            'bash', str(tools / 'scope.sh'), str(tools), *arguments]


def main():
    root = Path(os.environ['RUNE_SYNC_PRIVATE_ROOT'])
    assert root.is_absolute() and root.is_relative_to('/mnt/build')
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    env = clean_env(os.environ, root)
    target = Path(env['CARGO_TARGET_DIR'])
    assert target.is_absolute() and target.is_relative_to('/mnt/build'), 'declare a dedicated build-disk target'
    state = Path(env.get('RUNE_SYNC_STATE', str(root)))
    assert not (state / 'resource-failure.json').exists(), 'resource failure prohibits further work'
    arguments = sys.argv[1:]
    if arguments and arguments[0] == '--environment':
        env['RUNE_SYNC_ENV_READY'] = '1'
        os.execvpe(arguments[1], arguments[1:], env)
    # Only the scheduler/scope launcher receives the manager bus. Its child is sanitized again.
    env['XDG_RUNTIME_DIR'] = '/run/user/' + str(os.getuid())
    env['DBUS_SESSION_BUS_ADDRESS'] = 'unix:path=' + env['XDG_RUNTIME_DIR'] + '/bus'
    env['HEAVY_MEM'] = '8G'
    args = command(env, arguments)
    result = subprocess.run(args, env=env, stdin=subprocess.DEVNULL).returncode
    if result in (75, 137, -9):
        state.mkdir(parents=True, exist_ok=True)
        marker = state / 'resource-failure.json'
        try:
            with marker.open('x') as output:
                json.dump(dict(reason='scope resource stop or killed supervisor', exit=result), output)
        except FileExistsError:
            pass
    raise SystemExit(result if result >= 0 else 128 - result)


if __name__ == '__main__':
    main()
