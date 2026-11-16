"""Build the supplied fixed-keymap client against installed Wayland libraries."""
from pathlib import Path
import shlex
import subprocess

root = Path(__file__).parent
flags = shlex.split(subprocess.check_output(
    ['pkg-config', '--cflags', '--libs', 'wayland-client', 'xkbcommon'], text=True))
subprocess.run(['cc', '-O2', '-Wall', '-o', str(root / 'inject'),
    *[str(root / name) for name in ('inject.c', 'virtual-keyboard-protocol.c', 'virtual-pointer-protocol.c')],
    *flags], check=True)
assert (root / 'inject').is_file()
