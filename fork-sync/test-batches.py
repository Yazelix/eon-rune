"""List disjoint workspace test invocations from Cargo's target inventory."""

import json
from pathlib import Path
import sys


def batches(workspace):
    members = set(workspace['workspace_members'])
    packages = sorted((p for p in workspace['packages'] if p['id'] in members), key=lambda p: p['name'])
    assert {p['id'] for p in packages} == members, 'missing workspace member metadata'
    for package in packages:
        name = package['name']
        if name != 'rune':
            yield name, name, '-', '-'
            continue
        yield 'rune-lib', name, '--lib', '-'
        yield 'rune-doc', name, '--doc', '-'
        for target in package['targets']:
            if not target['test']:
                continue
            kind = target['kind'][0]
            if kind in ('test', 'bin', 'example'):
                yield f'rune-{kind}-{target["name"]}', name, f'--{kind}', target['name']


if __name__ == '__main__':
    log = Path(sys.argv[1]).read_text()
    metadata, = (json.loads(line) for line in log.splitlines() if line.startswith('{"packages":'))
    rows = list(batches(metadata))
    assert len({row[0] for row in rows}) == len(rows), 'duplicate test batch'
    assert {'rune-lib', 'rune-doc', 'rune-test-integration', 'rune-test-unit_native_tests'} <= {row[0] for row in rows}, 'missing Rune test shards'
    for row in rows:
        print(*row, sep='\t')
