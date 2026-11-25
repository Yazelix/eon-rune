"""Reject any change to the recorded disposable test-source copy."""
import hashlib
import json
import os
from pathlib import Path
import sys

source = Path(sys.argv[1])
assert source.parent.parent == Path(os.environ['RUNE_SYNC_STATE']) / 'focused'
assert source.name == 'source'
identity = json.loads((source.parent / 'identity.json').read_text())
assert str(source) == identity['source']
actual = {p.relative_to(source).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
          for p in source.rglob('*') if p.is_file()}
assert actual == identity['files_sha256'], 'focused source differs from recorded identity'
print('verified focused copy:', identity['staged_tree'], len(actual), 'files;', identity['broad_suite'])
