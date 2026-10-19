import importlib.util
import json
import base64
import hashlib
import io
import contextlib
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('macos_observe', Path(__file__).with_name('macos.py'))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class MacosPackageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir='/tmp')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        quiet = contextlib.redirect_stdout(io.StringIO())
        quiet.__enter__()
        self.addCleanup(quiet.__exit__, None, None, None)
        self.image = self.root / 'screen.png'
        self.image.write_bytes(b'\x89PNG\r\n\x1a\n' + b'\0' * 8 + struct.pack('>II', 800, 600))
        self.args = type('Args', (), dict(image=self.image, character='Test', vision=False, result=None))()

    def test_corrupt_archive_installs_nothing(self):
        archive = self.root / 'bad.zip'
        archive.write_bytes(b'not the official archive')
        with patch.dict(os.environ, XDG_CACHE_HOME=str(self.root / 'cache')):
            with self.assertRaisesRegex(ValueError, 'checksum'):
                m.install_ahk(archive)
        self.assertEqual(list(self.root.rglob('*.exe')), [])

    def test_profile_name_cannot_escape_configuration_directory(self):
        with self.assertRaises(ValueError):
            m.profile_path('../other')

    def test_environment_removes_legacy_profile_and_background_settings(self):
        config = dict(prefix='/prefix', runner='/wine', ahk='/ahk', window_title='FFXI')
        with patch.dict(os.environ, FFXI_OBSERVE_PROFILE='wrong', FFXI_OBSERVE_BG='1', HXI_BG='1'):
            result = m.environment(config)
        self.assertNotIn('FFXI_OBSERVE_PROFILE', result)
        self.assertNotIn('FFXI_OBSERVE_BG', result)
        self.assertNotIn('HXI_BG', result)
        self.assertEqual(result['WINEPREFIX'], '/prefix')

    def test_key_rejects_invalid_input_before_host_access(self):
        for name, duration in [('enter', 'nan'), ('enter', '-1'), ('enter', '6'), ('no-such-key', '.2')]:
            with self.subTest(name=name, duration=duration), patch.object(m, 'unlocked') as unlocked:
                with self.assertRaises(ValueError):
                    m.key({}, name, duration)
                unlocked.assert_not_called()

    def test_key_file_must_be_private(self):
        key = self.root / 'key'
        key.write_text('test-key')
        key.chmod(0o644)
        with self.assertRaises(ValueError):
            m.credential(key)
        key.chmod(0o600)
        self.assertEqual(m.credential(key), 'test-key')

    def test_missing_key_http_timeout_and_bad_answer_request_vision(self):
        errors = [FileNotFoundError(), ValueError('HTTP 503'), TimeoutError(), KeyError('answers')]
        for error in errors:
            with self.subTest(error=type(error)), patch.object(m, 'jev_decide', side_effect=error):
                m.decide({}, self.args)
                result = json.loads(self.image.with_suffix('.decision.json').read_text())
                self.assertEqual(result['status'], 'needs_vision')
                self.assertEqual(result['image_sha256'], m.digest(self.image))
                self.assertIn('schema', result)

    def resize_stub(self, arguments, **kwargs):
        Path(arguments[-1]).write_bytes(Path(arguments[3]).read_bytes())

    def test_malformed_jev_json_requests_vision(self):
        payloads = [None, [], {'answers': None}, {'answers': []},
                    {'answers': {'next_key': None}}, {'answers': {'screen': [], 'next_key': {}}}]
        config = dict(jev_model=m.JEV_MODEL, jev_key_file='/not-read')
        for payload in payloads:
            with self.subTest(payload=payload), patch.object(m, 'run', side_effect=self.resize_stub), \
                    patch.object(m, 'credential', return_value='test'), \
                    patch.object(m.urllib.request, 'urlopen', return_value=io.StringIO(json.dumps(payload))):
                m.decide(config, self.args)
                result = json.loads(self.image.with_suffix('.decision.json').read_text())
                self.assertEqual(result['status'], 'needs_vision')

    def test_capture_replacement_cannot_relabel_an_old_decision(self):
        original = self.image.read_bytes()
        submitted = []
        def response(request, **kwargs):
            body = json.loads(request.data)
            submitted.append(base64.b64decode(body['assets']['game-screen']['data_base64']))
            self.image.write_bytes(original + b'new frame')
            return io.StringIO(json.dumps({'answers': {
                'screen': {'choice': 'character_select', 'confidence': 1},
                'next_key': {'choice': 'enter', 'confidence': 1}}}))
        config = dict(jev_model=m.JEV_MODEL, jev_key_file='/not-read')
        with patch.object(m, 'run', side_effect=self.resize_stub), \
                patch.object(m, 'credential', return_value='test'), \
                patch.object(m.urllib.request, 'urlopen', side_effect=response):
            m.decide(config, self.args)
        result = json.loads(self.image.with_suffix('.decision.json').read_text())
        self.assertEqual(submitted, [original])
        self.assertEqual(result['status'], 'needs_vision')
        raw = json.loads(self.image.with_suffix('.jev.json').read_text())
        self.assertEqual(raw['image_sha256'], hashlib.sha256(original).hexdigest())
        self.assertNotEqual(raw['image_sha256'], result['image_sha256'])

    def test_valid_jev_response_keeps_its_capture_hash(self):
        payload = {'answers': {'screen': {'choice': 'gameplay', 'confidence': 1},
                               'next_key': {'choice': 'stop', 'confidence': 1}}}
        config = dict(jev_model=m.JEV_MODEL, jev_key_file='/not-read')
        with patch.object(m, 'run', side_effect=self.resize_stub), \
                patch.object(m, 'credential', return_value='test'), \
                patch.object(m.urllib.request, 'urlopen', return_value=io.StringIO(json.dumps(payload))):
            m.decide(config, self.args)
        result = json.loads(self.image.with_suffix('.decision.json').read_text())
        self.assertEqual(result['provider'], 'jev')
        self.assertEqual(result['decision']['image_sha256'], m.digest(self.image))

    def test_low_confidence_jev_answer_requests_vision(self):
        response = dict(seconds=.1, image_sha256=m.digest(self.image), response=dict(answers={
            'screen': dict(choice='gameplay', confidence=.7),
            'next_key': dict(choice='stop', confidence=1)}))
        with patch.object(m, 'jev_decide', return_value=response):
            m.decide({}, self.args)
        result = json.loads(self.image.with_suffix('.decision.json').read_text())
        self.assertEqual(result['status'], 'needs_vision')

    def test_forced_vision_never_calls_jev(self):
        self.args.vision = True
        with patch.object(m, 'jev_decide') as jev:
            m.decide({}, self.args)
            jev.assert_not_called()

    def test_decision_rejects_wrong_image_and_unsafe_enter(self):
        result = dict(image_sha256=m.digest(self.image), screen='gameplay', next_key='stop', confidence=1)
        self.assertEqual(m.validate_decision(result, self.image), result)
        for changes in [dict(image_sha256='wrong'), dict(next_key='enter'), dict(confidence=float('nan')),
                        dict(screen='character_select', next_key='enter', confidence=.4)]:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                m.validate_decision({**result, **changes}, self.image)

    def test_native_vision_result_is_validated_and_saved(self):
        result = dict(image_sha256=m.digest(self.image), screen='gameplay', next_key='stop', confidence=1)
        self.args.result = self.root / 'vision.json'
        self.args.result.write_text(json.dumps(result))
        with patch.object(m, 'jev_decide') as jev:
            m.decide({}, self.args)
            jev.assert_not_called()
        saved = json.loads(self.image.with_suffix('.decision.json').read_text())
        self.assertEqual(saved['provider'], 'vision')
        self.assertEqual(saved['decision']['next_key'], 'stop')

    def test_changed_dependency_blocks_runtime(self):
        runner, loader, ahk = [self.root / name for name in ('wine', 'loader.exe', 'ahk.exe')]
        for path in (runner, loader, ahk):
            path.write_bytes(b'original')
            path.chmod(0o700)
        prefix = self.root / 'prefix'
        prefix.mkdir()
        (prefix / 'system.reg').touch()
        install = self.root / 'game'
        install.mkdir()
        dll = install / 'FFXiMain.dll'
        dll.write_bytes(b'client')
        config = dict(schema=m.PROFILE_VERSION, runner=str(runner), loader=str(loader), ahk=str(ahk),
                      runner_sha256=m.digest(runner), loader_sha256=m.digest(loader), ahk_sha256=m.digest(ahk),
                      prefix=str(prefix), install=str(install), client_sha256=m.digest(dll))
        profile = self.root / 'profile.json'
        profile.write_text(json.dumps(config))
        m.load(profile)
        ahk.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'ahk changed'):
            m.load(profile)

    def test_atomic_profile_is_private(self):
        profile = self.root / 'config' / 'profile.json'
        m.private_write(profile, b'{}')
        self.assertEqual(profile.stat().st_mode & 0o777, 0o600)
        m.private_write(profile, b'{"schema": 1}')
        self.assertEqual(json.loads(profile.read_text()), dict(schema=1))

    def test_ambiguous_windows_are_rejected(self):
        response = type('Result', (), dict(stdout=json.dumps([dict(name='FINAL FANTASY XI')] * 2)))()
        with patch.object(m, 'environment', return_value={}), patch.object(m, 'run', return_value=response):
            with self.assertRaisesRegex(ValueError, 'found 2'):
                m.window(dict(window_title='FINAL FANTASY XI'))


if __name__ == '__main__':
    unittest.main()
