"""Admit host graphics inputs and recheck the exact files consumed by native gates."""
import hashlib
import json
from pathlib import Path
import sys


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def file_input(path):
    path = Path(path).resolve(strict=True)
    assert path.is_file(), f'host input is not a file: {path}'
    return dict(path=str(path), sha256=digest(path))


def icd(driver):
    return {'file_format_version': '1.0.0', 'ICD': {
        'library_path': driver['path'], 'api_version': '1.4.354'}}


def materialize(state, driver, libraries, lavapipe=None):
    driver = file_input(driver)
    syslibs = state / 'syslibs'
    syslibs.mkdir()
    admitted = {}
    for name, target in sorted(libraries.items()):
        assert Path(name).name == name and name not in ('', '.', '..'), f'invalid library name: {name!r}'
        admitted[name] = file_input(target)
        (syslibs / name).symlink_to(admitted[name]['path'])
    (state / 'intel_abs_icd.json').write_text(json.dumps(icd(driver)) + '\n')
    inputs = dict(driver=driver, libraries=admitted)
    if lavapipe is not None:
        inputs['lavapipe'] = file_input(lavapipe)
        (state / 'lavapipe_abs_icd.json').write_text(json.dumps(icd(inputs['lavapipe'])) + '\n')
    seal = state / 'provenance/host-inputs.json'
    with seal.open('x') as stream:
        json.dump(inputs, stream, indent=2)
        stream.write('\n')


def verify(state):
    seal = json.loads((state / 'provenance/host-inputs.json').read_text())
    driver = seal['driver']
    assert json.loads((state / 'intel_abs_icd.json').read_text()) == icd(driver), 'driver ICD changed'
    syslibs = state / 'syslibs'
    assert not syslibs.is_symlink() and {p.name for p in syslibs.iterdir()} == set(seal['libraries']), 'library inventory changed'
    paths = [(Path(driver['path']), driver)]
    if lavapipe := seal.get('lavapipe'):
        assert json.loads((state / 'lavapipe_abs_icd.json').read_text()) == icd(lavapipe), 'lavapipe ICD changed'
        paths.append((Path(lavapipe['path']), lavapipe))
    for name, item in seal['libraries'].items():
        link = syslibs / name
        assert link.is_symlink(), f'library link changed: {name}'
        paths.append((link, item))
    for path, item in paths:
        assert file_input(path) == item, f'host execution input changed: {path}'
    return [(path, item['sha256']) for path, item in paths]


if __name__ == '__main__':
    for path, sha256 in verify(Path(sys.argv[1])):
        # The same paths are checked by sha256sum when approving the proposal.
        assert not any(ch in str(path) for ch in '\n\r\\'), f'cannot seal path: {path!r}'
        print(f'{sha256}  {path}')
