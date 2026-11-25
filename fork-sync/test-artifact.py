"""Return the single executable belonging to a Cargo test target."""

import json
from pathlib import Path
import sys

name, log = sys.argv[1:]
artifacts = [json.loads(line) for line in Path(log).read_text().splitlines() if line.startswith('{')]
exe, = [item['executable'] for item in artifacts
        if item.get('reason') == 'compiler-artifact'
        and item['target']['name'] == name and item.get('executable')]
print(Path(exe).resolve(strict=True))
