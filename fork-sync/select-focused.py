import os
"""Export owned-winit build copies; prune only test registrations in the focused copy."""
import difflib
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib
from native.owned import sealed_tree

spec = importlib.util.spec_from_file_location('test_shards', Path(__file__).resolve().parents[1] / 'scripts/test-shards.py')
SHARDS = importlib.util.module_from_spec(spec)
spec.loader.exec_module(SHARDS)

WORK = Path(os.environ["RUNE_SYNC_STATE"])
ROOT = Path(os.environ["RUNE_SYNC_CANDIDATE"])
KEEP = {
    ('gpu/memory.rs', 'tests'),
    ('clipboard.rs', 'tests'),
    ('clipboard.rs', 'secret_tests'),
    ('clipboard/html.rs', 'tests'),
    ('clipboard/plain.rs', 'tests'),
    ('runtime/selection/selectable.rs', 'virtual_selection_tests'),
    ('runtime.rs', 'tests'),
    ('runtime.rs', 'live_only_tests'),
    ('runtime/input/primary.rs', 'tests'),
    ('tooling/interaction.rs', 'tests'),
    ('window/keys.rs', 'tests'),
    ('window/keys.rs', 'layout_tests'),
}
MODULE = re.compile(
    r'(?m)^(?P<indent>[ \t]*)#\[cfg\((?P<cfg>[^\n]+)\)\]'
    r'(?P<attrs>\n(?:[ \t]*#\[[^\n]+\]\n)*)'
    r'[ \t]*(?:pub(?:\([^\n]+\))?[ \t]+)?mod[ \t]+(?P<name>\w+)[ \t]*[;{]'
)


def select(name, original):
    gates = SHARDS.gates(original)
    edits = []
    # Requested regressions run in the library harness regardless of their shard.
    for gate in gates:
        keep = ((name, gate['name']) in KEEP
                or (name == 'runtime/tests/mod.rs' and gate['name'] == 'input_clipboard')
                or name == 'tooling/interaction/tests.rs')
        if keep:
            edits.append((gate['start'], original[gate['start']:gate['end']], gate['payload']))
        else:
            start = original.index('#[cfg(test)]', gate['start'])
            edits.append((start, '#[cfg(test)]', '#[cfg(any())]'))
    for match in MODULE.finditer(original):
        if any(g['start'] <= match.start() < g['end'] for g in gates):
            continue
        cfg = match['cfg'].strip()
        test_only = cfg == 'test' or (cfg.startswith('all(') and re.search(r'\btest\b', cfg) and 'not(test)' not in cfg)
        if test_only and (name, match['name']) not in KEEP | SHARDS.SUPPORT:
            start = match.start() + len(match['indent'])
            before = '#[cfg(' + match['cfg'] + ')]'
            assert original[start:start + len(before)] == before
            edits.append((start, before, '#[cfg(any())]'))
    if name == 'runtime/tests/mod.rs':
        for match in re.finditer(r'(?m)^mod (\w+);$', original):
            if any(g['start'] <= match.start() < g['end'] for g in gates):
                continue
            if match[1] != 'input_clipboard':
                edits.append((match.start(), '', '#[cfg(any())]\n'))
    result = original
    for offset, before, after in sorted(edits, reverse=True):
        assert result[offset:offset + len(before)] == before
        result = result[:offset] + after + result[offset + len(before):]
    return result, edits


def self_check():
    sample = '#[cfg(test)]\n#[path = "tests.rs"]\nmod tests {\n    fn body() {}\n}\n'
    assert select('clipboard.rs', sample) == (sample, [])
    changed, edits = select('other.rs', sample)
    assert changed == sample.replace('#[cfg(test)]', '#[cfg(any())]') and len(edits) == 1
    helper = '#[cfg(test)]\npub(crate) fn helper() {}\n'
    assert select('other.rs', helper) == (helper, [])
    linux = '#[cfg(all(test, target_os = "linux"))]\nmod tests;\n'
    assert select('runtime/input/primary.rs', linux) == (linux, [])
    assert select('other.rs', linux)[0] == '#[cfg(any())]\nmod tests;\n'
    modules = 'mod other;\nmod input_clipboard;\nthread_local! {}\n'
    assert select('runtime/tests/mod.rs', modules)[0] == '#[cfg(any())]\n' + modules
    gated = SHARDS.wrap(sample, SHARDS.modules(sample)[0], 'native')
    assert select('clipboard.rs', gated)[0] == sample
    disabled, edits = select('other.rs', gated)
    assert disabled.startswith('#[cfg(any())]\n') and 'test_group!' in disabled
    replayed = gated
    for offset, before, after in sorted(edits, reverse=True):
        assert replayed[offset:offset + len(before)] == before
        replayed = replayed[:offset] + after + replayed[offset + len(before):]
    assert replayed == disabled
    helper = '#[cfg(test)]\npub(crate) mod test_alloc;\n'
    assert select('tooling.rs', helper) == (helper, [])


self_check()
if len(sys.argv) == 2 and sys.argv[1] == '--self-check':
    print('test-module selector self-checks passed')
    raise SystemExit(0)
label = sys.argv[1]
all_tests = sys.argv[2:] == ['--all-tests']
assert not sys.argv[2:] or all_tests, 'unexpected export mode'
assert re.fullmatch(r'[a-z0-9-]+', label), label
assert Path.cwd() == ROOT
subprocess.run(['git', 'diff', '--quiet'], check=True)
tree = sealed_tree(ROOT, os.environ['RUNE_SYNC_TREE'])
destination = WORK / 'focused' / label
destination.mkdir(parents=True, exist_ok=False)
source = destination / 'source'
source.mkdir()
archive = subprocess.Popen(['git', 'archive', tree], stdout=subprocess.PIPE)
try:
    subprocess.run(['tar', '-x', '-C', str(source)], stdin=archive.stdout, check=True)
finally:
    archive.stdout.close()
assert archive.wait() == 0
records, differences = [], []
for path in sorted((source / 'crates/rune/src').rglob('*.rs')):
    name = path.relative_to(source / 'crates/rune/src').as_posix()
    original = path.read_bytes().decode()
    selected, edits = (original, []) if all_tests else select(name, original)
    if not edits:
        continue
    path.write_bytes(selected.encode())
    records.append({'file': str(path.relative_to(source)),
                    'before_sha256': hashlib.sha256(original.encode()).hexdigest(),
                    'after_sha256': hashlib.sha256(selected.encode()).hexdigest(),
                    'registration_edits': edits})
    differences.extend(difflib.unified_diff(original.splitlines(keepends=True), selected.splitlines(keepends=True),
                       fromfile='a/' + str(path.relative_to(source)), tofile='b/' + str(path.relative_to(source))))
assert records or all_tests
changed = {record['file']: record for record in records}
# The public workspace retains upstream's vendor. Native acceptance uses the
# reviewed owned winit; this manifest/lock exception is separate from Rust edits.
winit = tomllib.loads((Path(__file__).parent / 'fixtures/Cargo.toml').read_text())['patch']['crates-io']['winit']
assert re.fullmatch(r'[0-9a-f]{40}', winit['rev'])
manifest = source / 'Cargo.toml'
before_manifest = manifest.read_bytes()
old_patch = 'winit = { path = "vendor/winit" }'
assert manifest.read_text().count(old_patch) == 1
manifest.write_text(manifest.read_text().replace(old_patch,
    'winit = { git = "' + winit['git'] + '", rev = "' + winit['rev'] + '" }'))
lock_evidence = destination / 'lock-resolution'
lock_evidence.mkdir()
(lock_evidence / 'seed.lock').write_bytes((source / 'Cargo.lock').read_bytes())
(source / 'Cargo.lock').write_bytes((WORK / 'provenance/lock/workspace.lock').read_bytes())
command = ['bash', str(Path(__file__).parent / 'cargo.sh'), 'metadata', '--offline', '--format-version', '1',
           '--manifest-path', str(manifest), '--features', 'rune-ref/gpu', '--locked']
(lock_evidence / 'command.json').write_text(json.dumps(command) + '\n')
with (lock_evidence / 'metadata.log').open('w') as log:
    subprocess.run(command, cwd=source, stdout=log, check=True)
(lock_evidence / 'resolved.lock').write_bytes((source / 'Cargo.lock').read_bytes())
(lock_evidence / 'resolution.diff').write_text(''.join(difflib.unified_diff(
    (lock_evidence / 'seed.lock').read_text().splitlines(keepends=True),
    (source / 'Cargo.lock').read_text().splitlines(keepends=True), fromfile='seed.lock', tofile='resolved.lock')))
# cargo.sh prefixes compiler identity before Cargo's JSON.
metadata = json.loads(next(line for line in (lock_evidence / 'metadata.log').read_text().splitlines() if line.startswith('{')))
resolved, = [p for p in metadata['packages'] if p['name'] == 'winit']
assert resolved['source'].endswith('#' + winit['rev']), resolved
build_edits = {'Cargo.toml': manifest.read_bytes(), 'Cargo.lock': (source / 'Cargo.lock').read_bytes()}
file_hashes = {}
for path in sorted(source.rglob('*')):
    if not path.is_file():
        continue
    name = path.relative_to(source).as_posix()
    expected = build_edits.get(name, (ROOT / name).read_bytes())
    if name in changed:
        text = expected.decode()
        for offset, before, after in sorted(changed[name]['registration_edits'], reverse=True):
            assert text[offset:offset + len(before)] == before
            text = text[:offset] + after + text[offset + len(before):]
        expected = text.encode()
    assert path.read_bytes() == expected, name
    file_hashes[name] = hashlib.sha256(expected).hexdigest()
(destination / 'registration-only.patch').write_text(''.join(differences))
(destination / 'identity.json').write_text(json.dumps({'staged_tree': tree, 'source': str(source),
    'keep': sorted(KEEP), 'edits': records, 'files_sha256': file_hashes,
    'all_tests': all_tests, 'owned_winit': resolved, 'build_manifest_before_sha256': hashlib.sha256(before_manifest).hexdigest(),
    'build_exception_files': sorted(build_edits), 'lock_resolution': str(lock_evidence),
    'selector_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    'broad_suite': 'NOT completed by source export; execution receipts required'}, indent=2) + '\n')
print(json.dumps({'tree': tree, 'source': str(source), 'changed_registration_files': len(records),
                  'registration_edits': sum(len(r['registration_edits']) for r in records)}))
