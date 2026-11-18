"""Materialize immutable fixture inputs outside the checkout; never edit Eon."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from host_inputs import materialize as materialize_host_inputs

tools = Path(__file__).resolve().parent
state = Path(os.environ['RUNE_SYNC_STATE'])
candidate = Path(os.environ['RUNE_SYNC_CANDIDATE'])
upstream = sys.argv[1]
recipe = json.loads((tools / 'fixture-source.json').read_text())
source_repo = os.environ['RUNE_SYNC_EON_SOURCE']
state.mkdir()
for folder in ('logs', 'candidates', 'provenance', 'xdg', 'xdg-gpu', 'native-home'):
    (state / folder).mkdir(mode=0o700)
shutil.copytree(tools / 'native', state / 'native-tools')
shutil.copytree(tools / 'fixtures', state / 'fixtures')
fixtures = state / 'fixtures'
for entry in recipe['entries']:
    data = subprocess.check_output(['git', '-C', source_repo, 'show', recipe['revision'] + ':' + entry['source']])
    assert hashlib.sha256(data).hexdigest() == entry['sha256'], entry['source']
    target = fixtures / entry['destination']
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
subprocess.run(['git', 'apply', '--unidiff-zero', '--check', str(tools / 'fixtures/hosted.patch')], cwd=fixtures, check=True)
subprocess.run(['git', 'apply', '--unidiff-zero', str(tools / 'fixtures/hosted.patch')], cwd=fixtures, check=True)
for entry in recipe['entries']:
    assert hashlib.sha256((fixtures / entry['destination']).read_bytes()).hexdigest() == entry['result_sha256']
assert hashlib.sha256((fixtures / 'venus/examples/eon_transitions.rs').read_bytes()).hexdigest() == '2e1ef79542c7498ab99181a2db9737f1737adf3f66f25742c16ec533c47fa361'
for manifest in fixtures.rglob('Cargo.toml'):
    manifest.write_text(manifest.read_text().replace('@RUNE@', str(candidate)))
# Baseline is the new upstream target, not a prior accepted binary/source export.
baseline = state / 'upstream-source'
baseline.mkdir()
with subprocess.Popen(['git', '-C', str(candidate), 'archive', upstream], stdout=subprocess.PIPE) as archive:
    subprocess.run(['tar', '-x', '-C', str(baseline)], stdin=archive.stdout, check=True)
    archive.stdout.close()
    assert archive.wait() == 0
shutil.copytree(fixtures, state / 'baseline-fixtures')
for manifest in (state / 'baseline-fixtures').rglob('Cargo.toml'):
    manifest.write_text(manifest.read_text().replace(str(candidate), str(baseline)))
libraries = json.loads(Path(os.environ['RUNE_SYNC_LIBRARIES']).read_text())
materialize_host_inputs(state, os.environ['RUNE_SYNC_INTEL_DRIVER'], libraries)
(state / 'native-dbus.conf').write_text('''<busconfig>
  <type>session</type><listen>unix:tmpdir=''' + str(state / 'xdg') + '''</listen>
  <auth>EXTERNAL</auth><policy context="default">
    <allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/>
  </policy>
</busconfig>
''')
(state / 'sway.conf').write_text('xwayland disable\noutput HEADLESS-1 resolution 1920x1080 position 0 0\ndefault_border none\nfocus_follows_mouse no\n')
(state / 'sway-up.sh').write_text('''#!/usr/bin/env bash
set -euo pipefail
unset WAYLAND_DISPLAY DISPLAY SWAYSOCK I3SOCK
export WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=vulkan
export VK_ICD_FILENAMES="$RUNE_SYNC_STATE/intel_abs_icd.json"
export VK_DRIVER_FILES="$VK_ICD_FILENAMES" LD_LIBRARY_PATH="$RUNE_SYNC_STATE/syslibs"
export WLR_RENDER_DRM_DEVICE="${RUNE_SYNC_RENDER_NODE:?}"
exec "$RUNE_SYNC_SWAY" -c "$RUNE_SYNC_STATE/sway.conf"
''')
(state / 'materialized.json').write_text(json.dumps({'candidate': str(candidate), 'upstream': upstream,
    'fixture_source': recipe}, indent=2) + '\n')
# Geometry checks need terminal content, not a captured maintainer screen.
(state / 'capture.ansi').write_text(''.join(
    f'\x1b[38;2;180;200;220m{i:02d} Rune fork resize fixture: café 日本語\x1b[0m\n'
    for i in range(60)))
print('materialized verified fixture sources:', recipe['revision'])
