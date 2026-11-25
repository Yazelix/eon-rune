"""Bind freshly built artifacts to the exact candidate and new upstream target."""
import hashlib
import json
import os
from pathlib import Path
import sys
from native.owned import sealed_tree

state = Path(os.environ['RUNE_SYNC_STATE'])
candidate, upstream = sys.argv[1:]
tree = sealed_tree(os.environ['RUNE_SYNC_CANDIDATE'], os.environ['RUNE_SYNC_TREE'])
record = {'candidate': candidate, 'upstream': upstream, 'staged_tree': tree, 'binaries': {}}
for path in sorted((state / 'candidates').iterdir()):
    record['binaries'][path.name] = {'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
(state / 'provenance/native-candidates.json').write_text(json.dumps(record, indent=2) + '\n')
