"""Install reviewed private locks; qualification never changes dependency versions."""
import difflib
import hashlib
import json
import os
from pathlib import Path
import sys
import tomllib


def external(lock):
    return {tuple(package.get(key) for key in ('name', 'version', 'source', 'checksum'))
            for package in tomllib.loads(lock.decode())['package'] if package.get('source')}


def matching(candidate, baseline):
    assert external(candidate) == external(baseline), 'fixture external package identities differ'


def self_check():
    body = b'version = 4\n[[package]]\nname="sample"\nversion="1.0"\nsource="registry+fixture"\nchecksum="aaa"\n'
    matching(body, body + b'dependencies=["local"]\n')
    for before, after in ((b'1.0', b'2.0'), (b'registry+fixture', b'git+other'), (b'aaa', b'bbb')):
        try:
            matching(body, body.replace(before, after))
        except AssertionError:
            continue
        raise AssertionError('package identity drift accepted')


def main(state):
    spec = Path(os.environ['RUNE_SYNC_LOCKS']).resolve(strict=True)
    assert spec.is_relative_to('/mnt/build'), 'reviewed lock inputs must be on the build disk'
    entries = json.loads(spec.read_text())
    assert set(entries) == {'workspace', 'fixtures', 'baseline-fixtures'}
    locks = {}
    for name, entry in entries.items():
        source = Path(entry['path']).resolve(strict=True)
        assert source.is_relative_to('/mnt/build')
        body = source.read_bytes()
        assert hashlib.sha256(body).hexdigest() == entry['sha256'], 'reviewed lock hash changed: ' + name
        assert tomllib.loads(body.decode())['package'], 'empty reviewed lock: ' + name
        locks[name] = body
    matching(locks['fixtures'], locks['baseline-fixtures'])
    evidence = state / 'provenance/lock'
    evidence.mkdir()  # Existing evidence is never overwritten.
    (evidence / 'inputs.json').write_bytes(spec.read_bytes())
    (evidence / 'workspace.lock').write_bytes(locks['workspace'])
    for name, saved in (('fixtures', 'resolved.lock'), ('baseline-fixtures', 'baseline-resolved.lock')):
        target = state / name / 'Cargo.lock'
        before = target.read_bytes()
        (evidence / (name + '-seed.lock')).write_bytes(before)
        (evidence / saved).write_bytes(locks[name])
        (evidence / (name + '-input.diff')).write_text(''.join(difflib.unified_diff(
            before.decode().splitlines(keepends=True), locks[name].decode().splitlines(keepends=True),
            fromfile=name + '-seed.lock', tofile=name + '-reviewed.lock')))
        target.write_bytes(locks[name])
    print('Installed reviewed workspace/fixture locks; external fixture identities match')


if __name__ == '__main__':
    self_check()
    if sys.argv[1:] != ['--self-check']:
        main(Path(sys.argv[1]))
