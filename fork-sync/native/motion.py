"""Run the unchanged accepted no-fade scenario; timing acceptance requires a quiet grant."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import runpy
import subprocess
import time

from native_session import session
from reproduce import W, R, OWNED
from owned import sealed_tree


def accept_result(result, timed):
    assert result['analyzer_exit'] == 0, 'motion audit reported findings or failed'
    assert not timed or result['variant'] != 'consumer' or not result['over_budget'], 'candidate exceeds accepted 120 Hz bound'


arguments = argparse.ArgumentParser()
arguments.add_argument('label')
arguments.add_argument('--quiet', action='store_true')
args = arguments.parse_args()
assert not args.quiet or os.environ.get('HEAVY_QUIET_EXEMPT') == '1'
quiet_end = int(os.environ['RUNE_SYNC_QUIET_END']) if args.quiet else None
output = W / 'native-E' / args.label
output.mkdir(parents=True, exist_ok=False)
identity = json.loads((W / 'provenance/native-candidates.json').read_text())
scenario = W / 'native-tools/eon-transitions.json'
checker = W / 'native-tools/check-transitions.py'
assert hashlib.sha256((W / 'fixtures/venus/examples/eon_transitions.rs').read_bytes()).hexdigest() == '2e1ef79542c7498ab99181a2db9737f1737adf3f66f25742c16ec533c47fa361'
assert sealed_tree(R, os.environ['RUNE_SYNC_TREE']) == identity['staged_tree']
checks = runpy.run_path(str(checker))
schedule = [('upstream', 1), ('consumer', 1), ('upstream', 2), ('consumer', 2)] if args.quiet else [('consumer', 1)]
results = []
with session(output) as env:
    env.pop('RUNE_FRAME_LOG', None)
    for variant, number in schedule:
        name = 'motion-candidate' if variant == 'consumer' else 'motion-upstream'
        binary = W / 'candidates' / name
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == identity['binaries'][name]['sha256']
        prefix = output / f'{variant}-{number}'
        started = datetime.now(timezone.utc).isoformat()
        timeout = 180 if quiet_end is None else min(60, quiet_end - time.time() - 10)
        assert timeout > 0, 'quiet grant has no time left for a run and cleanup'
        with prefix.with_suffix('.log').open('w') as log:
            OWNED.run([str(binary), '--fps', '120', '--frames', '930', '--trace-all', '--script', str(scenario),
                '--trace', str(prefix.with_suffix('.jsonl')), '--tree', str(prefix.with_suffix('.tree.json')),
                '--snap', str(prefix.with_suffix('.png'))], env=env | {'RUNE_MOTION_GATE': str(prefix.with_suffix('.gate.txt'))},
                stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=timeout)
        assert 'panes: shell pane' not in prefix.with_suffix('.log').read_text()
        rows = [json.loads(line) for line in prefix.with_suffix('.jsonl').read_text().splitlines()]
        assert rows[0]['clock'] == 'virtual' and rows[0]['hz'] == 120 and rows[0]['scale'] == 2
        frames = rows[1:]
        witnesses = checks['check'](frames)
        checks['negative_checks'](frames)
        assert quiet_end is None or time.time() < quiet_end - 5, 'quiet grant expired during validation'
        assert len(witnesses) == 9
        over = [index for index in range(60, len(frames)) if frames[index]['quality']['cpu_ms'] > 1000 / 120]
        analyzer = OWNED.run(['python3', '-I', str(Path(__file__).with_name('analyze-exact.py')),
            str(R / 'scripts/motion-audit/analyze.py'), str(scenario), str(prefix.with_suffix('.jsonl')),
            str(prefix.with_suffix('.analyze.json'))], env=env, capture_output=True, text=True, check=False,
            timeout=60 if quiet_end is None else max(0, quiet_end - time.time() - 5))
        prefix.with_suffix('.analyze.txt').write_text(analyzer.stdout + analyzer.stderr)
        result = {'variant': variant, 'number': number, 'started': started, 'ended': datetime.now(timezone.utc).isoformat(),
                  'binary_sha256': identity['binaries'][name]['sha256'], 'scenario_sha256': hashlib.sha256(scenario.read_bytes()).hexdigest(),
                  'checker_sha256': hashlib.sha256(checker.read_bytes()).hexdigest(), 'witnesses': witnesses, 'negative_witnesses': 36,
                  'measured_frames': len(frames) - 60, 'max_cpu_ms': max(frame['quality']['cpu_ms'] for frame in frames[60:]),
                  'over_budget': over, 'analyzer_exit': analyzer.returncode,
                  'timing_acceptance': args.quiet and analyzer.returncode == 0 and (variant != 'consumer' or not over)}
        results.append(result)
        (output / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
        print(json.dumps({key: result[key] for key in ('variant', 'number', 'max_cpu_ms', 'over_budget', 'timing_acceptance')}), flush=True)
        accept_result(result, args.quiet)
if args.quiet:
    candidates = [result for result in results if result['variant'] == 'consumer']
    assert len(candidates) == 2 and all(not result['over_budget'] for result in candidates), 'candidate exceeds accepted 120 Hz bound'
