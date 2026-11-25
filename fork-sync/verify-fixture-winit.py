"""Verify both resolved hosted fixture graphs use Venus's Wayland decoration policy."""
import json
from pathlib import Path
import sys

for name in sys.argv[1:]:
    path = Path(name)
    # The guarded Cargo wrapper prints its toolchain and lock identities first.
    line, = (line for line in path.read_text().splitlines() if line.startswith('{"packages":'))
    metadata = json.loads(line)
    winit, = (package['id'] for package in metadata['packages'] if package['name'] == 'winit')
    node, = (node for node in metadata['resolve']['nodes'] if node['id'] == winit)
    features = set(node['features'])
    assert {'wayland', 'wayland-dlopen'} <= features, f'{path.name}: missing Wayland features'
    assert not any(feature.startswith('wayland-csd-') for feature in features), f'{path.name}: winit enables Adwaita CSD'
    assert not any(package['name'] == 'sctk-adwaita' for package in metadata['packages']), f'{path.name}: Adwaita is in the fixture graph'
    print(f'{path.name}: winit has Wayland support with no Adwaita CSD')
