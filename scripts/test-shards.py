#!/usr/bin/env python3
"""Check unit-test ownership; strip/restore only mechanical group wrappers."""
import argparse
import json
from pathlib import Path
import re
import sys
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'crates/rune/src'
GROUPS = ('core', 'layout', 'motion', 'input', 'paint', 'selection', 'app', 'native')
SUPPORT = {('runtime.rs', 'tests'), ('tooling/interaction.rs', 'tests'),
           ('window/gpu_failure.rs', 'tests'), ('tooling.rs', 'test_alloc')}
SUPPORT_FILES = {'runtime/tests/mod.rs', 'tooling/interaction/tests.rs', 'window/gpu_failure_tests.rs',
                 'window/gpu_failure_regression_tests.rs', 'tooling/test_alloc.rs'}
GATE = re.compile(r'(?m)^(?P<indent>[ \t]*)#\[cfg\(test\)\]\r?\n[ \t]*test_group!\s*\{\s*(?P<group>\w+)\s*=>\r?\n')
MODULE = re.compile(r'(?:pub(?:\([^\n]+?\))?\s+)?mod\s+(\w+)\s*([;{])')
RAW = re.compile(r'(?:br|cr|r)(#+)?"')
CHAR = re.compile(r"'(?:\\u\{[0-9a-fA-F_]+\}|\\x[0-9a-fA-F]{2}|\\.|[^'\\\n])'")


def mask(source):
    """Preserve offsets, excluding Rust comments and literal contents."""
    out = list(source)
    i = 0
    while i < len(source):
        start = i
        if source.startswith('//', i):
            end = source.find('\n', i)
            i = len(source) if end < 0 else end
        elif source.startswith('/*', i):
            depth = 1
            i += 2
            while depth:
                if i >= len(source):
                    raise ValueError('unterminated block comment')
                if source.startswith('/*', i):
                    depth += 1
                    i += 2
                elif source.startswith('*/', i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
        else:
            raw = RAW.match(source, i)
            char = CHAR.match(source, i)
            if raw:
                end = '"' + (raw[1] or '')
                p = source.find(end, raw.end())
                if p < 0:
                    raise ValueError('unterminated raw string')
                i = p + len(end)
            elif char:
                i = char.end()
            elif source[i] == '"':
                i += 1
                while i < len(source):
                    if source[i] == '\\':
                        i += 2
                    elif source[i] == '"':
                        i += 1
                        break
                    else:
                        i += 1
            else:
                i += 1
                continue
        for j in range(start, i):
            if out[j] != '\n':
                out[j] = ' '
    return ''.join(out)


def closing(masked, start, left='{', right='}'):
    assert masked[start] == left
    depth = 1
    end = start + 1
    while depth:
        if end >= len(masked):
            raise ValueError('unclosed delimiter')
        depth += (masked[end] == left) - (masked[end] == right)
        end += 1
    return end


def requires_test(cfg):
    cfg = cfg.strip()
    if cfg == 'test':
        return True
    for op, combine in [('all', any), ('any', all)]:
        if cfg.startswith(op + '(') and cfg.endswith(')'):
            args, depth, start = [], 0, len(op) + 1
            for i in range(start, len(cfg) - 1):
                depth += (cfg[i] == '(') - (cfg[i] == ')')
                if cfg[i] == ',' and depth == 0:
                    args.append(cfg[start:i])
                    start = i + 1
            args.append(cfg[start:-1])
            return combine(requires_test(arg) for arg in args if arg.strip())
    return False


def modules(source):
    masked = mask(source)
    found = []
    for match in re.finditer(r'(?m)^[ \t]*#\[cfg\(', masked):
        attr = masked.index('[', match.start())
        end = closing(masked, attr, '[', ']')
        if not requires_test(masked[attr + 5:end - 2]):
            continue
        pos = end
        while True:
            while pos < len(masked) and masked[pos].isspace():
                pos += 1
            if masked.startswith('#[', pos):
                pos = closing(masked, pos + 1, '[', ']')
            else:
                break
        item = MODULE.match(masked, pos)
        if item:
            end = item.end() if item[2] == ';' else closing(masked, item.end() - 1)
            found.append({'start':match.start(), 'end':end, 'name':item[1],
                          'kind':'mod', 'inline':item[2] == '{', 'head_end':item.end()})
    return found


def includes(source):
    pattern = r'(?m)^[ \t]*#\[cfg\(test\)\]\r?\n[ \t]*include!\("([^"]+)"\);'
    return [{'start':m.start(), 'end':m.end(), 'kind':'include', 'name':m[1]}
            for m in re.finditer(pattern, source)]


def attribute_start(masked, start, attrs):
    for a, b in reversed(attrs):
        if b <= start and not masked[b:start].strip():
            start = a
    line = masked.rfind('\n', 0, start) + 1
    return line if not masked[line:start].strip() else start


def functions(source, skip=()):
    masked = mask(source)
    attrs = [(m.start(), closing(masked, m.start() + 1, '[', ']'))
             for m in re.finditer(r'#\[', masked)]
    found = []
    for match in re.finditer(r'#\[test\]', masked):
        if any(s['start'] <= match.start() < s['end'] for s in skip):
            continue
        start, pos = attribute_start(masked, match.start(), attrs), match.end()
        while True:
            while pos < len(masked) and masked[pos].isspace():
                pos += 1
            if masked.startswith('#[', pos):
                pos = closing(masked, pos + 1, '[', ']')
            else:
                break
        item = re.match(r'(?:pub(?:\([^\n]+?\))?\s+)?fn\s+(\w+)\s*\(', masked[pos:])
        if not item:
            raise ValueError('unsupported #[test] function syntax')
        brace = masked.index('{', pos + item.end())
        found.append({'start':start, 'end':closing(masked, brace), 'name':item[1], 'kind':'fn'})
    return found


def gates(source):
    masked, found = mask(source), []
    for match in GATE.finditer(source):
        if masked[match.start():match.start() + len(match['indent']) + 1].strip() != '#':
            continue
        assert match['group'] in GROUPS, 'unknown group: ' + match['group']
        end = closing(masked, source.index('{', match.start()))
        line = source.rfind('\n', match.end(), end) + 1
        assert source[line:end - 1] == match['indent'], 'non-mechanical wrapper end'
        payload = source[match.end():line]
        assert payload.endswith('\n')
        payload = payload[:-2] if payload.endswith('\r\n') else payload[:-1]
        item = re.search(r'\b(mod|fn)\s+(\w+)', mask(payload))
        included = includes(payload)
        assert item or len(included) == 1, 'wrapper has no test item'
        kind, name = (item[1], item[2]) if item else ('include', included[0]['name'])
        found.append({'start':match.start(), 'end':end, 'payload':payload,
                      'group':match['group'], 'name':name, 'kind':kind})
    for a, b in zip(found, found[1:]):
        assert a['end'] <= b['start'], 'nested group wrappers'
    return found


def wrap(source, item, group):
    assert group in GROUPS
    a, b = item['start'], item['end']
    indent = re.match(r'[ \t]*', source[a:])[0]
    first_line = source.find('\n')
    newline = '\r\n' if first_line > 0 and source[first_line - 1] == '\r' else '\n'
    return source[:a] + indent + '#[cfg(test)]' + newline + indent + 'test_group! { ' + group + ' =>' + newline + source[a:b] + newline + indent + '}' + source[b:]


def strip(source):
    selected = gates(source)
    for gate in reversed(selected):
        source = source[:gate['start']] + gate['payload'] + source[gate['end']:]
    return source, [{k:g[k] for k in ('group','name','kind')} for g in selected]


def restore(source, records):
    keys = [(r['kind'], r['name']) for r in records]
    assert len(keys) == len(set(keys)), 'duplicate ownership'
    candidates = modules(source) + includes(source)
    candidates += functions(source, candidates)
    # runtime/tests/mod.rs also has un-attributed child module registrations.
    masked = mask(source)
    attrs = [(m.start(), closing(masked, m.start() + 1, '[', ']'))
             for m in re.finditer(r'#\[', masked)]
    for candidate in candidates:
        if candidate['kind'] == 'mod':
            candidate['start'] = attribute_start(masked, candidate['start'], attrs)
    for match in re.finditer(r'(?m)^[ \t]*mod\s+(\w+)\s*;', masked):
        if any(c['kind'] == 'mod' and c['end'] == match.end() for c in candidates):
            continue
        candidates.append({'start':attribute_start(masked, match.start(), attrs), 'end':match.end(), 'name':match[1], 'kind':'mod'})
    edits = []
    for record in records:
        matches = {(c['start'], c['end']):c for c in candidates
                   if (c['kind'], c['name']) == (record['kind'], record['name'])}
        assert len(matches) == 1, 'new, moved or ambiguous registration: ' + record['name']
        edits.append((next(iter(matches.values())), record['group']))
    for item, group in sorted(edits, key=lambda e:e[0]['start'], reverse=True):
        source = wrap(source, item, group)
    return source


def test_file(path):
    return path.name in {'tests.rs', 'mod_tests.rs'} or path.name.endswith('_tests.rs') or 'tests' in path.parts


def check(root=SOURCE, *, targets=True):
    if targets:
        manifest = tomllib.loads((root.parent / 'Cargo.toml').read_text(encoding='utf-8'))
        expected = {'rune' if g == 'core' else 'unit_' + g + '_tests':g for g in GROUPS}
        actual = {t['name'] for t in manifest.get('test', []) if t.get('path') == 'src/lib.rs'}
        assert actual == set(expected) - {'rune'}, 'missing or extra source-root test target'
        assert manifest.get('lib', {}).get('test', True), 'library test partition disabled'
        for name, group in expected.items():
            source = (root / 'test_shards' / (name + '.rs')).read_text(encoding='utf-8')
            selected = re.search(r'macro_rules!\s+test_group\s*\{\s*\((\w+)\s*=>', source)
            assert selected and selected[1] == group, 'wrong group in ' + name
    errors, owners = [], 0
    files = {p:p.read_bytes().decode() for p in root.rglob('*.rs') if p.parent.name != 'test_shards'}
    pure = set()
    # Include explicitly registered test files with nonstandard names, too.
    for path, source in files.items():
        clean, _ = strip(source)
        for item in modules(clean):
            if item['inline']:
                continue
            attrs = clean[item['start']:item['head_end']]
            explicit = re.search(r'#\[path\s*=\s*"([^"]+)"\]', attrs)
            if explicit:
                candidates = [path.parent / explicit[1]]
            else:
                base = path.parent if path.stem in {'lib','mod'} else path.with_suffix('')
                candidates = [base / (item['name'] + '.rs'), base / item['name'] / 'mod.rs']
            pure.update(p.resolve() for p in candidates if p.exists())
    for path, source in files.items():
        name = path.relative_to(root).as_posix()
        selected = gates(source)
        owners += len(selected)
        def covered(item):
            return any(g['start'] <= item['start'] and item['end'] <= g['end'] for g in selected)
        for item in modules(source):
            if not covered(item) and (name, item['name']) not in SUPPORT and (name in SUPPORT_FILES or not (test_file(path) or path.resolve() in pure)):
                errors.append(name + ': unowned module ' + item['name'])
        for item in includes(source):
            if not covered(item):
                errors.append(name + ': unowned include ' + item['name'])
        if name in SUPPORT_FILES:
            for match in re.finditer(r'(?m)^[ \t]*(?:pub(?:\([^\n]+?\))?\s+)?mod\s+(\w+)\s*[;{]', mask(source)):
                if (name, match[1]) == ('window/gpu_failure_tests.rs', 'regressions'):
                    continue
                if not covered({'start':match.start(), 'end':match.end()}):
                    errors.append(name + ': unowned child ' + match[1])
        if name in SUPPORT_FILES or not (test_file(path) or path.resolve() in pure):
            for item in functions(source, selected):
                if not covered(item):
                    errors.append(name + ': unowned function ' + item['name'])
        keys = [(g['kind'], g['name']) for g in selected]
        if len(keys) != len(set(keys)):
            errors.append(name + ': ambiguous registration identity')
    assert not errors, '\n'.join(errors)
    return owners


def self_check():
    sample = '#[cfg(test)]\nmod tests { #[test] fn a() { let _ = r###"}"###; /* { /* } */ } */ } }\n'
    item, = modules(sample)
    changed = wrap(sample, item, 'core')
    clean, records = strip(changed)
    assert clean == sample and restore(clean, records) == changed
    windows = sample.replace('\n', '\r\n')
    grouped = wrap(windows, modules(windows)[0], 'core')
    clean_windows, windows_records = strip(grouped)
    assert clean_windows == windows and restore(clean_windows, windows_records) == grouped
    for external in ['#[cfg(test)]\nmod outside;\n', '#[cfg(test)]\n#[path = "other.rs"]\nmod outside;\n',
                     '#[cfg(target_os = "linux")]\n#[path = "other.rs"]\nmod outside;\n',
                     '#[path = "other.rs"]\n#[cfg(test)]\nmod outside;\n']:
        item = {'start':0, 'end':len(external.rstrip('\n')), 'kind':'mod', 'name':'outside'}
        grouped = wrap(external, item, 'core')
        clean_external, external_records = strip(grouped)
        assert clean_external == external and restore(clean_external, external_records) == grouped
    bare = 'mod child;\n'
    bare_records = [{'kind':'mod', 'name':'child', 'group':'input'}]
    assert strip(restore(bare, bare_records))[0] == bare
    assert requires_test('all(test, target_os =     )')
    assert not requires_test('any(test, target_arch =       )')
    assert not requires_test('all(any(test, feature =    ), unix)')
    for source, saved in [(sample.replace('tests', 'renamed'), records), (sample, records * 2)]:
        try:
            result = restore(source, saved)
            gates(result)
        except (AssertionError, ValueError):
            pass
        else:
            raise AssertionError('accepted missing or duplicate ownership')
    try:
        gates(changed.replace('core =>', 'unknown =>'))
    except AssertionError:
        pass
    else:
        raise AssertionError('accepted unknown group')
    included = '#[cfg(test)]\ninclude!("parser_tests.rs");\n'
    item, = includes(included)
    grouped = wrap(included, item, 'native')
    clean, records = strip(grouped)
    assert clean == included and restore(clean, records) == grouped
    with tempfile.TemporaryDirectory(prefix='rune-shards-') as directory:
        root = Path(directory)
        owner = root / 'owner.rs'
        owner.write_text(sample)
        try:
            check(root, targets=False)
        except AssertionError:
            pass
        else:
            raise AssertionError('accepted an unowned test module')
        owner.write_text(changed)
        assert check(root, targets=False) == 1
        owner.write_text(changed + '\n#[cfg(test)]\nmod added { #[test] fn new_test() {} }\n')
        try:
            check(root, targets=False)
        except AssertionError:
            pass
        else:
            raise AssertionError('accepted a new unowned module')
        owner.write_text(changed, encoding='utf-8')
        shared = root / 'runtime/tests/mod.rs'
        shared.parent.mkdir(parents=True)
        direct = '#[test]\nfn shared_test() {}\n'
        shared.write_text(direct, encoding='utf-8')
        try:
            check(root, targets=False)
        except AssertionError as error:
            assert 'runtime/tests/mod.rs: unowned function shared_test' in str(error)
        else:
            raise AssertionError('accepted an unowned shared test')
        shared.write_text(wrap(direct, functions(direct)[0], 'input'), encoding='utf-8')
        assert check(root, targets=False) == 2
        base = 'fn production() -> u32 { 1 }\n' + sample
        wrapped = wrap(base, modules(base)[0], 'core')
        cleaned, registrations = strip(wrapped)
        incoming = base.replace('/* { /* } */ } */', '/* { /* } */ } */ let added = 1;')
        assert strip(restore(incoming, registrations))[0] == incoming
        paths = [root / n for n in ('ours', 'base', 'theirs')]
        for path, text in zip(paths, [cleaned.replace('{ 1 }', '{ 2 }'), base, incoming.replace('{ 1 }', '{ 3 }')]):
            path.write_text(text)
        merged = subprocess.run(['git', 'merge-file', '-p', *map(str, paths)], stdout=subprocess.PIPE, text=True)
        assert merged.returncode > 0 and '<<<<<<< ' in merged.stdout, 'hid a production conflict'


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', nargs='?', const=SOURCE, type=Path, metavar='SOURCE_ROOT')
    parser.add_argument('--self-check', action='store_true')
    parser.add_argument('--strip', type=Path)
    parser.add_argument('--restore', nargs=2, type=Path, metavar=('RECORD', 'MERGED_SOURCE'))
    args = parser.parse_args()
    self_check()
    if args.strip:
        source, registrations = strip(args.strip.read_bytes().decode())
        print(json.dumps({'source':source, 'registrations':registrations}, indent=2))
    elif args.restore:
        record, source = args.restore
        sys.stdout.buffer.write(restore(source.read_bytes().decode(), json.loads(record.read_text(encoding='utf-8'))['registrations']).encode())
    elif args.check:
        print('unit test ownership:', check(args.check), 'registrations')
    else:
        assert args.self_check, 'choose --check, --self-check, --strip or --restore'
        print('unit test ownership self-checks passed')
