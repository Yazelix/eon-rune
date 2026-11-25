"""Describe installed driver libraries for a private Nix/host Vulkan search path."""
import json
from pathlib import Path
import re
import subprocess
import sys

driver = Path(sys.argv[1]).resolve(strict=True)
cache = subprocess.check_output(['ldconfig', '-p'], text=True)
roots = [driver]
for name in ('libvulkan.so.1', 'libwayland-client.so.0', 'libgbm.so.1', 'libdisplay-info.so.1'):
    found = re.search(r'^\s*' + re.escape(name) + r' .* => (\S+)$', cache, re.M)
    assert found, 'missing host library ' + name
    roots.append(Path(found[1]))
# GBM loads its Mesa provider dynamically, so ldd(libgbm) does not name it.
# shortcut: one Mesa provider per host; take a provider input for multi-version hosts.
providers = sorted(driver.parent.glob('libgallium-*.so'))
assert len(providers) == 1, 'select one installed Mesa GBM provider explicitly'
roots.extend(providers)
libraries = {p.name: str(p) for p in roots[1:]}
for root in roots:
    linked = subprocess.check_output(['ldd', str(root)], text=True)
    assert 'not found' not in linked, linked
    for name, path in re.findall(r'^\s*(\S+) => (/\S+) ', linked, re.M):
        # The native artifact uses Nix glibc; keep that C runtime together.
        if not re.match(r'lib(c|m|pthread|dl|rt)\.so', name):
            libraries[name] = path
print(json.dumps(libraries, indent=2))
