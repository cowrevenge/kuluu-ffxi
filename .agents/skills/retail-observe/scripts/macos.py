#!/usr/bin/env python3
import argparse
import base64
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import zipfile

SCRIPT_DIR = Path(__file__).resolve().parent
AHK_VERSION = '2.0.30'
AHK_ZIP_SHA256 = '1e0ece7ecb2d5646e55758150472677996a57f9db5c83d2996d8b47b9e2f31a4'
AHK_EXE_SHA256 = '2eab99848ebc07f8ab0cc448bf69af8c700b2fc7fcd1ff8794df743dcc58da51'
AHK_URL = f'https://github.com/AutoHotkey/AutoHotkey/releases/download/v{AHK_VERSION}/AutoHotkey_{AHK_VERSION}.zip'
JEV_URL = 'https://api.typesafe.ai/v1/systemone'
JEV_MODEL = 'jev-1.14.0'
MIN_IMAGE_EDGE = 280
MAX_IMAGE_EDGE = 1568
MAX_DOWNLOAD_BYTES = 20 * 1024 * 1024
PROFILE_VERSION = 1
COMMAND_TIMEOUT_SECONDS = 30
MAX_KEY_HOLD_SECONDS = 5
DECISION_CONFIDENCE_THRESHOLD = 0.9
DEFAULT_HOLD_SECONDS = '0.2'
WIN32_KEY_COLUMN = 3
PNG_SIGNATURE = b'\x89PNG\r\n\x1a\n'
PNG_SIZE_OFFSET = 16
PNG_HEADER_BYTES = 24
PRIVATE_PERMISSION_MASK = 0o077


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def run(args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, timeout=COMMAND_TIMEOUT_SECONDS, **kwargs)


def private_write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd, temporary = tempfile.mkstemp(dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(data)
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def profile_path(name):
    if not re.fullmatch(r'[A-Za-z0-9_-]+', name):
        raise ValueError('Profile name must contain only letters, numbers, underscores, or hyphens.')
    root = Path(os.environ.get('XDG_CONFIG_HOME', Path.home() / '.config'))
    return root / 'ffxi-observe' / f'{name}.macos.json'


def checked_file(value, executable=False):
    path = Path(value).expanduser().resolve()
    if not path.is_file() or (executable and not os.access(path, os.X_OK)):
        raise ValueError(f'Required file is unavailable: {path}')
    return path


def install_ahk(archive=None):
    root = Path(os.environ.get('XDG_CACHE_HOME', Path.home() / '.cache'))
    target = root / 'ffxi-observe' / f'AutoHotkey-{AHK_VERSION}' / 'AutoHotkey64.exe'
    if target.is_file() and digest(target) == AHK_EXE_SHA256:
        return target
    if archive:
        data = checked_file(archive).read_bytes()
    else:
        with urllib.request.urlopen(AHK_URL, timeout=COMMAND_TIMEOUT_SECONDS) as response:
            data = response.read(MAX_DOWNLOAD_BYTES + 1)
    if len(data) > MAX_DOWNLOAD_BYTES or hashlib.sha256(data).hexdigest() != AHK_ZIP_SHA256:
        raise ValueError('AutoHotkey archive checksum failed. No executable was installed.')
    with tempfile.TemporaryFile() as stream:
        stream.write(data)
        stream.seek(0)
        with zipfile.ZipFile(stream) as archive_file:
            executable = archive_file.read('AutoHotkey64.exe')
    if hashlib.sha256(executable).hexdigest() != AHK_EXE_SHA256:
        raise ValueError('AutoHotkey executable checksum failed.')
    private_write(target, executable)
    return target


def setup(args, path):
    if path.exists() and not args.replace:
        raise ValueError('Profile already exists. Use --replace to update it.')
    runner = checked_file(shutil.which(args.runner) or args.runner, executable=True)
    prefix = Path(args.prefix).expanduser().resolve()
    checked_file(prefix / 'system.reg')
    install = Path(args.install).expanduser().resolve()
    dll = checked_file(install / 'FFXiMain.dll')
    loader = checked_file(args.loader)
    loader_hash = digest(loader)
    cache = Path(os.environ.get('XDG_CACHE_HOME', Path.home() / '.cache'))
    cached_loader = cache / 'ffxi-observe' / 'loaders' / loader_hash / loader.name
    private_write(cached_loader, loader.read_bytes())
    loader = cached_loader
    if any(re.search(r'(?i)(--?(?:user|pass|password|username)(?:=|\b))', arg) for arg in args.loader_arg):
        raise ValueError('Do not store login credentials in loader arguments. Use FFXI_USER and FFXI_PASS.')
    ahk = install_ahk(args.ahk_archive)
    version = run([runner, '--version'], capture_output=True, text=True).stdout.strip()
    config = dict(schema=PROFILE_VERSION, runner=str(runner), runner_sha256=digest(runner),
                  runner_version=version, prefix=str(prefix), install=str(install),
                  client_sha256=digest(dll), loader=str(loader), loader_sha256=digest(loader),
                  loader_args=args.loader_arg, server=args.server, window_title='FINAL FANTASY XI',
                  ahk=str(ahk), ahk_sha256=AHK_EXE_SHA256, jev_model=JEV_MODEL,
                  jev_key_file=str(Path(args.jev_key_file).expanduser().resolve()),
                  machine=platform.machine(), macos=platform.mac_ver()[0])
    private_write(path, (json.dumps(config, indent=2) + '\n').encode())
    print(f'Profile saved: {path}')


def load(path):
    config = json.loads(path.read_text())
    if config['schema'] != PROFILE_VERSION:
        raise ValueError('Unsupported profile version. Run setup again.')
    for field in ('runner', 'loader', 'ahk'):
        checked_file(config[field], executable=field == 'runner')
        if digest(config[field]) != config[field + '_sha256']:
            raise ValueError(f'{field} changed. Verify the update, then run setup with --replace.')
    checked_file(Path(config['prefix']) / 'system.reg')
    if digest(checked_file(Path(config['install']) / 'FFXiMain.dll')) != config['client_sha256']:
        raise ValueError('The client changed. Verify the update, then run setup with --replace.')
    return config


def environment(config):
    env = os.environ.copy()
    for name in list(env):
        if name.startswith('FFXI_OBSERVE_') or name in ('HXI_BG', 'HXI_GAME_RE', 'HXI_OWNER_RE'):
            del env[name]
    env.update(WINEPREFIX=config['prefix'], WINEDEBUG='-all', FFXI_OBSERVE_HOST='macos',
               FFXI_OBSERVE_RUNNER=config['runner'], FFXI_OBSERVE_AHK=config['ahk'],
               FFXI_OBSERVE_WINDOW_TITLE=config['window_title'])
    return env


def permissions():
    script = '''ObjC.import('CoreGraphics');
ObjC.bindFunction('CGPreflightScreenCaptureAccess', ['bool', []]);
JSON.stringify({capture: $.CGPreflightScreenCaptureAccess()});'''
    result = run(['osascript', '-l', 'JavaScript', '-e', script], capture_output=True, text=True)
    if not json.loads(result.stdout)['capture']:
        raise ValueError('Grant Screen Recording to the calling application, then restart it.')


def unlocked():
    result = run(['ioreg', '-l', '-w0'], capture_output=True, text=True)
    if '"CGSSessionScreenIsLocked"=Yes' in result.stdout:
        raise ValueError('Unlock the Mac before sending input.')


def window(config, allow_missing=False):
    env = environment(config)
    env['SCRIPT_DIR'] = str(SCRIPT_DIR)
    command = '. "$SCRIPT_DIR/lib/common.sh"; . "$SCRIPT_DIR/lib/macos.sh"; WINDOW_OWNER=.; host_windows_json'
    result = run(['bash', '-c', command], env=env, capture_output=True, text=True)
    matches = [item for item in json.loads(result.stdout)
               if config['window_title'].lower() in item['name'].lower()]
    if not matches and allow_missing:
        return None
    if len(matches) != 1:
        raise ValueError(f'Expected one FFXI window; found {len(matches)}. Close duplicate clients or start the game.')
    return matches[0]


def png_size(path):
    data = path.read_bytes()
    if len(data) < PNG_HEADER_BYTES or not data.startswith(PNG_SIGNATURE):
        raise ValueError('Expected a PNG capture.')
    return struct.unpack('>II', data[PNG_SIZE_OFFSET:PNG_HEADER_BYTES])


def capture(config, path):
    permissions()
    path = path.expanduser().resolve()
    target = window(config)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.unlink(missing_ok=True)
    run(['caffeinate', '-u', '-t', '2'], capture_output=True)
    run(['screencapture', '-x', '-o', '-l', target['id'], path], capture_output=True)
    width, height = png_size(path)
    if not width or not height:
        raise ValueError('The capture has invalid dimensions.')
    print(json.dumps(dict(image=str(path), width=width, height=height,
                          scale=width / target['w'], window=target)))
    return path


def key(config, name, seconds):
    duration = float(seconds)
    if not math.isfinite(duration) or not 0 <= duration <= MAX_KEY_HOLD_SECONDS:
        raise ValueError(f'Hold duration must be between 0 and {MAX_KEY_HOLD_SECONDS} seconds.')
    keys = dict(line.split('\t')[:1] + [line.split('\t')[WIN32_KEY_COLUMN]]
                for line in (SCRIPT_DIR / 'lib/keys.tsv').read_text().splitlines()
                if line and not line.startswith('#'))
    if name not in keys:
        raise ValueError('Unknown logical key. Use observe.sh keys to list the names.')
    unlocked()
    window(config)
    lock_id = hashlib.sha256(config['prefix'].encode()).hexdigest()
    lock = profile_path('input-' + lock_id).with_suffix('.lock')
    lock.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with lock.open('a') as stream:
        try:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError('Another packaged input command is active for this Wine prefix.') from None
        env = environment(config)
        helper = SCRIPT_DIR / 'lib/wine-key.ahk'
        converted = run([config['runner'], 'winepath.exe', '-w', helper],
                        env=env, capture_output=True, text=True).stdout.strip()
        run([config['runner'], config['ahk'], '/ErrorStdOut', converted,
             config['window_title'], keys[name], round(duration * 1000)], env=env, capture_output=True)
    print(json.dumps(dict(key=name, hold_seconds=duration, submitted=True,
                          visible_result_verified=False)))


def credential(path):
    info = path.stat()
    if info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) & PRIVATE_PERMISSION_MASK:
        raise ValueError('The Jev key file must belong to this user and have permissions 0600 or 0400.')
    value = path.read_text().strip()
    if not value or '\n' in value or '\r' in value:
        raise ValueError('The Jev key file is empty or invalid.')
    return value


def jev_decide(config, image, character):
    image = image.expanduser().resolve()
    png_size(image)
    with tempfile.TemporaryDirectory() as temporary:
        source = Path(temporary) / 'source.png'
        source_data = image.read_bytes()
        source_hash = hashlib.sha256(source_data).hexdigest()
        source.write_bytes(source_data)
        resized = Path(temporary) / 'screen.png'
        run(['sips', '-Z', MAX_IMAGE_EDGE, source, '--out', resized], capture_output=True)
        if min(png_size(resized)) < MIN_IMAGE_EDGE:
            raise ValueError('The image is too small for Jev. Each edge must be at least 280 pixels.')
        image_data = resized.read_bytes()
    reference = 'game-screen'
    body = {'model': config['jev_model'], 'state': {'screen_image': reference, 'character': character},
            'assets': {reference: {'type': 'image', 'format': 'png',
                                  'data_base64': base64.b64encode(image_data).decode()}},
            'questions': {
                'screen': {'type': 'choice', 'instructions': 'Classify only the visible screen_image.',
                           'criteria': {'agreement': 'A user agreement or consent prompt.',
                                        'character_menu': 'The title menu with Select Character highlighted.',
                                        'character_select': 'The character list.',
                                        'character_confirm': 'A local character login confirmation.',
                                        'loading': 'The game is loading or connecting.',
                                        'gameplay': 'A character is inside a game zone with a gameplay HUD.',
                                        'error': 'An error dialog.', 'other': 'An unclear or different screen.'}},
                'next_key': {'type': 'choice',
                             'instructions': 'Suggest one action. Never accept agreements or dismiss errors. '
                                             'Use state.character as the exact character name.',
                             'criteria': {'enter': 'Select Character is highlighted, or the exact requested character '
                                                  'is highlighted, or Yes is highlighted for that character login.',
                                          'wait': 'Loading or connecting requires no input.',
                                          'stop': 'Gameplay, agreement, error, different character, or unclear state.'}}}}
    request = urllib.request.Request(JEV_URL, data=json.dumps(body).encode(),
                                     headers={'Authorization': 'Bearer ' + credential(Path(config['jev_key_file'])),
                                              'Content-Type': 'application/json',
                                              'TypeSafe-Danger-Zone-Images': 'true'})
    started = time.monotonic()
    try:
        with urllib.request.urlopen(request, timeout=COMMAND_TIMEOUT_SECONDS) as response:
            result = json.load(response)
    except urllib.error.HTTPError as error:
        raise ValueError(f'Jev returned HTTP {error.code}. No input was sent.') from None
    if not isinstance(result, dict) or not isinstance(result.get('answers'), dict):
        raise ValueError('Jev returned an invalid answer structure. No input was sent.')
    answers = result['answers']
    if any(not isinstance(answers.get(field), dict) for field in ('screen', 'next_key')):
        raise ValueError('Jev returned an invalid answer structure. No input was sent.')
    if answers['next_key'].get('choice') not in ('enter', 'wait', 'stop'):
        raise ValueError('Jev returned an unsupported decision. No input was sent.')
    output = dict(image=str(image), image_sha256=source_hash,
                  submitted_sha256=hashlib.sha256(image_data).hexdigest(),
                  seconds=round(time.monotonic() - started, 3), response=result)
    target = image.with_suffix('.jev.json')
    private_write(target, (json.dumps(output, indent=2) + '\n').encode())
    return output


SCREENS = ('agreement', 'character_menu', 'character_select', 'character_confirm',
           'loading', 'gameplay', 'error', 'other')
DECISION_SCHEMA = {
    'type': 'object', 'additionalProperties': False,
    'required': ['image_sha256', 'screen', 'next_key', 'confidence'],
    'properties': {'image_sha256': {'type': 'string'},
                   'screen': {'enum': list(SCREENS)},
                   'next_key': {'enum': ['enter', 'wait', 'stop']},
                   'confidence': {'type': 'number', 'minimum': 0, 'maximum': 1}}}


def validate_decision(result, image):
    if set(result) != set(DECISION_SCHEMA['required']):
        raise ValueError('Decision fields do not match the requested schema.')
    if result['image_sha256'] != digest(image):
        raise ValueError('The decision does not match this capture.')
    if result['screen'] not in SCREENS or result['next_key'] not in ('enter', 'wait', 'stop'):
        raise ValueError('Unsupported screen or action.')
    confidence = result['confidence']
    if type(confidence) not in (int, float) or not math.isfinite(confidence) or not 0 <= confidence <= 1:
        raise ValueError('Invalid decision confidence.')
    if result['next_key'] == 'enter' and (confidence < DECISION_CONFIDENCE_THRESHOLD or result['screen'] not in
                                        ('character_menu', 'character_select', 'character_confirm')):
        raise ValueError('Enter is not permitted for this screen or confidence.')
    if result['next_key'] == 'wait' and result['screen'] != 'loading':
        raise ValueError('Wait is permitted only for a loading screen.')
    return result


def decide(config, args):
    image = args.image.expanduser().resolve()
    png_size(image)
    if args.result:
        result = validate_decision(json.loads(args.result.read_text()), image)
        output = dict(status='decision', provider='vision', decision=result, image=str(image))
    else:
        try:
            if args.vision:
                raise ValueError('Vision was selected.')
            result = jev_decide(config, image, args.character)
            answers = result['response']['answers']
            decision = dict(image_sha256=result['image_sha256'], screen=answers['screen']['choice'],
                            next_key=answers['next_key']['choice'],
                            confidence=min(answers['screen']['confidence'], answers['next_key']['confidence']))
            validate_decision(decision, image)
            if decision['confidence'] < DECISION_CONFIDENCE_THRESHOLD:
                raise ValueError('Jev confidence requires vision review.')
            output = dict(status='decision', provider='jev', decision=decision,
                          image=str(image), seconds=result['seconds'])
        except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
            output = dict(status='needs_vision', provider='vision', reason=type(error).__name__, image=str(image),
                          image_sha256=digest(image), character=args.character, schema=DECISION_SCHEMA,
                          instructions='Open this capture with a vision-capable agent or subagent. '
                                       'Return only the requested JSON. Identify the exact requested character. '
                                       'Use Enter only for Select Character, that character, or its Yes login confirmation. '
                                       'Use Wait only for loading. Use Stop for gameplay, agreements, errors, or uncertainty. '
                                       'Do not send input. Validate the result with decide --result before use.')
    private_write(image.with_suffix('.decision.json'), (json.dumps(output, indent=2) + '\n').encode())
    print(json.dumps(output, indent=2))


def main():
    parser = argparse.ArgumentParser(description='Configure and drive the FFXI Wine client on macOS.')
    parser.add_argument('--profile', default='macos')
    sub = parser.add_subparsers(dest='command', required=True)
    setup_parser = sub.add_parser('setup')
    for option in ('runner', 'prefix', 'install', 'loader'):
        setup_parser.add_argument('--' + option, required=True)
    setup_parser.add_argument('--server', default='127.0.0.1')
    setup_parser.add_argument('--loader-arg', action='append', default=[])
    setup_parser.add_argument('--jev-key-file', default='~/.config/typesafe/api-key')
    setup_parser.add_argument('--ahk-archive')
    setup_parser.add_argument('--replace', action='store_true')
    sub.add_parser('doctor')
    capture_parser = sub.add_parser('capture')
    capture_parser.add_argument('image', type=Path)
    key_parser = sub.add_parser('key')
    key_parser.add_argument('name')
    key_parser.add_argument('seconds', nargs='?', default=DEFAULT_HOLD_SECONDS)
    decide_parser = sub.add_parser('decide')
    decide_parser.add_argument('image', type=Path)
    decide_parser.add_argument('--character', required=True)
    decide_parser.add_argument('--vision', action='store_true')
    decide_parser.add_argument('--result', type=Path)
    sub.add_parser('launch')
    args = parser.parse_args()
    if sys.platform != 'darwin':
        raise ValueError('This command requires macOS.')
    path = profile_path(args.profile)
    if args.command == 'setup':
        setup(args, path)
        return
    config = load(path)
    if args.command == 'doctor':
        for command in ('python3', 'bash', 'osascript', 'screencapture', 'sips', 'ioreg', 'caffeinate'):
            if not shutil.which(command):
                raise ValueError(f'Required command is missing: {command}')
        permissions()
        unlocked()
        print(json.dumps(dict(window=window(config, allow_missing=True))))
        if platform.machine() == 'arm64':
            run(['arch', '-x86_64', '/usr/bin/true'], capture_output=True)
            print('Rosetta execution passed.')
        print(f"Wine: {config['runner_version']}")
        print(f"Client SHA256: {config['client_sha256']}")
        print('Input dependencies passed. Input delivery still requires a visible check.')
        print('If no window was found, launch the client before the visible check.')
        if Path(config['jev_key_file']).exists():
            try:
                credential(Path(config['jev_key_file']))
                print('Jev credential permissions passed. API access is checked only by decide.')
            except (OSError, ValueError):
                print('Jev credential is unavailable. Decide will request vision fallback.')
        else:
            print('Jev credential is absent. Capture and input do not require Jev.')
    elif args.command == 'capture':
        capture(config, args.image)
    elif args.command == 'key':
        key(config, args.name, args.seconds)
    elif args.command == 'decide':
        decide(config, args)
    elif args.command == 'launch':
        unlocked()
        user, password = os.environ.get('FFXI_USER'), os.environ.get('FFXI_PASS')
        if not user or not password:
            raise ValueError('Set FFXI_USER and FFXI_PASS before launch.')
        arguments = [config['runner'], config['loader'], *config['loader_args'],
                     '--server', config['server'], '--user', user, '--pass', password]
        os.chdir(Path(config['loader']).parent)
        env = environment(config)
        env['WINEDLLOVERRIDES'] = 'mscoree,mshtml='
        os.execve(config['runner'], arguments, env)


if __name__ == '__main__':
    try:
        main()
    except subprocess.CalledProcessError as error:
        print(f'macos-observe: A required command failed (exit {error.returncode}). No command arguments were logged.', file=sys.stderr)
        raise SystemExit(1)
    except (OSError, ValueError, KeyError, subprocess.TimeoutExpired, zipfile.BadZipFile) as error:
        print(f'macos-observe: {error}', file=sys.stderr)
        raise SystemExit(1)
