"""Export the sealed source and lock for both focused and broad Rune checks."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

from native.owned import sealed_tree

state = Path(os.environ['RUNE_SYNC_STATE'])
root = Path(os.environ['RUNE_SYNC_CANDIDATE'])
tree = sealed_tree(root, os.environ['RUNE_SYNC_TREE'])
assert sys.argv[1:] == ['all', '--all-tests'], 'expected the exact, complete source export'
destination = state / 'focused/all'
source = destination / 'source'
source.mkdir(parents=True, exist_ok=False)
subprocess.run(['git', '-C', str(root), 'diff', '--quiet', tree, '--'], check=True)
with subprocess.Popen(['git', '-C', str(root), 'archive', tree], stdout=subprocess.PIPE) as archive:
    subprocess.run(['tar', '-x', '-C', str(source)], stdin=archive.stdout, check=True)
    archive.stdout.close()
    assert archive.wait() == 0
file_hashes = {}
for path in sorted(source.rglob('*')):
    if path.is_file():
        name = path.relative_to(source).as_posix()
        data = path.read_bytes()
        assert data == (root / name).read_bytes(), name
        file_hashes[name] = hashlib.sha256(data).hexdigest()
(destination / 'identity.json').write_text(json.dumps({
    'staged_tree': tree, 'source': str(source), 'files_sha256': file_hashes,
    'broad_suite': 'source and lock sealed; execution receipts required',
}, indent=2) + '\n')
print('exported sealed test source:', tree, len(file_hashes), 'files')
