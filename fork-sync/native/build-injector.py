"""Build the supplied fixed-keymap client against installed Wayland libraries."""
from pathlib import Path
import os
import shlex
import subprocess
from owned import check_resources

assert os.environ.get('RUNE_SYNC_BOUNDED') == '1', 'run through the bounded heavy launcher'
assert os.environ.get('RUNE_SYNC_BUILD_HELD') == '1', 'requires the shared Rune build lock'
check_resources()
root = Path(__file__).parent
flags = shlex.split(subprocess.check_output(
    ['pkg-config', '--cflags', '--libs', 'wayland-client', 'xkbcommon'], text=True))
subprocess.run(['cc', '-O2', '-Wall', '-o', str(root / 'inject'),
    *[str(root / name) for name in ('inject.c', 'virtual-keyboard-protocol.c', 'virtual-pointer-protocol.c')],
    *flags], check=True)
assert (root / 'inject').is_file()
