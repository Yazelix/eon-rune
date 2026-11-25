"""Negative controls for verdict and frozen-source gates; no Cargo or native runs."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from native.owned import clean_env


def check(command, *, accepted, env=None):
    if env is None:
        env = clean_env(os.environ, os.environ['RUNE_SYNC_PRIVATE_ROOT'])
    result = subprocess.run(command, capture_output=True, text=True, env=env)
    assert (result.returncode == 0) == accepted, (command, result.stdout, result.stderr)


def main():
    tools = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(dir=sys.argv[1], prefix='gate-controls-') as directory:
        state = Path(directory)
        log = state / 'verdict.log'
        # Linux and non-Linux compile different PRIMARY regressions; each stays mandatory in its group.
        groups = json.loads((tools / 'required-tests.json').read_text())
        nonlinux, = groups['primary-unsupported-nonlinux']
        assert nonlinux not in groups['clipboard']
        for group in ('allocation', 'clipboard', 'primary-unsupported-nonlinux'):
            names = groups[group]
            passed = ''.join(f'test {name} ... ok\n' for name in names)
            final = f'test result: ok. {len(names)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n'
            command = [sys.executable, str(tools / 'verify-tests.py'), group, str(log)]
            for text, accepted in [(passed + final, True),
                                   (passed.replace(names[0], 'missing-regression') + final, False),
                                   (passed, False), (final, False),
                                   (passed + final + final, False)]:
                log.write_text(text)
                check(command, accepted=accepted)
        source = state / 'focused/selected/source'
        source.mkdir(parents=True)
        body = b'fn retained_assertion() {}\n'
        file = source / 'lib.rs'
        file.write_bytes(body)
        identity = dict(source=str(source), staged_tree='synthetic-control', broad_suite='not a runtime proof',
                        files_sha256={'lib.rs': hashlib.sha256(body).hexdigest()})
        (source.parent / 'identity.json').write_text(json.dumps(identity))
        command = [sys.executable, str(tools / 'verify-focused.py'), str(source)]
        env = clean_env(os.environ, os.environ['RUNE_SYNC_PRIVATE_ROOT'])
        env['RUNE_SYNC_STATE'] = str(state)
        check(command, accepted=True, env=env)
        file.write_bytes(body + b'changed\n')
        check(command, accepted=False, env=env)
        file.write_bytes(body)
        (source / 'extra.rs').write_text('unrecorded')
        check(command, accepted=False, env=env)
    print('PASS complete verdict, missing regression/final summary, duplicate summary, platform-specific required groups and frozen source tamper controls')


if __name__ == '__main__':
    main()
