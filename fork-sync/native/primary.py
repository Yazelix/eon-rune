"""Isolated native evidence; requires the machine resource wrapper and a private bus."""
from pathlib import Path
import json, os, re, shlex, subprocess, sys, time
from pointer import Pointer, uints
from owned import clean_env, Processes

W = Path(os.environ["RUNE_SYNC_STATE"])
E = W/'native-primary'/sys.argv[1]
E.mkdir(parents=True, exist_ok=False)
BIN = W/'candidates'
SWAY = os.environ["RUNE_SYNC_SWAY"]
SWAYMSG = SWAY+'msg'
OWNED = Processes()
env = clean_env(os.environ, os.environ['RUNE_SYNC_PRIVATE_ROOT'])
bus = os.environ['DBUS_SESSION_BUS_ADDRESS']
assert bus.startswith('unix:path=' + str(W / 'xdg') + '/'), 'requires the private bus'
env['DBUS_SESSION_BUS_ADDRESS'] = bus
(W/'native-home').mkdir(mode=0o700, exist_ok=True)
env.update(HOME=str(W/'native-home'), FONTCONFIG_FILE='/etc/fonts/fonts.conf')
for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'SWAYSOCK', 'I3SOCK', 'AT_SPI_BUS_ADDRESS', 'XDG_ACTIVATION_TOKEN', 'DESKTOP_STARTUP_ID'):
    env.pop(key, None)
env.update(XDG_RUNTIME_DIR=str(W/'xdg'), WLR_BACKENDS='headless', WLR_LIBINPUT_NO_DEVICES='1',
           WLR_RENDERER='vulkan', XDG_CURRENT_DESKTOP='sway', XDG_SESSION_DESKTOP='sway',
           VK_ICD_FILENAMES=str(W/'intel_abs_icd.json'),
           VK_DRIVER_FILES=str(W/'intel_abs_icd.json'),
           LD_LIBRARY_PATH=str(W/'syslibs'), WLR_RENDER_DRM_DEVICE=os.environ['RUNE_SYNC_RENDER_NODE'])
(W/'xdg').mkdir(mode=0o700, exist_ok=True)
(E/'sway.conf').write_text('xwayland disable\noutput HEADLESS-1 mode 2560x720 scale 1\ndefault_border none\nfocus_follows_mouse no\nseat seat0 fallback true\n')
for kind in ('CACHE', 'CONFIG', 'DATA', 'STATE'):
    folder = W / ('native-' + kind.lower())
    folder.mkdir(mode=0o700, exist_ok=True)
    env['XDG_' + kind + '_HOME'] = str(folder)
assert (W/'native-tools/inject').is_file(), 'build injector before native runs'
def start(args, name, extra=None):
    p = OWNED.start(list(map(str,args)), env=env | (extra or {}), stdout=(E/f'{name}.log').open('w'), stderr=subprocess.STDOUT)
    (E/f'{name}.pid').write_text(str(p.pid))
    return p
def stop(p):
    OWNED.stop(p)
def msg(command):
    args = ['-t',command] if command.startswith('get_') else [command]
    result = OWNED.run([SWAYMSG,'-s',env['SWAYSOCK'],*args],env=env,text=True,capture_output=True,check=True,timeout=5).stdout
    with (E/'ipc.log').open('a') as log: log.write(command+' '+result+'\n')
    if not command.startswith('get_'): assert all(i.get('success') for i in json.loads(result)), result
    return json.loads(result)
def shot(name): OWNED.run(['grim',str(E/name)],env=env,check=True,timeout=5)
def inject(commands):
    token=str(time.time_ns())
    with os.fdopen(os.open(input_fifo,os.O_WRONLY|os.O_NONBLOCK),'w') as out: out.write(commands.replace(';','\n')+f'\nsync {token}\n')
    for _ in range(200):
        assert keyboard.poll() is None,'fixed-keymap injector exited'
        if f'sync {token}\n' in (E/'keyboard.log').read_text():
            time.sleep(.15); return
        time.sleep(.02)
    raise AssertionError('fixed-keymap injector did not acknowledge keys')
def key(*args):
    assert len(args)==2 and args[0]=='-k',args
    inject('tap '+str({'F12':88,'BackSpace':14}[args[1]]))
def chord(letter): inject(f'down 29;tap '+str({'a':30,'c':46,'v':47}[letter])+';up 29')
def click(x,y,button=0x110): pointer.click(x,y,button); time.sleep(.4)
def windows(node):
    yield node
    for key_ in ('nodes','floating_nodes'):
        for child in node.get(key_,[]): yield from windows(child)
def mapped(p):
    for _ in range(150):
        assert p.poll() is None, 'native client exited'
        found = [n for n in windows(msg('get_tree')) if n.get('pid') == p.pid]
        if found: return found[0]
        time.sleep(.1)
    raise AssertionError('native window did not map')
def observe(name, **data):
    data.update(item=name, utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()))
    with (E/'observations.jsonl').open('a') as f: f.write(json.dumps(data,ensure_ascii=False)+'\n')
    print(json.dumps(data,ensure_ascii=False),flush=True)
def copy(value, primary=False):
    start(['wl-copy','--foreground',*(['--primary'] if primary else []),'--',value],f'copy-{len(OWNED.started)}')
    time.sleep(.4)
def paste(primary=False):
    p = OWNED.run(['wl-paste','--no-newline',*(['--primary'] if primary else [])],env=env,capture_output=True,text=True,timeout=8)
    assert p.returncode == 0, p.stderr
    return p.stdout

def probe_states(value):
    return [json.loads(item) for item in re.findall(r'PRIMARY probe (\{[^\n]*\})', value)]
assert probe_states('noise\nPRIMARY probe {"field":"café 日本語","middle":1}\n')[0]['field'] == 'café 日本語'
assert probe_states('PRIMARY probe incomplete') == []

try:
    sway = start([SWAY,'-c',E/'sway.conf'],'sway')
    for _ in range(100):
        sock = W/f'xdg/sway-ipc.{os.getuid()}.{sway.pid}.sock'
        displays = [p for p in (W/'xdg').glob('wayland-*') if p.is_socket()]
        if sock.is_socket() and displays: break
        assert sway.poll() is None, 'sway failed'
        time.sleep(.1)
    assert sock.is_socket() and displays
    env.update(SWAYSOCK=str(sock),WAYLAND_DISPLAY=displays[0].name)
    (E/'session-env.json').write_text(json.dumps({k:env[k] for k in ('XDG_RUNTIME_DIR','WAYLAND_DISPLAY','SWAYSOCK','DBUS_SESSION_BUS_ADDRESS')},indent=2))
    (E/'session-env.json').chmod(0o600)
    env['VK_ICD_FILENAMES'] = env['VK_DRIVER_FILES'] = str(W/'intel_abs_icd.json')
    input_fifo=E/'input.fifo';os.mkfifo(input_fifo,0o600)
    keyboard = start([W/'native-tools/inject',input_fifo],'keyboard')
    for _ in range(100):
        if 'layout us' in (E/'keyboard.log').read_text(): break
        assert keyboard.poll() is None,'fixed-keymap injector failed'
        time.sleep(.02)
    pointer = Pointer(env)
    msg('output HEADLESS-1 mode 2560x720 scale 1')
    probe = start([BIN/'primary'],'primary',{'WAYLAND_DEBUG':'1'})
    mapped(probe)
    msg(f'[pid={probe.pid}] floating enable')
    msg(f'[pid={probe.pid}] resize set 900 520')
    msg(f'[pid={probe.pid}] move position 40 40')
    msg(f'[pid={probe.pid}] focus')
    time.sleep(2)
    def rect(name):
        values = re.findall(r'PRIMARY layout '+re.escape(name)+r' (\[[^\n]+\])',(E/'primary.log').read_text())
        assert values, name
        return json.loads(values[-1])
    origin = msg('get_tree')
    window = next(n for n in windows(origin) if n.get('pid')==probe.pid)
    origin = window['rect']['x'],window['rect']['y']
    def point(name, end=False):
        x,y,w,h=rect(name)
        return origin[0]+x+(w-10 if end else 10),origin[1]+y+min(h/2,14)
    def dump():
        count = len(probe_states((E/'primary.log').read_text()))
        key('-k','F12')
        deadline = time.monotonic() + 4
        while time.monotonic() < deadline:
            states = probe_states((E/'primary.log').read_text())
            if len(states) > count:
                return states[-1]
            assert probe.poll() is None, 'probe exited before F12 reply'
            time.sleep(.02)
        raise AssertionError('fresh F12 state reply missing')
    copy('REGULAR_SENTINEL café')
    copy('PRIMARY_SENTINEL',True)
    click(*point('field')); chord('a')
    selected = paste(True)
    observe('primary-export-field',text=selected,regular=paste())
    assert selected == 'Rune → Wayland: café 日本語'
    assert paste() == 'REGULAR_SENTINEL café'
    copy('OTHER_OWNER',True); time.sleep(1)
    assert paste(True)=='OTHER_OWNER'
    observe('primary-owner-not-reclaimed',text=paste(True))
    key('-k','BackSpace')
    copy('Wayland → Rune primary: café 日本語',True)
    click(*point('field',True),button=0x112)
    state=dump()
    observe('primary-import-field',**state,regular=paste())
    assert state['field']=='Wayland → Rune primary: café 日本語'
    assert paste()=='REGULAR_SENTINEL café'
    shot('primary-field.png')
    chord('a'); chord('c')
    assert paste()=='Wayland → Rune primary: café 日本語'
    observe('regular-roundtrip',text=paste())
    copy('SECURE_SENTINEL',True)
    click(*point('secure')); chord('a')
    assert paste(True)=='SECURE_SENTINEL'
    observe('secure-no-export',text=paste(True))
    click(*point('export-terminal'))
    assert paste(True)=='Terminal → Wayland: 日本語'
    observe('primary-export-terminal',text=paste(True))
    click(*point('field'))
    copy('Wayland → terminal: 日本語',True)
    click(*point('terminal'),button=0x112)
    state=dump()
    observe('primary-import-terminal',**state)
    assert state['terminal']=='Wayland → terminal: 日本語' and state['middle']==1
    assert state['field']=='Wayland → Rune primary: café 日本語'
    x,y=point('read-only')
    pointer.move(x-9,y)
    pointer.send(5,2,uints(int(time.monotonic()*1000)&0xffffffff,0x110,1));pointer.send(5,4);pointer.sync()
    time.sleep(.2); pointer.move(x+500,y); time.sleep(.4)
    pointer.send(5,2,uints(int(time.monotonic()*1000)&0xffffffff,0x110,0));pointer.send(5,4);pointer.sync()
    time.sleep(.4)
    observe('primary-export-read-only',text=paste(True))
    assert paste(True)=='Read-only selection café 日本語'
    shot('primary-terminal-and-selection.png')
    identity_node = next(n for n in windows(msg('get_tree')) if n.get('pid') == probe.pid)
    observe('app-id', pid=probe.pid, app_id=identity_node.get('app_id'))
    assert identity_node.get('app_id') == 'org.yazelix.rune-stack-proof'
    click(*point('field')); chord('a'); key('-k','BackSpace')
    inject('map us intl;tap 40;tap 18;tap 57;tap 40;tap 57')
    state = dump()
    observe('dead-key-space', **state)
    assert state['field'] == "é '", state
    chord('a'); key('-k','BackSpace'); inject('tap 57')
    state = dump()
    observe('ordinary-space', **state)
    assert state['field'] == ' ', state
    click(*point('terminal')); pointer.move(2559,719); time.sleep(2)
    def ticks():
        fields = Path(f'/proc/{probe.pid}/stat').read_text().rsplit(') ',1)[1].split()
        return int(fields[11]) + int(fields[12])
    before = len(re.findall(r'wl_surface#\d+\.commit\(', (E/'primary.log').read_text()))
    first = ticks(); time.sleep(10); last = ticks()
    after = len(re.findall(r'wl_surface#\d+\.commit\(', (E/'primary.log').read_text()))
    observe('idle-native-surface', seconds=10, new_surface_commits=after-before, cpu_ticks=last-first)
    assert after == before, 'native surface keeps committing while resting'
    assert last == first, 'native client used CPU while resting'
    shot('primary-dead-key.png')
    stop(probe)
    stop(keyboard); pointer.close()
    (E/'PASS').write_text('Real primary, secure isolation, app_id, dead-key/Space and resting surface checks passed.\n')
finally:
    stopped=OWNED.close()
    time.sleep(1)
    mounts=[l for l in Path('/proc/mounts').read_text().splitlines() if str(W) in l]
    remaining=[p.pid for p in OWNED.started if p.poll() is None]
    (E/'cleanup.json').write_text(json.dumps({'stopped_helpers':stopped,'remaining':remaining,'mounts':mounts},indent=2))
    assert not remaining and not mounts, 'private primary cleanup incomplete'
