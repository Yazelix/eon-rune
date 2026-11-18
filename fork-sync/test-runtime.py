"""Safety controls with fake inputs/processes; no GPU, external fetch or Rune compilation."""
import hashlib
import ast
from contextlib import nullcontext
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch, Mock, call

import bounded
import host_inputs
import launch
from native import owned

TOOLS = Path(__file__).resolve().parent


class RuntimeControls(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir=sys.argv[1], prefix='runtime-controls-')
        self.root = Path(self.temporary.name)
        private = self.root / 'private'
        private.mkdir()
        self.env = owned.clean_env(os.environ, private)
        self.env['RUNE_SYNC_STATE'] = str(self.root / 'state')
        (self.root / 'state/provenance').mkdir(parents=True)

    def tearDown(self):
        self.temporary.cleanup()

    def git(self, *args, env=None):
        return subprocess.check_output(['git', *args], cwd=self.root, env=env or self.env, text=True).strip()

    def repository(self):
        self.git('init', '-q')
        self.git('config', 'user.name', 'Synthetic control')
        self.git('config', 'user.email', 'control@example.invalid')
        self.git('config', 'commit.gpgsign', 'false')
        (self.root / 'file').write_text('one')
        self.git('add', 'file')
        self.git('commit', '-qm', 'synthetic fixture')

    def test_private_home_blocks_global_hooks_and_fixture_output_paths(self):
        hooks = self.root / 'hooks'
        hooks.mkdir()
        marker = self.root / 'hook-fired'
        hook = hooks / 'pre-commit'
        hook.write_text('#!/bin/sh\nprintf fired > ' + str(marker) + '\n')
        hook.chmod(0o700)
        config = self.root / 'caller-config'
        config.write_text('[core]\n hooksPath = ' + str(hooks) + '\n')
        shared = self.root / 'shared-output'
        shared.write_text('keep')
        dirty = dict(self.env, HOME=str(self.root), GIT_CONFIG_GLOBAL=str(config),
                     TERMGRID_STATS=str(shared), RUNE_FRAME_LOG=str(shared), CARGO_HOME='/caller/cache',
                     RUNE_SYNC_STATE='/unrelated/state')
        safe = owned.clean_env(dirty, Path(self.env['RUNE_SYNC_PRIVATE_ROOT']))
        self.assertEqual(safe['CARGO_HOME'], self.env['CARGO_HOME'])
        self.assertNotIn('TERMGRID_STATS', safe)
        self.assertNotIn('RUNE_FRAME_LOG', safe)
        self.assertNotIn('RUNE_SYNC_STATE', safe)
        self.repository()
        (self.root / 'file').write_text('two')
        self.git('add', 'file', env=safe)
        self.git('commit', '-qm', 'isolated commit', env=safe)
        self.assertFalse(marker.exists())
        fixture = 'import os,pathlib; [pathlib.Path(os.environ[k]).write_text("bad") for k in ("TERMGRID_STATS","RUNE_FRAME_LOG") if k in os.environ]'
        subprocess.run([sys.executable, '-c', fixture], env=safe, check=True)
        self.assertEqual(shared.read_text(), 'keep')
        # The controlled hostile configuration really would fire without isolation.
        (self.root / 'file').write_text('three')
        self.git('add', 'file', env=dirty)
        self.git('commit', '-qm', 'negative control', env=dirty)
        self.assertEqual(marker.read_text(), 'fired')

    def test_offline_cargo_never_contacts_a_missing_git_source(self):
        requests = []
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                requests.append(self.path)
                self.send_error(404)
            def log_message(self, *args):
                pass
        server = HTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever)
        worker.start()
        try:
            (self.root / 'src').mkdir()
            (self.root / 'src/lib.rs').write_text('')
            (self.root / 'Cargo.toml').write_text('[package]\nname="offline-control"\nversion="0.1.0"\nedition="2024"\n[dependencies]\nmissing={git="http://127.0.0.1:' + str(server.server_port) + '/missing.git"}\n')
            (self.root / 'Cargo.lock').write_text('version = 4\n')
            env = dict(self.env, RUNE_SYNC_BOUNDED='1', RUNE_SYNC_BUILD_HELD='1')
            env.pop('RUNE_SYNC_CARGO_SEED', None)
            result = subprocess.run(['bash', str(TOOLS / 'cargo.sh'), 'metadata', '--locked', '--format-version', '1'],
                                    cwd=self.root, env=env, capture_output=True, text=True, timeout=20)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('offline', result.stderr.lower())
            self.assertFalse(requests)
        finally:
            server.shutdown()
            worker.join()
            server.server_close()

    def test_cache_seed_copies_only_sources_and_does_not_change_the_seed(self):
        seed = self.root / 'seed'
        (seed / 'registry/cache').mkdir(parents=True)
        (seed / 'registry/cache/sample.crate').write_bytes(b'public fixture bytes')
        executable = seed / 'registry/cache/synthetic-script'
        executable.write_text('#!/bin/sh\nexit 0\n')
        executable.chmod(0o755)
        objects = seed / 'git/db/example/objects'
        objects.mkdir(parents=True)
        alternate = seed / 'git/checkouts/example/rev/.git/objects/info/alternates'
        alternate.parent.mkdir(parents=True)
        alternate.write_text(str(objects) + '\n')
        (seed / 'config.toml').write_text('synthetic caller configuration')
        (seed / 'credentials.toml').write_text('synthetic credential marker')
        before = {str(p.relative_to(seed)): p.read_bytes() for p in seed.rglob('*') if p.is_file()}
        for path in sorted(seed.rglob('*'), reverse=True):
            path.chmod(path.stat().st_mode & ~0o222)
        seed.chmod(seed.stat().st_mode & ~0o222)
        subprocess.run([sys.executable, str(TOOLS / 'cache.py'), str(seed), self.env['CARGO_HOME']],
                       env=self.env, check=True, capture_output=True)
        cache = Path(self.env['CARGO_HOME'])
        self.assertEqual((cache / 'registry/cache/sample.crate').read_bytes(), b'public fixture bytes')
        self.assertFalse((cache / 'config.toml').exists())
        self.assertFalse((cache / 'credentials.toml').exists())
        self.assertEqual((cache / executable.relative_to(seed)).stat().st_mode & 0o111, 0o111)
        self.assertEqual((cache / alternate.relative_to(seed)).read_text(), str(cache / objects.relative_to(seed)) + '\n')
        self.assertEqual({str(p.relative_to(seed)): p.read_bytes() for p in seed.rglob('*') if p.is_file()}, before)

    def test_cargo_workdir_rejects_ancestor_configuration(self):
        package = self.root / 'package'
        package.mkdir()
        manifest = package / 'Cargo.toml'
        manifest.write_text('[package]\nname="isolated"\nversion="0.1.0"\n')
        self.assertEqual(owned.cargo_dir(manifest), package)
        config = self.root / '.cargo'
        config.mkdir()
        (config / 'config.toml').write_text('synthetic ancestor configuration')
        with self.assertRaises(AssertionError):
            owned.cargo_dir(manifest)

    def test_injector_rejects_bypassing_the_bounded_launcher_and_build_lock(self):
        for missing in ('RUNE_SYNC_BOUNDED', 'RUNE_SYNC_BUILD_HELD'):
            env = dict(self.env, RUNE_SYNC_BOUNDED='1', RUNE_SYNC_BUILD_HELD='1')
            env.pop(missing)
            result = subprocess.run([sys.executable, str(TOOLS / 'native/build-injector.py')],
                                    env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('AssertionError', result.stderr)

    def fake_resources(self, name):
        scope = self.root / name
        scope.mkdir()
        for file, value in [('memory.max', '8589934592'), ('memory.swap.max', '0'),
                            ('memory.peak', '1'), ('memory.events', 'oom 0\noom_kill 0\n')]:
            (scope / file).write_text(value)
        return owned.Resources(scope, scope / 'resource-failure.json')

    def test_resource_failure_fences_the_second_launch_and_retry(self):
        for mode in ('oom', 'oom_kill', 'peak'):
            guard = self.fake_resources(mode)
            owner = owned.Processes()
            with patch.object(owned, 'check_resources', guard.check):
                first = owner.start([sys.executable, '-c', 'pass'], env=self.env)
                first.wait(timeout=5)
                if mode == 'peak':
                    (guard.scope / 'memory.peak').write_text(str(owned.LIMIT + 1))
                else:
                    (guard.scope / 'memory.events').write_text('oom ' + str(int(mode == 'oom')) + '\noom_kill ' + str(int(mode == 'oom_kill')) + '\n')
                forbidden = self.root / (mode + '-second-attempt')
                with self.assertRaises(owned.ResourceViolation):
                    owner.start([sys.executable, '-c', 'from pathlib import Path; Path(' + repr(str(forbidden)) + ').write_text("bad")'], env=self.env)
                self.assertFalse(forbidden.exists())
                self.assertEqual(len(owner.started), 1)
                (guard.scope / 'memory.events').write_text('oom 0\noom_kill 0\n')
                (guard.scope / 'memory.peak').write_text('1')
                with self.assertRaises(owned.ResourceViolation):
                    guard.check()
                resources = guard.scope / 'resources'
                resources.mkdir()
                (resources / 'native-1.json').write_text('{"resource_ok":true,"child_exit":1}')
                self.assertFalse(owned.retryable(guard.scope, 'native'))
            owner.close()

    def test_watch_detects_resource_failure_while_child_is_running(self):
        guard = self.fake_resources('watch')
        child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'], env=self.env, start_new_session=True)
        caught = threading.Event()
        def watch():
            try:
                bounded.watch(child, guard)
            except owned.ResourceViolation:
                caught.set()
        worker = threading.Thread(target=watch)
        worker.start()
        try:
            update = guard.scope / 'events.next'
            update.write_text('oom 1\noom_kill 0\n')
            update.replace(guard.scope / 'memory.events')
            self.assertTrue(caught.wait(2), 'watcher did not detect pressure while the child ran')
            self.assertIsNone(child.poll())
        finally:
            child.terminate()
            child.wait(timeout=5)
            worker.join(timeout=5)
        self.assertFalse(worker.is_alive())

    def test_cleanup_pressure_is_fenced_and_an_optimistic_receipt_is_not_retryable(self):
        for mode in ('oom', 'oom_kill', 'peak'):
            guard = self.fake_resources('rune-sync-final-' + mode)
            label = 'cleanup-' + mode
            env = dict(self.env, RUNE_SYNC_LABEL=label)
            child = Mock(returncode=1)
            child.wait.return_value = 1
            def cleanup(scope, sig):
                if sig == bounded.signal.SIGKILL:
                    if mode == 'peak':
                        (scope / 'memory.peak').write_text(str(owned.LIMIT + 1))
                    else:
                        (scope / 'memory.events').write_text('oom ' + str(int(mode == 'oom')) + '\noom_kill ' + str(int(mode == 'oom_kill')) + '\n')
            with patch.object(bounded, 'Resources', return_value=guard), \
                 patch.object(bounded, 'clean_env', return_value=env), \
                 patch.object(bounded.subprocess, 'Popen', return_value=child), \
                 patch.object(bounded, 'watch'), \
                 patch.object(bounded, 'signal_scope', side_effect=cleanup), \
                 patch.object(bounded.time, 'sleep'):
                self.assertEqual(bounded.main(), 75)
            state = Path(env['RUNE_SYNC_STATE'])
            receipt, = (state / 'resources').glob(label + '-*.json')
            row = json.loads(receipt.read_text())
            self.assertFalse(row['resource_ok'])
            self.assertTrue(guard.marker.exists())
            self.assertFalse((state / 'resource-failure.json').exists())
            row['resource_ok'] = True  # Model the old optimistic receipt without a marker.
            receipt.write_text(json.dumps(row))
            self.assertFalse(owned.retryable(state, label))

    def test_primary_copy_uses_retained_ownership_for_regular_and_primary_boards(self):
        source = ast.parse((TOOLS / 'native/primary.py').read_text())
        function, = [node for node in source.body if isinstance(node, ast.FunctionDef) and node.name == 'copy']
        owner = Mock(started=[object()])
        start = Mock(side_effect=lambda *args: owner.started.append(object()))
        sleep = Mock()
        namespace = dict(OWNED=owner, start=start, time=Mock(sleep=sleep))
        exec(compile(ast.Module(body=[function], type_ignores=[]), 'primary-copy-control', 'exec'), namespace)
        namespace['copy']('regular')
        namespace['copy']('selection', primary=True)
        self.assertEqual(start.call_args_list, [
            call(['wl-copy', '--foreground', '--', 'regular'], 'copy-1'),
            call(['wl-copy', '--foreground', '--primary', '--', 'selection'], 'copy-2'),
        ])
        self.assertEqual(sleep.call_args_list, [call(.4), call(.4)])

    def test_cleanup_spares_an_unregistered_process_with_the_same_environment(self):
        owner = owned.Processes()
        outsider = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'], env=self.env, start_new_session=True)
        try:
            inside = owner.start([sys.executable, '-c', 'import time; time.sleep(60)'], env=self.env)
            owner.close()
            self.assertIsNotNone(inside.poll())
            self.assertIsNone(outsider.poll())
            with self.assertRaises(AssertionError):
                owner.stop(outsider)
        finally:
            outsider.terminate()
            outsider.wait(timeout=5)

    def test_sealed_tree_ignores_a_different_index_and_writes_no_objects(self):
        self.repository()
        tree = self.git('rev-parse', 'HEAD^{tree}')
        (self.root / 'file').write_text('different staged body')
        self.git('add', 'file')
        git = self.root / '.git'
        def snapshot():
            return {str(p.relative_to(git)): hashlib.sha256(p.read_bytes()).hexdigest()
                    for p in git.rglob('*') if p.is_file()}
        before = snapshot()
        with patch.dict(os.environ, self.env, clear=True):
            self.assertEqual(owned.sealed_tree(self.root, tree), tree)
        self.assertEqual(snapshot(), before)

    def test_distinct_targets_use_the_same_lock_and_serialize(self):
        heavy = self.root / 'fake-heavy.sh'
        heavy.write_text('printf "%s start\\n" "$CARGO_TARGET_DIR" >> "$CONTROL_LOG"\nsleep .1\nprintf "%s end\\n" "$CARGO_TARGET_DIR" >> "$CONTROL_LOG"\n')
        log = self.root / 'ordering'
        private_lock = str(self.root / 'synthetic-rune.lock')
        pairs = []
        for target, arguments in [('one', ['--build', 'true']), ('two', ['true'])]:
            env = dict(self.env, RUNE_SYNC_HEAVY=str(heavy), CARGO_TARGET_DIR=target, CONTROL_LOG=str(log))
            command = launch.command(env, arguments)
            self.assertEqual(command[:3], ['flock', '--exclusive', '/mnt/build/rune-build.lock'])
            self.assertEqual(command.count('flock'), 1)
            self.assertEqual(env.get('RUNE_SYNC_BUILD_HELD'), '1' if target == 'one' else None)
            command[2] = private_lock  # Synthetic scheduler control; never takes the production lock.
            pairs.append((command, env))
        children = [subprocess.Popen(command, env=env) for command, env in pairs]
        for child in children:
            self.assertEqual(child.wait(timeout=10), 0)
        lines = log.read_text().splitlines()
        self.assertEqual(len(lines), 4)
        self.assertEqual(lines[0].split()[0], lines[1].split()[0])
        self.assertEqual(lines[2].split()[0], lines[3].split()[0])
        self.assertEqual([line.split()[1] for line in lines], ['start', 'end', 'start', 'end'])
        self.assertNotEqual(lines[0].split()[0], lines[2].split()[0])

    def test_host_inputs_reject_driver_and_library_drift(self):
        state = self.root / 'state'
        host_digests = [sys.executable, str(TOOLS / 'host_inputs.py'), str(state)]
        missing = subprocess.run(host_digests, env=self.env, capture_output=True, text=True)
        self.assertNotEqual(missing.returncode, 0)
        self.assertIn('host-inputs.json', missing.stderr)
        driver, library = self.root / 'driver.so', self.root / 'library.so'
        for path in (driver, library):
            path.write_bytes(b'original')
        host_inputs.materialize(state, driver, {'library.so': str(library)})
        host_inputs.verify(state)
        seal = self.root / 'artifacts.sha256'
        with seal.open('w') as stream:
            subprocess.run(['bash', str(TOOLS / 'artifacts.sh'), str(state)], env=self.env, check=True, stdout=stream)
            subprocess.run(host_digests, env=self.env, check=True, stdout=stream)
        def proposal_accepts():
            return subprocess.run(['sha256sum', '--check', '--status', str(seal)], env=self.env).returncode == 0
        self.assertTrue(proposal_accepts())
        for path in (driver, library):
            path.write_bytes(b'replacement')
            try:
                with self.assertRaisesRegex(AssertionError, 'host execution input changed'):
                    host_inputs.verify(state)
                self.assertFalse(proposal_accepts())
            finally:
                path.write_bytes(b'original')
        (state / 'syslibs/extra.so').symlink_to(library)
        with self.assertRaisesRegex(AssertionError, 'library inventory changed'):
            host_inputs.verify(state)

    def assert_motion_stops(self, analyzer_exit, slow_candidate, expected):
        state = self.root / 'state'
        (state / 'native-tools').mkdir()
        (state / 'candidates').mkdir()
        (state / 'native-tools/eon-transitions.json').write_text('{}')
        (state / 'native-tools/check-transitions.py').write_text('def check(frames): return [1]*9\ndef negative_checks(frames): pass\n')
        fixture = state / 'fixtures/venus/examples/eon_transitions.rs'
        fixture.parent.mkdir(parents=True)
        fixture.write_bytes((TOOLS / 'fixtures/venus/examples/eon_transitions.rs').read_bytes())
        binaries = {}
        for name in ('motion-upstream', 'motion-candidate'):
            path = state / 'candidates' / name
            path.write_bytes(name.encode())
            binaries[name] = {'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
        (state / 'provenance/native-candidates.json').write_text(json.dumps(dict(staged_tree='tree', binaries=binaries)))

        rendered = []
        def run(argv, **kwargs):
            if '--trace' not in argv:
                return subprocess.CompletedProcess(argv, analyzer_exit, stdout='analyzer output', stderr='')
            rendered.append(Path(argv[0]).name)
            trace = Path(argv[argv.index('--trace') + 1])
            regular = json.dumps({'quality': {'cpu_ms': 1}}) + '\n'
            last = json.dumps({'quality': {'cpu_ms': 9}}) + '\n' if slow_candidate and rendered[-1] == 'motion-candidate' else regular
            trace.write_text(json.dumps(dict(clock='virtual', hz=120, scale=2)) + '\n'
                             + regular * 929 + last)

        modules = dict(native_session=SimpleNamespace(session=lambda _: nullcontext({})),
                       reproduce=SimpleNamespace(W=state, R=self.root, OWNED=SimpleNamespace(run=run)),
                       owned=SimpleNamespace(sealed_tree=lambda *_: 'tree'))
        env = dict(self.env, RUNE_SYNC_TREE='tree', HEAVY_QUIET_EXEMPT='1',
                   RUNE_SYNC_QUIET_END=str(int(time.time()) + 300))
        script = TOOLS / 'native/motion.py'
        with patch.dict(os.environ, env, clear=True), patch.dict(sys.modules, modules), \
                patch.object(sys, 'argv', [str(script), 'control', '--quiet']):
            with self.assertRaisesRegex(AssertionError, 'motion audit' if analyzer_exit else '120 Hz'):
                runpy.run_path(str(script))
        receipt = json.loads((state / 'native-E/control/results.json').read_text())
        self.assertEqual(rendered, ['motion-upstream', 'motion-candidate'][:expected])
        self.assertEqual(len(receipt), expected)
        self.assertFalse(receipt[-1]['timing_acceptance'])

    def test_motion_saves_analyzer_failure_and_stops(self):
        self.assert_motion_stops(analyzer_exit=1, slow_candidate=False, expected=1)

    def test_motion_saves_timing_failure_and_stops(self):
        self.assert_motion_stops(analyzer_exit=0, slow_candidate=True, expected=2)


if __name__ == '__main__':
    assert os.environ.get('RUNE_SYNC_BOUNDED') == os.environ.get('RUNE_SYNC_BUILD_HELD') == '1'
    unittest.main(argv=[sys.argv[0]])
