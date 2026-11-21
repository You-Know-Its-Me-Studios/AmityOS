"""Project-local F-drive storage policy; no host-profile or HOME changes.

Call configure() before a build, VM, provisioner, or standalone test writes.
Only explicitly registered disposable raw images can enter automatic retention.
User images, backups, logs, source, and unregistered historical files never do.
"""
from __future__ import annotations

from contextlib import contextmanager
import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
EXPECTED_ROOT = Path('/mnt/f/AmityOS')
GIB = 1024 ** 3
POLICY_FILE = ROOT / '.relocation/storage-policy.json'
STATE = ROOT / '.cache/storage'
REGISTRY = STATE / 'disposables.json'
DEFAULTS = {'reserve_bytes': 30 * GIB, 'project_budget_bytes': 84 * GIB,
            'disposable_bytes': 16 * GIB, 'keep_successful_runs': 2}
_configured = False
_configuration = None
_run_id = None


def _mount():
    if os.name != 'posix' or ROOT != EXPECTED_ROOT or ROOT.is_symlink():
        raise RuntimeError('AmityOS storage requires the actual F:\\AmityOS checkout (/mnt/f/AmityOS); no C-drive fallback.')
    # An unmounted /mnt/f directory can otherwise silently write into the WSL
    # system VHD. Check the actual mount, filesystem, and Windows volume source.
    mounted = False
    for line in Path('/proc/self/mountinfo').read_text().splitlines():
        left, separator, right = line.partition(' - ')
        fields, filesystem = left.split(), right.split()
        if separator and len(fields) > 5 and fields[4] == '/mnt/f':
            source = re.sub(r'\\([0-7]{3})', lambda m: chr(int(m[1], 8)), filesystem[1])
            mounted = filesystem[0] in ('9p', 'drvfs') and source.lower().rstrip('\\/') == 'f:'
            mounted = mounted and 'rw' in fields[5].split(',')
            break
    if not mounted or not ROOT.is_dir():
        raise RuntimeError('F: is unavailable or not mounted read/write in WSL; no files created and no fallback permitted.')


def _policy():
    result = dict(DEFAULTS)
    if POLICY_FILE.exists():
        require_output(POLICY_FILE)
        if POLICY_FILE.stat().st_size > 4096:
            raise RuntimeError('AmityOS storage policy exceeds 4096 bytes.')
        supplied = json.loads(POLICY_FILE.read_text())
        if not isinstance(supplied, dict) or set(supplied) - set(result):
            raise RuntimeError('Unknown AmityOS storage policy fields.')
        result.update(supplied)
    if any(type(value) is not int or value <= 0 for value in result.values()):
        raise RuntimeError('Storage policy values must be positive integer bytes/counts.')
    result['reserve_bytes'] = max(result['reserve_bytes'], 30 * GIB)
    if result['keep_successful_runs'] < 2 or result['project_budget_bytes'] < result['disposable_bytes']:
        raise RuntimeError('Storage policy must retain at least two successes and fit retention inside the project budget.')
    return result


def require_output(path):
    """Validate an output before creating it; existing path components cannot redirect it."""
    _mount()
    path = Path(path).absolute()
    try:
        path.relative_to(ROOT)
        path.resolve().relative_to(ROOT)
    except ValueError as error:
        raise RuntimeError('AmityOS output must remain inside F:\\AmityOS: ' + str(path)) from error
    current = path
    while current != ROOT:
        if current.is_symlink():
            raise RuntimeError('AmityOS output cannot traverse a symbolic link: ' + str(current))
        current = current.parent
    return path.resolve()


def _project_bytes():
    # Native enumeration carries cached WIN32_FIND_DATA metadata. A Linux
    # lstat for every DrvFS file otherwise stalls the guard before compilation.
    # Count each file's full logical length, including sparse files and repeated
    # hardlinks, so the budget remains conservative. Never walk a C: fallback.
    _mount()
    helper = require_output(ROOT / 'tooling/storage_usage.ps1')
    if not helper.is_file():
        raise RuntimeError('Missing native F-drive storage accounting helper.')
    powershell = shutil.which('powershell.exe')
    if not powershell:
        raise RuntimeError('Native Windows storage accounting is unavailable; operation stopped.')
    try:
        result = subprocess.run(
            [powershell, '-NoProfile', '-NonInteractive', '-File',
             r'F:\AmityOS\tooling\storage_usage.ps1'],
            capture_output=True, text=True, encoding='utf-8-sig', timeout=30)
    except (OSError, subprocess.SubprocessError, UnicodeError) as error:
        raise RuntimeError('Native F-drive storage accounting failed or exceeded 30 seconds; '
                           'operation stopped without fallback.') from error
    if result.returncode != 0:
        raise RuntimeError('Cannot account for all F-drive project storage: '
                           + result.stderr.strip()[:512])
    try:
        if len(result.stdout) > 8192:
            raise ValueError('Oversized accounting response')
        value = json.loads(result.stdout)
        if (not isinstance(value, dict) or value.get('version') != 1
                or value.get('root') != r'F:\AmityOS'
                or any(type(value.get(key)) is not int or not 0 <= value[key] < 2**63
                       for key in ('project_bytes', 'file_count', 'skipped_reparse'))):
            raise ValueError('Invalid accounting response')
    except (ValueError, TypeError, KeyError) as error:
        raise RuntimeError('Native F-drive storage accounting returned invalid data; '
                           'operation stopped.') from error
    return value['project_bytes']


def require_space(predicted_bytes=0):
    """Reserve capacity for the next operation; never delete data to force success."""
    _mount()
    if type(predicted_bytes) is not int or predicted_bytes < 0:
        raise ValueError('predicted_bytes must be a nonnegative integer')
    policy = _policy()
    usage = shutil.disk_usage(ROOT)
    project = _project_bytes()
    if usage.free < policy['reserve_bytes'] + predicted_bytes:
        raise RuntimeError(f'AmityOS storage stopped: F: has {usage.free / GIB:.2f} GiB free; '
                           f'{policy["reserve_bytes"] / GIB:.2f} GiB reserve plus '
                           f'{predicted_bytes / GIB:.2f} GiB predicted allocation required. No C-drive fallback.')
    if project + predicted_bytes > policy['project_budget_bytes']:
        raise RuntimeError(f'AmityOS project budget exceeded: {project / GIB:.2f} GiB present + '
                           f'{predicted_bytes / GIB:.2f} GiB predicted; '
                           f'limit {policy["project_budget_bytes"] / GIB:.2f} GiB. '
                           'Protected files and unregistered evidence were not deleted.')
    return {'free_bytes': usage.free, 'project_bytes': project,
            'predicted_bytes': predicted_bytes, **policy}


def _environment():
    global _configured
    _mount()
    locations = {'TMPDIR': ROOT / '.cache/tmp', 'TEMP': ROOT / '.cache/tmp',
                 'TMP': ROOT / '.cache/tmp', 'CARGO_HOME': ROOT / '.deps/cargo',
                 'RUSTUP_HOME': ROOT / '.deps/rustup', 'CARGO_TARGET_DIR': ROOT / 'target',
                 'XDG_CACHE_HOME': ROOT / '.cache/xdg', 'PIP_CACHE_DIR': ROOT / '.cache/pip',
                 'DOTNET_CLI_HOME': ROOT / '.cache/dotnet', 'NUGET_PACKAGES': ROOT / '.cache/nuget',
                 'npm_config_cache': ROOT / '.cache/npm', 'UV_CACHE_DIR': ROOT / '.cache/uv',
                 'SCCACHE_DIR': ROOT / '.cache/sccache', 'CCACHE_DIR': ROOT / '.cache/ccache'}
    # Never create an empty replacement toolchain and silently download into it.
    for name in ('CARGO_HOME', 'RUSTUP_HOME'):
        path = require_output(locations[name])
        if not path.is_dir():
            raise RuntimeError(f'Missing relocated {name}: {path}; complete the verified toolchain move first.')
    for path in set(locations.values()) | {STATE}:
        require_output(path).mkdir(parents=True, exist_ok=True)
    for name, path in locations.items():
        os.environ[name] = str(path)
    tempfile.tempdir = str(locations['TMPDIR'])  # Overrides an earlier cached gettempdir().
    os.environ['PYTHONDONTWRITEBYTECODE'] = '1'
    os.environ['AMITY_WORKSPACE_ROOT'] = str(ROOT)
    os.environ['CARGO_BUILD_JOBS'] = '2'
    os.environ['CMAKE_BUILD_PARALLEL_LEVEL'] = '2'
    os.environ['MAKEFLAGS'] = '-j2'
    os.environ['PYTHONPYCACHEPREFIX'] = str(ROOT / '.cache/python')
    # Translate Linux paths for native Windows children, scoped to this process.
    forwarded = set(locations) - {'TMPDIR', 'CARGO_HOME', 'RUSTUP_HOME', 'CARGO_TARGET_DIR'}
    entries = [item for item in os.environ.get('WSLENV', '').split(':')
               if item and item.split('/')[0] not in forwarded | {'PYTHONPYCACHEPREFIX', 'PYTHONDONTWRITEBYTECODE'}]
    os.environ['WSLENV'] = ':'.join(entries + [name + '/pw' for name in sorted(forwarded)]
                                  + ['PYTHONPYCACHEPREFIX/pw', 'PYTHONDONTWRITEBYTECODE/w'])
    _configured = True
    return locations


def configure(predicted_bytes=0):
    """Initialize scoped paths, enforce completed-run retention, then check capacity."""
    global _configured, _configuration
    if type(predicted_bytes) is not int or predicted_bytes < 0:
        raise ValueError('predicted_bytes must be a nonnegative integer')
    if _configuration is not None:
        # Imports of several host suites share initialization. Explicit capacity
        # reservations still inspect current mount/free space/project size.
        _mount()
        if predicted_bytes:
            return {**_configuration, 'space': require_space(predicted_bytes)}
        return _configuration
    try:
        paths = _environment()
        # Retention is deliberately invoked here, rather than an unused helper.
        retention = prune_disposables()
        space = require_space(predicted_bytes)
        _configuration = {'root': str(ROOT), 'paths': {key: str(value) for key, value in paths.items()},
                          'space': space, 'retention': retention}
        return _configuration
    except Exception:
        # An environment partially configured before a capacity/retention error
        # is not a successful initialization; a subsequent attempt must retry.
        _configured = False
        _configuration = None
        raise


def rust_tool(name):
    if name not in ('cargo', 'rustc', 'rustup', 'rustfmt'):
        raise ValueError('Unknown Rust tool: ' + str(name))
    if _configuration is None:
        configure()
    path = ROOT / '.deps/cargo/bin' / name
    # rustup proxy links within the relocated Cargo bin directory are expected.
    try:
        path.resolve().relative_to(ROOT / '.deps')
    except ValueError as error:
        raise RuntimeError('Rust tool escapes the relocated dependency directory: ' + str(path)) from error
    if not path.is_file() or not os.access(path, os.X_OK):
        raise RuntimeError('Relocated Rust executable is missing/not executable: ' + str(path))
    return str(path)


def _process_start(pid):
    try:
        # comm may contain spaces or parentheses; fields after the last ')' start at field 3.
        return Path(f'/proc/{int(pid)}/stat').read_text().rsplit(')', 1)[1].split()[19]
    except FileNotFoundError:
        return None


def _boot_id():
    return Path('/proc/sys/kernel/random/boot_id').read_text().strip()


def _validate_registry(data):
    if not isinstance(data, dict) or data.get('version') != 1 or not isinstance(data.get('runs'), dict):
        raise RuntimeError('Invalid disposable registry; no cleanup performed.')
    if len(data['runs']) > 4096:
        raise RuntimeError('Too many disposable runs; no cleanup performed.')
    names = set()
    for key, run in data['runs'].items():
        if (not isinstance(key, str) or not re.fullmatch(r'[A-Za-z0-9_.-]{1,160}', key)
                or not isinstance(run, dict) or type(run.get('pid')) is not int or run['pid'] <= 0
                or not isinstance(run.get('start'), str) or not isinstance(run.get('boot'), str)
                or run.get('status') not in ('running', 'passed', 'failed', 'incomplete', 'interrupted')
                or not isinstance(run.get('images'), dict)
                or type(run.get('finished', 0)) not in (int, float)):
            raise RuntimeError('Malformed disposable run; no cleanup performed.')
        for name, identity in run['images'].items():
            if (not isinstance(name, str) or name in names or not isinstance(identity, dict)
                    or set(identity) != {'device', 'inode', 'size', 'mtime_ns'}
                    or any(type(value) is not int or value < 0 for value in identity.values())):
                raise RuntimeError('Malformed/duplicate disposable image identity; no cleanup performed.')
            _eligible_path(ROOT / name)
            names.add(name)


@contextmanager
def _registry():
    if not _configured:
        _environment()
    require_output(STATE).mkdir(parents=True, exist_ok=True)
    lock = require_output(STATE / 'disposables.lock')
    with lock.open('a+b') as stream:
        fcntl.flock(stream, fcntl.LOCK_EX)
        data = {'version': 1, 'runs': {}}
        if REGISTRY.exists():
            require_output(REGISTRY)
            if REGISTRY.stat().st_size > 4 * 1024 * 1024:
                raise RuntimeError('Disposable registry exceeds its bounded size; no cleanup performed.')
            data = json.loads(REGISTRY.read_text())
            _validate_registry(data)
        yield data


def _save(data):
    encoded = (json.dumps(data, indent=2) + '\n').encode()
    if len(encoded) > 4 * 1024 * 1024:
        raise RuntimeError('Disposable registry exceeds its bounded size.')
    fd, name = tempfile.mkstemp(prefix='registry-', dir=STATE)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, REGISTRY)
    finally:
        Path(name).unlink(missing_ok=True)


def _eligible_path(path):
    path = require_output(path)
    relative = path.relative_to(ROOT)
    allowed = any(path.is_relative_to(base) for base in
                  (ROOT / 'artifacts/verification', ROOT / '.cache/disposable'))
    protected = any('backup' in part.lower() for part in relative.parts)
    if not allowed or protected or path.suffix.lower() != '.img' or path.name in ('amity-user.img', 'amity-system.img'):
        raise RuntimeError('Not an eligible disposable test image: ' + str(path))
    return path


def _image(path):
    path = _eligible_path(path)
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
        raise RuntimeError('Disposable images must be ordinary single-link files: ' + str(path))
    with path.open('rb') as stream:
        header = stream.read(512)
    # Current Amity test images are raw partitioned disks, never backing chains.
    if len(header) != 512 or header[510:512] != b'\x55\xaa' or header[:4] in (b'QFI\xfb', b'KDMV'):
        raise RuntimeError('Disposable retention accepts only standalone raw MBR images: ' + str(path))
    return path, {'device': info.st_dev, 'inode': info.st_ino,
                  'size': info.st_size, 'mtime_ns': info.st_mtime_ns}


def register_disposable(path, *, run_id=None, run_dir=None):
    """Opt in a generated image, never a backup or caller-supplied persistent disk."""
    global _run_id
    if _configuration is None:
        configure()
    path, identity = _image(path)
    if run_dir is not None and not path.is_relative_to(require_output(run_dir)):
        raise RuntimeError('Disposable image lies outside the explicit test run directory.')
    if run_id is None:
        if _run_id is None:
            _run_id = f'{os.getpid()}-{uuid.uuid4().hex}'
        run_id = _run_id
    if not isinstance(run_id, str) or not re.fullmatch(r'[A-Za-z0-9_.-]{1,160}', run_id):
        raise ValueError('run_id must be a short filename-safe identifier')
    with _registry() as data:
        runs = data['runs']
        if run_id not in runs:
            if len(runs) >= 4096:
                raise RuntimeError('Disposable run registry is full; no unregistered data was deleted.')
            runs[run_id] = {'pid': os.getpid(), 'start': _process_start(os.getpid()), 'boot': _boot_id(),
                            'created': time.time(), 'status': 'running', 'images': {}}
        run = runs[run_id]
        if (run['pid'], run['start'], run['boot'], run['status']) != (os.getpid(), _process_start(os.getpid()), _boot_id(), 'running'):
            raise RuntimeError('Cannot append to another process/completed disposable run.')
        key = path.relative_to(ROOT).as_posix()
        if any(key in other['images'] for name, other in runs.items() if name != run_id):
            raise RuntimeError('Disposable image is already owned by another run.')
        run['images'][key] = identity
        _save(data)
    return run_id


def finish_disposable_run(status, *, run_id=None):
    """Call after all child VMs have exited. Failed/incomplete runs remain protected."""
    run_id = run_id or _run_id
    if run_id is None:
        return {'registered': False}
    if status not in ('passed', 'failed', 'incomplete', 'interrupted'):
        raise ValueError('Explicit final disposable-run status required')
    with _registry() as data:
        run = data['runs'].get(run_id)
        if run is None or (run['pid'], run['start'], run['boot'], run['status']) != (os.getpid(), _process_start(os.getpid()), _boot_id(), 'running'):
            raise RuntimeError('Only the registered run owner may finish it.')
        for key in run['images']:
            _, run['images'][key] = _image(ROOT / key)
        run['status'], run['finished'] = status, time.time()
        _save(data)
        return {'registered': True, 'run_id': run_id, 'status': status, 'images': len(run['images'])}


def _closed(path, identity):
    # Do not read unrelated command lines. Inspect only QEMU process names and
    # their open-file identities; any inaccessible QEMU is a refusal.
    for proc in Path('/proc').iterdir():
        if not proc.name.isdigit():
            continue
        try:
            if not (proc / 'comm').read_text().strip().startswith('qemu'):
                continue
            for fd in (proc / 'fd').iterdir():
                try:
                    info = fd.stat()
                except FileNotFoundError:
                    continue
                if (info.st_dev, info.st_ino) == (identity['device'], identity['inode']):
                    raise RuntimeError('Disposable image is still opened by QEMU: ' + str(path))
        except FileNotFoundError:
            continue
    # Native Windows QEMU is invisible in /proc. The existing read-exclusive
    # probe refuses sharing violations and every unknown/error outcome.
    powershell = shutil.which('powershell.exe')
    if not powershell:
        raise RuntimeError('Cannot verify native Windows image handles; image retained.')
    native = lambda p: 'F:\\AmityOS\\' + str(p.relative_to(ROOT)).replace('/', '\\')
    result = subprocess.run([powershell, '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
                             '-File', native(ROOT / 'tooling/windows_disk_probe.ps1'),
                             '-ImagePath', native(path)], capture_output=True, text=True, timeout=20)
    if result.returncode != 0 or json.loads(result.stdout).get('closed') is not True:
        raise RuntimeError('Native Windows handle probe refused cleanup: ' + str(path))


def prune_disposables():
    """Keep two latest successful runs; remove only closed, unchanged older images.

    The sidecar lease excludes cooperating launchers. Native handle probing is
    point-in-time; no claim is made about arbitrary out-of-band Linux launches.
    """
    _mount()
    policy = _policy()
    if not REGISTRY.exists():
        return {'deleted': [], 'retained': [], 'registered_bytes': 0}
    report = {'deleted': [], 'retained': [], 'registered_bytes': 0}
    with _registry() as data:
        runs = data['runs']
        completed = sorted((key for key, run in runs.items() if run['status'] == 'passed'),
                           key=lambda key: runs[key].get('finished', 0), reverse=True)
        keep = set(completed[:policy['keep_successful_runs']])
        for key, run in list(runs.items()):
            alive = run['boot'] == _boot_id() and _process_start(run['pid']) == run['start']
            eligible = run['status'] == 'passed' and key not in keep and not alive
            for name, expected in list(run['images'].items()):
                path = ROOT / name
                if not path.exists():
                    if eligible:
                        del run['images'][name]
                    continue
                info = path.lstat()
                report['registered_bytes'] += info.st_size
                if not eligible:
                    continue
                try:
                    path, actual = _image(path)
                    if actual != expected:
                        raise RuntimeError('Image changed since completed test; retained.')
                    lock = require_output(Path(str(path) + '.lock'))
                    with lock.open('a+b') as lease:
                        fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        _closed(path, actual)
                        if _image(path)[1] != actual:
                            raise RuntimeError('Image changed during cleanup check; retained.')
                        path.unlink()
                    report['deleted'].append(name)
                    report['registered_bytes'] -= actual['size']
                    del run['images'][name]
                except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
                    report['retained'].append({'path': name, 'reason': str(error)})
            if eligible and not run['images']:
                del runs[key]
        _save(data)
    # Failed/running runs are deliberately not sacrificed to satisfy retention.
    if report['registered_bytes'] > policy['disposable_bytes']:
        raise RuntimeError('Disposable image budget exceeds '
                           f'{policy["disposable_bytes"] / GIB:.2f} GiB after safe retention; '
                           'active, failed, changed, and protected images were retained. '
                           + json.dumps(report))
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('check', 'prune'))
    parser.add_argument('--predicted-bytes', type=int, default=0)
    args = parser.parse_args()
    result = configure(args.predicted_bytes) if args.command == 'check' else prune_disposables()
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
