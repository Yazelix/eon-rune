"""Exercise the accepted native regression at three actual Wayland scales."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from native_session import session
from reproduce import W, sway, OWNED

output = W / 'native-self-resize' / sys.argv[1]
output.mkdir(parents=True, exist_ok=False)
artifacts = []
for line in (W / 'logs/self-resize-build.log').read_text().splitlines():
    if not line.startswith('{'):
        continue
    item = json.loads(line)
    if item.get('reason') == 'compiler-artifact' and item.get('target', {}).get('name') == 'self_resize' and item.get('executable'):
        artifacts.append(Path(item['executable']))
assert len(artifacts) == 1, artifacts
binary = artifacts[0]
results = []
with session(output) as env:
    env['RUNE_GUI_TESTS'] = '1'
    for scale in (1, 1.2, 1.25):
        assert all(r.get('success') for r in sway(env, f'output HEADLESS-1 resolution 2400x1350 scale {scale}'))
        # Each request starts in a fresh client, without a configure from the preceding resize.
        for case in ('shrink', 'grow'):
            path = output / f'scale-{scale}-{case}.log'
            case_env = env | {'RUNE_SELF_RESIZE_CASE': case, 'RUNE_SELF_RESIZE_SCALE': str(scale)}
            with path.open('wb') as log:
                run = OWNED.run([str(binary)], env=case_env, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT, timeout=30)
            text = path.read_text()
            result = {'scale': scale, 'case': case, 'exit': run.returncode,
                      'passed': run.returncode == 0 and 'self_resize: ok' in text and 'skipped' not in text}
            results.append(result)
            print(json.dumps(result), flush=True)
            assert result['passed'], text
(output / 'result.json').write_text(json.dumps({'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'results': results}, indent=2) + '\n')
