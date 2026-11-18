"""Require the known regression names; count drift alone is informational."""
import json
from pathlib import Path
import re
import sys

group, log = sys.argv[1:]
required = json.loads((Path(__file__).parent / 'required-tests.json').read_text())[group]
text = Path(log).read_text()
passed = set(re.findall(r'^test (\S+) \.\.\. ok$', text, re.M))
summary = re.findall(r'^test result: ok\. (\d+) passed; 0 failed; \d+ ignored; \d+ measured; \d+ filtered out; finished in ', text, re.M)
assert len(summary) == 1 and int(summary[0]) == len(passed), 'missing or incomplete final libtest verdict'
assert set(required) <= passed, 'missing passing regressions: ' + repr(set(required) - passed)
print(group, 'required', len(required), 'passed', len(passed), 'extra', sorted(passed - set(required)))
