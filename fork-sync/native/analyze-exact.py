"""Run the frozen Rune analyzer with a single-use exact fixture allowlist."""
import hashlib
import json
from pathlib import Path
import runpy
import sys


def exact_matcher(entries, reasons):
    remaining = {
        (row['kind'], row['node'], str(row['frame']), row['detail']): {'reason': reasons[row['kind']]}
        for row in entries
    }
    assert len(remaining) == len(entries)

    def allowed(rules, _trace, finding):
        assert not rules, 'the stock substring allowlist is disabled for this fixture'
        key = (finding['kind'], finding['element'], finding['at'], finding['detail'])
        return remaining.pop(key, None)

    return allowed


analyzer, scenario, trace, report = map(Path, sys.argv[1:])
spec = json.loads(Path(__file__).with_name('allowlist.json').read_text())
assert hashlib.sha256(analyzer.read_bytes()).hexdigest() == spec['analyzer_sha256']
entries, reasons = spec['entries'], spec['reasons']
assert len(entries) == 40

# Each control uses the production matcher. The real allowance count starts fresh.
control = exact_matcher(entries, reasons)
sample = entries[0]
finding = dict(kind=sample['kind'], element=sample['node'], at=str(sample['frame']), detail=sample['detail'])
for mutation in (
    dict(kind='hitch'),
    dict(element=finding['element'].split('/', 1)[1]),
    dict(element='#99/' + finding['element']),
    dict(at='929'),
    dict(at=finding['at'] + '→' + finding['at']),
    dict(detail='different geometry'),
):
    assert control([], trace.name, finding | mutation) is None, mutation
assert control([], trace.name, finding)
assert control([], trace.name, finding) is None, 'a second occurrence exceeded its exact allowance'

# Keep the stock findings, output and --fail verdict. Replace only rule matching.
main = runpy.run_path(str(analyzer))['main']
main.__globals__['allowed'] = exact_matcher(entries, reasons)
sys.argv = [str(analyzer), '--script', str(scenario), str(trace), '--json', str(report), '--fail']
main()
