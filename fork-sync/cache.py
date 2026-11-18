"""Copy only approved fetched registry/Git data into a private offline Cargo home."""
import hashlib
import json
from pathlib import Path
import shutil
import sys
import os

from native.owned import check_resources

seed, destination = map(Path, sys.argv[1:])
assert seed.is_absolute() and destination.is_absolute()
assert seed.is_relative_to('/mnt/build') and destination.is_relative_to('/mnt/build')
seed = seed.resolve(strict=True)
assert seed.is_relative_to('/mnt/build'), 'seed must remain on the build disk'
assert destination.is_relative_to('/mnt/build')
assert seed.stat().st_mode & 0o222 == 0, 'approved seed must be read-only'
for name in ('config', 'config.toml', 'credentials', 'credentials.toml'):
    assert not (destination / name).exists(), 'private cache must be credential/config free'
complete = destination / 'seed.json'

def record_provenance():
    if 'RUNE_SYNC_STATE' in os.environ:
        provenance = Path(os.environ['RUNE_SYNC_STATE']) / 'provenance'
        if provenance.is_dir():
            target = provenance / 'cargo-seed.json'
            if target.exists():
                assert target.read_bytes() == complete.read_bytes()
            else:
                target.write_bytes(complete.read_bytes())

if complete.exists():
    assert json.loads(complete.read_text())['source'] == str(seed)
    record_provenance()
    raise SystemExit(0)
assert not list(destination.iterdir()), 'partial cache seed; preserve it and stop'
records = []
for name in ('registry', 'git'):
    source = seed / name
    if not source.exists():
        continue
    for path in sorted(source.rglob('*')):
        check_resources()
        assert path.is_symlink() or path.stat().st_mode & 0o222 == 0, 'seed contains writable data'
        relative = path.relative_to(seed)
        target = destination / relative
        if path.is_symlink():
            link = path.readlink()
            assert not link.is_absolute() and path.resolve().is_relative_to(seed), 'seed link escapes source data'
            target.parent.mkdir(parents=True, exist_ok=True)
            target.symlink_to(link)
            records.append(dict(path=str(relative), symlink=str(link)))
        elif path.is_dir():
            target.mkdir(parents=True, exist_ok=True)
        elif path.is_file():
            target.parent.mkdir(parents=True, exist_ok=True)
            if path.name == 'alternates':
                relocated = []
                for line in path.read_text().splitlines():
                    old = Path(line)
                    old = old if old.is_absolute() else path.parent.parent / old
                    old = old.resolve(strict=True)
                    assert old.is_relative_to(seed), 'Git alternate escapes the approved seed'
                    relocated.append(str(destination / old.relative_to(seed)))
                target.write_text('\n'.join(relocated) + '\n')
            elif path.name in ('config', '.git') and relative.parts[0] == 'git':
                target.write_text(path.read_text().replace(str(seed) + '/', str(destination) + '/'))
            else:
                shutil.copyfile(path, target)
            target.chmod((path.stat().st_mode & 0o777) | 0o200)
            records.append(dict(path=str(relative), source_sha256=hashlib.sha256(path.read_bytes()).hexdigest(), sha256=hashlib.sha256(target.read_bytes()).hexdigest()))
assert records, 'approved registry/Git seed is empty; request missing inputs'
complete.write_text(json.dumps(dict(source=str(seed), files=records), indent=2) + '\n')
record_provenance()
print('Seeded private offline Cargo data:', len(records), 'files; no config/credentials copied')
