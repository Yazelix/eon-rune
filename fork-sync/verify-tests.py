"""Require the known regression names; count drift alone is informational."""
import json
from pathlib import Path
import re
import sys

group, log = sys.argv[1:]
required = json.loads((Path(__file__).parent / 'required-tests.json').read_text())[group]
passed = set(re.findall(r'^test (\S+) \.\.\. ok$', Path(log).read_text(), re.M))
assert set(required) <= passed, 'missing passing regressions: ' + repr(set(required) - passed)
print(group, 'required', len(required), 'passed', len(passed), 'extra', sorted(passed - set(required)))
