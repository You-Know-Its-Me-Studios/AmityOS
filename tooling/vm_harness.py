#!/usr/bin/env python3
"""Drive a running AmityOS development VM from the Windows host for acceptance.

The VM is started by AmityOS.ps1 with a root serial console and QMP, both on
loopback TCP. This harness runs commands on the serial console, takes guest
screenshots through QMP, and writes a JSON record of what was observed.
Standard library only.
"""
from __future__ import annotations

import argparse
import functools
import http.server
import json
import re
import socket
import threading
import time
from pathlib import Path
from typing import Any


class Serial:
    """Root shell on the guest's first serial port (devel image only)."""

    def __init__(self, port: int, log: Path):
        self.sock = self._connect(port)
        self.sock.settimeout(1.0)
        self.buffer = ''
        self.log = log.open('a', encoding='utf-8', newline='')
        self.count = 0

    @staticmethod
    def _connect(port: int, timeout: float = 60.0) -> socket.socket:
        deadline = time.monotonic() + timeout
        while True:
            try:
                return socket.create_connection(('127.0.0.1', port), timeout=5)
            except OSError:
                if time.monotonic() > deadline:
                    raise
                time.sleep(0.5)

    def _read(self) -> None:
        try:
            data = self.sock.recv(65536)
        except socket.timeout:
            return
        if not data:
            raise ConnectionError('serial connection closed')
        text = data.decode('utf-8', 'replace')
        self.log.write(text)
        self.log.flush()
        self.buffer += text

    def wait_for(self, pattern: str, timeout: float) -> re.Match:
        deadline = time.monotonic() + timeout
        compiled = re.compile(pattern)
        while True:
            match = compiled.search(self.buffer)
            if match:
                return match
            if time.monotonic() > deadline:
                raise TimeoutError(f'serial: no {pattern!r} within {timeout:.0f}s; tail: {self.buffer[-400:]!r}')
            self._read()

    def login(self, timeout: float) -> None:
        """Wait for the autologin root prompt, nudging the console with Enter."""
        deadline = time.monotonic() + timeout
        while True:
            self.sock.sendall(b'\n')
            try:
                self.wait_for(r'root@[\w-]+:[^\n]*# ?$', 5)
                break
            except TimeoutError:
                # A guest that panicked will never show a prompt: fail now, not at the deadline.
                panic = re.search(r'Kernel panic - not syncing[^\r\n]*', self.buffer)
                if panic:
                    raise RuntimeError(f'the guest kernel panicked during boot: {panic.group(0)}') from None
                if time.monotonic() > deadline:
                    raise
        # A plain, predictable shell: no colours, no line editing surprises, and
        # none of systemd's shell-integration escape sequences around commands.
        self.run("unset PROMPT_COMMAND PS0; export TERM=dumb PS1= PS2=; stty -echo cols 250; "
                 "bind 'set enable-bracketed-paste off' 2>/dev/null; true", 30)

    # Terminal control sequences (OSC, DCS, CSI) that are not command output.
    ESCAPES = re.compile(r'\x1b\][^\x1b\x07]*(?:\x1b\\|\x07)|\x1bP[^\x1b]*\x1b\\|\x1b\[[0-9;?]*[ -/]*[@-~]')

    def run(self, command: str, timeout: float = 120.0) -> tuple[int, str]:
        self.count += 1
        marker = f'AMITY_DONE_{self.count}'
        self.buffer = ''
        self.sock.sendall((command + f"; printf '\\n{marker}=%s\\n' $?\n").encode())
        match = self.wait_for(rf'\n{marker}=(\d+)\r?\n', timeout)
        output = self.ESCAPES.sub('', self.buffer[:match.start()]).replace('\r', '')
        return int(match.group(1)), output.strip()


class Qmp:
    def __init__(self, port: int):
        self.sock = Serial._connect(port)
        self.file = self.sock.makefile('rwb')
        json.loads(self.file.readline())
        self.execute('qmp_capabilities')

    def execute(self, command: str, arguments: dict | None = None) -> dict:
        request: dict[str, Any] = {'execute': command}
        if arguments:
            request['arguments'] = arguments
        self.file.write(json.dumps(request).encode() + b'\n')
        self.file.flush()
        while True:
            reply = json.loads(self.file.readline())
            if 'return' in reply:
                return reply['return']
            if 'error' in reply:
                raise RuntimeError(f'QMP {command}: {reply["error"]}')

    def screenshot(self, path: Path, head: int | None = None) -> None:
        arguments: dict[str, Any] = {'filename': str(path), 'format': 'png'}
        if head is not None:
            arguments.update({'device': 'video0', 'head': head})
        self.execute('screendump', arguments)

    # Input goes through the guest's virtual keyboard and tablet, so it takes
    # the same path as real hardware: kernel, libinput, compositor.
    def key(self, name: str, down: bool) -> None:
        event = {'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': name}}}
        self.execute('input-send-event', {'events': [event]})
        time.sleep(0.05)

    def chord(self, *names: str) -> None:
        """Press the keys in order and release them in reverse order."""
        for name in names:
            self.key(name, True)
        time.sleep(0.05)
        for name in reversed(names):
            self.key(name, False)

    KEY_NAMES = {' ': 'spc', '-': 'minus', '.': 'dot', '/': 'slash', '=': 'equal', ',': 'comma'}

    def type_text(self, text: str) -> None:
        """Type lowercase letters, digits and a few symbols on a US layout."""
        for character in text:
            self.chord(self.KEY_NAMES.get(character, character))

    def pointer(self, x: float, y: float, size: tuple[int, int]) -> None:
        """Move the pointer to a pixel of a desktop that is `size` pixels large
        (all outputs side by side: the tablet covers the whole layout)."""
        events = [
            {'type': 'abs', 'data': {'axis': 'x', 'value': round(x * 0x7fff / max(size[0] - 1, 1))}},
            {'type': 'abs', 'data': {'axis': 'y', 'value': round(y * 0x7fff / max(size[1] - 1, 1))}},
        ]
        self.execute('input-send-event', {'events': events})
        time.sleep(0.08)

    def button(self, down: bool, button: str = 'left') -> None:
        event = {'type': 'btn', 'data': {'down': down, 'button': button}}
        self.execute('input-send-event', {'events': [event]})
        time.sleep(0.05)

    def click(self, x: float, y: float, size: tuple[int, int], button: str = 'left') -> None:
        self.pointer(x, y, size)
        self.button(True, button)
        self.button(False, button)


class Head:
    """Switches a secondary display head of the VM on and off.

    QEMU keeps the extra outputs of a virtio GPU disconnected until a display
    front end announces a size for them. Without a window, the only front end
    is the VNC server bound to that head, so this is a minimal RFB client that
    sends one SetDesktopSize request. The server listens on loopback only.
    """

    def __init__(self, port: int):
        self.sock = Serial._connect(port, 20)
        self.sock.settimeout(10)
        if not self._read(12).startswith(b'RFB '):
            raise RuntimeError('not a VNC server')
        self.sock.sendall(b'RFB 003.008\n')
        types = self._read(self._read(1)[0])
        if 1 not in types:
            raise RuntimeError(f'the VNC server wants authentication: {list(types)}')
        self.sock.sendall(bytes([1]))                 # security type: none
        if int.from_bytes(self._read(4), 'big') != 0:
            raise RuntimeError('VNC security handshake failed')
        self.sock.sendall(bytes([1]))                 # ClientInit: shared
        header = self._read(24)                       # ServerInit
        self._read(int.from_bytes(header[20:24], 'big'))
        # SetEncodings: raw and ExtendedDesktopSize, which enables SetDesktopSize.
        self.sock.sendall(bytes([2, 0]) + (2).to_bytes(2, 'big') + (0).to_bytes(4, 'big', signed=True)
                          + (-308).to_bytes(4, 'big', signed=True))

    def _read(self, count: int) -> bytes:
        data = b''
        while len(data) < count:
            chunk = self.sock.recv(count - len(data))
            if not chunk:
                raise ConnectionError('VNC connection closed')
            data += chunk
        return data

    def resize(self, width: int, height: int) -> None:
        """Announce a size for the head; 0 x 0 disconnects it."""
        screen = (0).to_bytes(4, 'big') + bytes(4) + width.to_bytes(2, 'big') + height.to_bytes(2, 'big') + bytes(4)
        self.sock.sendall(bytes([251, 0]) + width.to_bytes(2, 'big') + height.to_bytes(2, 'big') + bytes([1, 0]) + screen)


class Recorder:
    def __init__(self, out: Path, serial: Serial, qmp: Qmp):
        self.out, self.serial, self.qmp = out, serial, qmp
        self.record: dict[str, Any] = {'checks': [], 'method': 'root serial console + QMP screendump; no host input automation'}

    def check(self, name: str, command: str, expect: str | None = None, timeout: float = 120.0, required: bool = True):
        code, output = self.serial.run(command, timeout)
        passed = code == 0 and (expect is None or re.search(expect, output) is not None)
        entry = {'name': name, 'command': command, 'exit_code': code, 'output': output[-2000:],
                 'status': 'passed' if passed else 'failed'}
        self.record['checks'].append(entry)
        print(f"{entry['status'].upper():6} {name}: {output.splitlines()[-1][:120] if output else ''}", flush=True)
        if required and not passed:
            self.save()
            raise RuntimeError(f'check failed: {name}')
        return passed, output

    def screenshot(self, name: str, head: int | None = None, settle: float = 1.5) -> None:
        # Give the compositor and the software renderers time to present the state just reached.
        time.sleep(settle)
        path = self.out / f'{name}.png'
        self.qmp.screenshot(path, head)
        self.record.setdefault('screenshots', []).append(path.name)

    def expect(self, name: str, passed: bool, detail: Any = '', required: bool = True) -> bool:
        """Record a check the harness evaluated itself (not a guest command)."""
        entry = {'name': name, 'status': 'passed' if passed else 'failed', 'detail': detail}
        self.record['checks'].append(entry)
        print(f"{entry['status'].upper():6} {name}: {str(detail)[:140]}", flush=True)
        if required and not passed:
            self.save()
            raise RuntimeError(f'check failed: {name}')
        return passed

    def state(self) -> dict:
        """The compositor's state from its development-only debug socket."""
        code, output = self.serial.run(
            'curl -s --max-time 5 --unix-socket /run/user/$(id -u amity)/amity-comp-debug.sock http://localhost/state', 30)
        if code != 0:
            raise RuntimeError(f'compositor state unavailable (curl exit {code}): {output[-300:]}')
        return json.loads(output)

    def wait_state(self, description: str, predicate, timeout: float = 20.0, required: bool = True) -> dict:
        """Poll the compositor state until `predicate(state)` holds."""
        started = time.monotonic()
        deadline = started + timeout
        state: dict = {}
        while True:
            try:
                state = self.state()
                if predicate(state):
                    self.expect(description, True, f'after {time.monotonic() - started:.1f} s')
                    return state
            except (RuntimeError, ValueError, LookupError, StopIteration, TypeError):
                pass  # no answer yet, or the state does not have what the predicate looks for yet
            if time.monotonic() > deadline:
                self.expect(description, False, {'last_state': state}, required=required)
                return state
            time.sleep(0.5)

    def save(self) -> None:
        (self.out / 'results.json').write_text(json.dumps(self.record, indent=2) + '\n', encoding='utf-8')


def as_user(command: str) -> str:
    """Run a command in the amity user's Wayland session."""
    return ("cd /home/amity && runuser -u amity -- env XDG_RUNTIME_DIR=/run/user/$(id -u amity) "
            "WAYLAND_DISPLAY=$(basename $(ls /run/user/$(id -u amity)/wayland-? | head -1)) "
            "DISPLAY=:$(ls /tmp/.X11-unix 2>/dev/null | head -1 | tr -d X) "
            "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u amity)/bus " + command)


def phase2(recorder: Recorder, install_apps: bool) -> None:
    check = recorder.check
    check('os-release is AmityOS', 'cat /etc/os-release', r'NAME="AmityOS"')
    check('kernel', 'uname -r')
    check('boot finished', 'systemctl is-system-running --wait || systemctl --failed --no-legend', timeout=300, required=False)
    check('root filesystem is btrfs and was grown', "findmnt -no FSTYPE,SIZE /; lsblk -bno SIZE /dev/vda | head -1", r'btrfs')
    check('graphical session active', 'systemctl is-active amity-session.service', r'^active')
    check('wayland socket present', 'ls /run/user/$(id -u amity)/ | grep wayland', r'wayland-\d')
    check('network address', "ip -4 -o addr show scope global | awk '{print $2, $4}'", r'\d+\.\d+\.\d+\.\d+')
    check('NetworkManager connected', 'nmcli -t -f STATE general', r'connected')
    check('DNS and HTTPS', "curl -sS -o /dev/null -w '%{http_code}' https://archive.ubuntu.com/ubuntu/dists/resolute/Release", r'200', timeout=60)
    check('Flathub remote configured', 'flatpak remotes --system --columns=name,url', r'flathub', required=False)
    recorder.screenshot('01-session')
    if not install_apps:
        return
    check('apt update', 'apt-get update -q 2>&1 | tail -2', timeout=600)
    check('download Chrome', "curl -fsSL -o /tmp/chrome.deb https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb && ls -l /tmp/chrome.deb", timeout=900)
    check('install Chrome', 'DEBIAN_FRONTEND=noninteractive apt-get install -y -q /tmp/chrome.deb 2>&1 | tail -3', timeout=1800)
    check('Chrome version', 'google-chrome-stable --version', r'Google Chrome \d+')
    check('download VS Code', "curl -fsSL -o /tmp/code.deb 'https://code.visualstudio.com/sha/download?build=stable&os=linux-deb-x64' && ls -l /tmp/code.deb", timeout=900)
    check('install VS Code', 'DEBIAN_FRONTEND=noninteractive apt-get install -y -q /tmp/code.deb 2>&1 | tail -3', timeout=1800)
    check('VS Code version', as_user('code --version'), r'^\d+\.\d+\.\d+')
    check('launch Chrome', as_user("sh -c 'google-chrome-stable --no-first-run --no-default-browser-check about:version >/tmp/chrome.log 2>&1 &'") + '; sleep 25; pgrep -u amity -f "chrome" | wc -l', r'^[1-9]', timeout=120)
    recorder.screenshot('02-chrome')
    check('Chrome window on Wayland or XWayland', 'sleep 5; pgrep -u amity -af "type=renderer" | wc -l', r'^[1-9]', required=False)
    check('close Chrome', 'pkill -u amity -f chrome; sleep 3; true')
    launched, _ = check('launch VS Code', as_user("sh -c 'code --new-window >/tmp/code.log 2>&1 &'") + '; sleep 30; pgrep -u amity -f "/usr/share/code/code" | wc -l', r'^[1-9]', timeout=120, required=False)
    recorder.screenshot('03-vscode')
    if not launched:
        check('VS Code failure log', 'cat /tmp/code.log | tail -20', required=False)
        recorder.save()
        raise RuntimeError('check failed: launch VS Code')
    check('VS Code log', 'tail -5 /tmp/code.log', required=False)
    check('close VS Code', 'pkill -u amity -f /usr/share/code/code; sleep 2; true')


def serve_directory(directory: Path) -> str:
    """Serve build outputs to the guest. The server listens on host loopback
    only; the guest reaches it through QEMU user networking at 10.0.2.2."""
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(directory))
    handler.log_message = lambda *args: None  # type: ignore[method-assign]
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return f'http://10.0.2.2:{server.server_address[1]}'


def launch(command: str, log: str) -> str:
    """Start a program in the amity user's session without waiting for it."""
    return as_user(f"sh -c '{command} >/tmp/{log}.log 2>&1 &'")


def window(state: dict, window_id: int) -> dict:
    return next(w for w in state['windows'] if w['id'] == window_id)


def switch_to(recorder: Recorder, window_id: int) -> None:
    """Pick a window in the Alt+Tab switcher, however many steps away it is."""
    qmp = recorder.qmp
    qmp.key('alt', True)
    try:
        for _ in range(12):
            qmp.key('tab', True)
            qmp.key('tab', False)
            time.sleep(0.5)
            switcher = recorder.state()['switcher']
            if switcher and switcher['candidates'][switcher['selected']] == window_id:
                break
    finally:
        qmp.key('alt', False)


def start_session(recorder: Recorder, host_url: str | None) -> dict:
    """Start the graphical session on the compositor under test and return its first state.

    With `host_url`, the binaries built on the host replace the ones in the
    image first, which avoids an image rebuild while iterating.
    """
    check, expect = recorder.check, recorder.expect
    recorder.record['method'] = ('root serial console; QMP input-send-event through the virtio keyboard and tablet; '
                                 'compositor state from its development debug socket; QMP screendump')
    check('kernel command line', 'cat /proc/cmdline', required=False)
    if host_url:
        install_build(recorder, host_url)
    else:
        check('no build override in /usr/local/bin', 'ls /usr/local/bin/ | grep -c amity; true', r'^0$')
    check('compositor and shell are installed', 'command -v amity-comp amity-shell', r'amity-shell')
    check('runtime libraries resolve',
          'for b in amity-comp amity-shell; do ldd "$(command -v $b)" | grep "not found"; done | wc -l', r'^0$')
    check('start the session on amity-comp',
          'systemctl stop amity-session.service; loginctl terminate-user amity 2>/dev/null; sleep 2; '
          'systemctl reset-failed amity-session.service 2>/dev/null; date "+%Y-%m-%d %H:%M:%S" > /run/amity-test-start; '
          'systemctl start amity-session.service; '
          'sleep 5; systemctl is-active amity-session.service', r'^active', timeout=60)
    check('amity-comp is the session compositor', 'pgrep -u amity -x amity-comp | wc -l', r'^1$')
    state = recorder.wait_state('compositor answers on its debug socket', lambda s: len(s['outputs']) >= 1)
    expect('XWayland display is up', state['x11_display'] is not None, state['x11_display'])
    return recorder.wait_state('shell is running', lambda s: s['shell_running'], 15)


def install_build(recorder: Recorder, host_url: str) -> None:
    check = recorder.check
    check('install the build under test',
          f'for b in amity-comp amity-shell; do curl -fsS -o /usr/local/bin/$b.new {host_url}/$b && '
          'chmod 755 /usr/local/bin/$b.new && mv -f /usr/local/bin/$b.new /usr/local/bin/$b || exit 1; done; '
          'mkdir -p /usr/share/amityos/wallpapers && '
          f'curl -fsS -o /usr/share/amityos/wallpapers/amity-default.png {host_url}/amity-default.png; '
          'sync; ls -l /usr/local/bin/amity-comp /usr/local/bin/amity-shell | cut -c1-90', timeout=180)
    # Images built before the debug socket was part of the development profile.
    check('development session settings',
          'mkdir -p /etc/systemd/system/amity-session.service.d && '
          "printf '[Service]\\nEnvironment=AMITY_COMP_DEBUG_SOCKET=1\\n' "
          '> /etc/systemd/system/amity-session.service.d/devel.conf && systemctl daemon-reload')


def session_is_clean(recorder: Recorder) -> None:
    recorder.check('compositor and shell did not crash or restart',
                   'journalctl -b --no-pager -o cat -t amity-session --since "$(cat /run/amity-test-start)" '
                   '| grep -c -E "panicked|RUST_BACKTRACE|the shell exited|the shell closed its channel"; '
                   'systemctl show amity-session.service -p NRestarts --value', r'^0\n0$')


def phase3(recorder: Recorder, host_url: str | None) -> None:
    """amity-comp on DRM/KMS in the VM, driven through the virtual keyboard and pointer."""
    check, expect, qmp = recorder.check, recorder.expect, recorder.qmp
    state = start_session(recorder, host_url)
    output = state['outputs'][0]
    size = (output['geometry'][2], output['geometry'][3])
    area = output['work_area']
    recorder.record['output'] = output
    expect('one output at the VM resolution', size[0] >= 1024 and size[1] >= 720, output)
    expect('taskbar reserves space at the bottom',
           area[3] < size[1] and area[1] == 0 and area[2] == size[0], {'work_area': area, 'output': size})
    recorder.screenshot('10-session')

    # --- window placement and focus
    check('launch a terminal', launch('foot --app-id=term.one', 'foot1'))
    state = recorder.wait_state('first window is mapped and focused',
                                lambda s: len(s['windows']) == 1 and s['windows'][0]['geometry'] and s['active_window'])
    first = state['windows'][0]
    expect('first window: identity', first['app_id'] == 'term.one', first)
    expect('first window has keyboard focus', state['keyboard_focus'] == f"window:{first['id']}", state['keyboard_focus'])
    x, y, w, h = first['geometry']
    expect('first window lies inside the work area',
           x >= area[0] and y >= area[1] and x + w <= area[0] + area[2] and y + h <= area[1] + area[3], first['geometry'])
    expect('first window is at the first cascade position', (x, y) == (area[0] + 130, area[1] + 68), first['geometry'])

    check('launch a second terminal', launch('foot --app-id=term.two', 'foot2'))
    state = recorder.wait_state('second window is mapped and focused',
                                lambda s: len(s['windows']) == 2 and s['windows'][0]['app_id'] == 'term.two'
                                and s['windows'][0]['geometry'])
    second = state['windows'][0]
    expect('new window takes focus and leads the use order',
           state['active_window'] == second['id'] and state['keyboard_focus'] == f"window:{second['id']}", state['keyboard_focus'])
    expect('second window is cascaded', tuple(second['geometry'][:2]) == (area[0] + 168, area[1] + 102), second['geometry'])
    recorder.screenshot('11-two-windows')

    # --- click to focus
    qmp.click(first['geometry'][0] + 12, first['geometry'][1] + first['geometry'][3] - 12, size)
    state = recorder.wait_state('click focuses the window under the pointer', lambda s: s['active_window'] == first['id'], 5)
    expect('click raises the window in the use order', state['windows'][0]['id'] == first['id'], [w['id'] for w in state['windows']])

    # --- MRU switcher
    qmp.chord('alt', 'tab')
    state = recorder.wait_state('Alt+Tab switches to the previous window', lambda s: s['active_window'] == second['id'], 5)
    expect('switcher is closed after Alt is released', state['switcher'] is None, state['switcher'])
    qmp.key('alt', True)
    qmp.key('tab', True)
    qmp.key('tab', False)
    state = recorder.wait_state('switcher opens while Alt is held', lambda s: s['switcher'] is not None, 5)
    expect('switcher lists windows in use order and selects the previous one',
           state['switcher'] == {'candidates': [second['id'], first['id']], 'selected': 1}, state['switcher'])
    expect('nothing changes while the switcher is open', state['active_window'] == second['id'], state['active_window'])
    recorder.screenshot('12-switcher')
    qmp.key('tab', True)
    qmp.key('tab', False)
    state = recorder.wait_state('Tab steps and wraps around', lambda s: s['switcher'] and s['switcher']['selected'] == 0, 5)
    qmp.key('shift', True)
    qmp.key('tab', True)
    qmp.key('tab', False)
    qmp.key('shift', False)
    state = recorder.wait_state('Shift+Tab steps back', lambda s: s['switcher'] and s['switcher']['selected'] == 1, 5)
    qmp.key('esc', True)
    qmp.key('esc', False)
    state = recorder.wait_state('Escape cancels the switcher', lambda s: s['switcher'] is None, 5)
    qmp.key('alt', False)
    time.sleep(0.3)
    state = recorder.state()
    expect('cancelling leaves focus unchanged', state['active_window'] == second['id'], state['active_window'])

    # --- maximize, snap, restore
    before = window(state, second['id'])['geometry']
    qmp.chord('meta_l', 'up')
    state = recorder.wait_state('Super+Up maximizes', lambda s: window(s, second['id'])['tiling'] == 'maximized', 5)
    state = recorder.wait_state('maximized window fills the work area',
                                lambda s: window(s, second['id'])['geometry'] == area, 5)
    recorder.screenshot('13-maximized')
    qmp.chord('meta_l', 'down')
    state = recorder.wait_state('Super+Down restores the previous geometry',
                                lambda s: window(s, second['id'])['tiling'] == 'floating'
                                and window(s, second['id'])['geometry'] == before, 5)
    half = area[2] // 2
    qmp.chord('meta_l', 'left')
    state = recorder.wait_state('Super+Left snaps to the left half',
                                lambda s: window(s, second['id'])['geometry'] == [area[0], area[1], half, area[3]], 5)
    qmp.chord('meta_l', 'right')
    state = recorder.wait_state('Super+Right snaps to the right half',
                                lambda s: window(s, second['id'])['geometry'] == [area[0] + half, area[1], area[2] - half, area[3]], 5)
    recorder.screenshot('14-snapped')
    qmp.chord('meta_l', 'up')
    state = recorder.wait_state('a snapped window can be maximized', lambda s: window(s, second['id'])['geometry'] == area, 5)
    qmp.chord('meta_l', 'down')
    state = recorder.wait_state('and returns to its floating geometry',
                                lambda s: window(s, second['id'])['geometry'] == before, 5)

    # --- title bar controls (server-side decoration)
    geometry = window(state, second['id'])
    expect('terminal has a compositor-drawn title bar', geometry['server_side_decoration'], geometry)
    x, y, w, h = geometry['geometry']
    qmp.click(x + w - 57, y + 18, size)
    state = recorder.wait_state('title bar: maximize button', lambda s: window(s, second['id'])['geometry'] == area, 5)
    qmp.click(area[0] + area[2] - 57, area[1] + 18, size)
    state = recorder.wait_state('title bar: restore button', lambda s: window(s, second['id'])['geometry'] == before, 5)
    qmp.click(x + w - 95, y + 18, size)
    state = recorder.wait_state('title bar: minimize button', lambda s: window(s, second['id'])['minimized'], 5)
    expect('minimizing moves focus to the next window', state['active_window'] == first['id'], state['active_window'])
    expect('a minimized window stays in the window list', len(state['windows']) == 2, len(state['windows']))
    qmp.chord('alt', 'tab')
    state = recorder.wait_state('Alt+Tab restores a minimized window',
                                lambda s: not window(s, second['id'])['minimized'] and s['active_window'] == second['id'], 5)
    expect('restored window is back where it was', window(state, second['id'])['geometry'] == before, window(state, second['id']))

    # drag by the title bar
    qmp.pointer(x + 60, y + 18, size)
    qmp.button(True)
    for step in range(1, 6):
        qmp.pointer(x + 60 + step * 20, y + 18 + step * 10, size)
    qmp.button(False)
    state = recorder.wait_state('dragging the title bar moves the window',
                                lambda s: window(s, second['id'])['geometry'] == [x + 100, y + 50, w, h], 5)
    recorder.screenshot('15-moved')

    # resize by the frame: the bottom-right corner, then the left edge. The terminal
    # rounds its size to whole character cells, hence the tolerance.
    x2, y2 = x + 100, y + 50
    qmp.pointer(x2 + w - 2, y2 + h - 2, size)
    qmp.button(True)
    for step in range(1, 5):
        qmp.pointer(x2 + w - 2 + step * 15, y2 + h - 2 + step * 10, size)
    qmp.button(False)

    def grew(geometry: list) -> bool:
        return (geometry[:2] == [x2, y2] and abs(geometry[2] - (w + 60)) <= 16 and abs(geometry[3] - (h + 40)) <= 24
                and geometry[2] > w and geometry[3] > h)

    state = recorder.wait_state('dragging the bottom-right corner of the frame resizes the window',
                                lambda s: grew(window(s, second['id'])['geometry']), 10)
    time.sleep(1)
    rx, ry, rw, rh = window(recorder.state(), second['id'])['geometry']
    expect('the top-left corner stayed in place', [rx, ry] == [x2, y2], {'before': [x2, y2, w, h], 'after': [rx, ry, rw, rh]})
    qmp.pointer(rx + 2, ry + 300, size)
    qmp.button(True)
    for step in range(1, 5):
        qmp.pointer(rx + 2 - step * 10, ry + 300, size)
    qmp.button(False)

    def widened(geometry: list) -> bool:
        return (geometry[0] + geometry[2] == rx + rw and abs(geometry[2] - (rw + 40)) <= 16 and geometry[2] > rw
                and geometry[1] == ry and geometry[3] == rh)

    state = recorder.wait_state('dragging the left edge moves that edge and keeps the right one in place',
                                lambda s: widened(window(s, second['id'])['geometry']), 10)
    recorder.record['frame_resize'] = {'start': [x2, y2, w, h], 'after_corner': [rx, ry, rw, rh],
                                       'after_left_edge': window(state, second['id'])['geometry']}
    recorder.screenshot('15-resized')

    # --- close
    qmp.chord('ctrl', 'q')
    state = recorder.wait_state('Ctrl+Q closes the active window', lambda s: len(s['windows']) == 1, 10)
    expect('focus moves to the remaining window',
           state['active_window'] == first['id'] and state['keyboard_focus'] == f"window:{first['id']}", state['keyboard_focus'])
    x, y, w, h = window(state, first['id'])['geometry']
    qmp.click(x + w - 19, y + 18, size)
    state = recorder.wait_state('title bar: close button', lambda s: len(s['windows']) == 0, 10)
    expect('no window has focus when none is left', state['active_window'] is None, state['keyboard_focus'])
    check('terminals exited', 'sleep 1; pgrep -u amity -x foot | wc -l', r'^0$')

    # --- launcher
    bar_y = area[1] + area[3] + 31                     # middle of the 62 px taskbar
    qmp.chord('meta_l', 'spc')
    state = recorder.wait_state('Super+Space opens the launcher',
                                lambda s: s['keyboard_focus'] == 'layer:amity-launcher', 10)
    qmp.chord('meta_l', 'spc')
    state = recorder.wait_state('Super+Space closes it again', lambda s: s['keyboard_focus'] == 'none', 10)
    qmp.click(area[0] + 50, bar_y, size)
    state = recorder.wait_state('the taskbar button opens the launcher',
                                lambda s: s['keyboard_focus'] == 'layer:amity-launcher', 10)
    qmp.type_text('foot')
    recorder.screenshot('16-launcher')
    qmp.chord('ret')
    state = recorder.wait_state('Enter starts the selected application',
                                lambda s: len(s['windows']) == 1 and s['windows'][0]['app_id'] == 'foot'
                                and s['windows'][0]['geometry'], 30)
    launched = state['windows'][0]
    state = recorder.wait_state('the launcher closes and the new window has the keyboard',
                                lambda s: s['keyboard_focus'] == f"window:{launched['id']}", 10)
    check('the application runs as its own systemd user service',
          as_user("systemctl --user list-units --no-legend --plain 'app-AmityOS-foot-*'"),
          r'app-AmityOS-foot-\S+\.service\s+loaded\s+active\s+running')
    check('the application sees the session displays',
          'tr "\\0" "\\n" < /proc/$(pgrep -u amity -x foot | head -1)/environ | grep -E "^(WAYLAND_DISPLAY|DISPLAY|XDG_CURRENT_DESKTOP)=" | sort | tr "\\n" " "',
          r'DISPLAY=:\d+ WAYLAND_DISPLAY=wayland-\d+ XDG_CURRENT_DESKTOP=AmityOS')
    check('the private sockets of the shell are not passed on to applications',
          'tr "\\0" "\\n" < /proc/$(pgrep -u amity -x foot | head -1)/environ | grep -c -E "^(AMITY_SHELL_FD|WAYLAND_SOCKET)="; true',
          r'^0$')

    # --- taskbar entry (shell-behaviour.md 2.6, one window)
    qmp.click(area[0] + 119, bar_y, size)
    state = recorder.wait_state('a press on the entry of the focused window minimizes it',
                                lambda s: window(s, launched['id'])['minimized'], 10)
    recorder.screenshot('17-minimized')
    qmp.click(area[0] + 119, bar_y, size)
    state = recorder.wait_state('a press on the entry of a minimized window restores and focuses it',
                                lambda s: not window(s, launched['id'])['minimized']
                                and s['keyboard_focus'] == f"window:{launched['id']}", 10)

    # --- the launcher hands the keyboard back
    qmp.chord('meta_l', 'spc')
    state = recorder.wait_state('launcher opens over a window', lambda s: s['keyboard_focus'] == 'layer:amity-launcher', 10)
    expect('the window stays the active window while the launcher is open',
           state['active_window'] == launched['id'], state['active_window'])
    qmp.chord('esc')
    state = recorder.wait_state('Escape closes the launcher and returns the keyboard to the window',
                                lambda s: s['keyboard_focus'] == f"window:{launched['id']}", 10)
    qmp.chord('ctrl', 'q')
    state = recorder.wait_state('the launched application closes', lambda s: len(s['windows']) == 0, 10)

    # --- fullscreen
    check('launch a terminal that starts fullscreen', launch('foot --fullscreen --app-id=term.full', 'foot4'))
    state = recorder.wait_state('a window that asks for it opens fullscreen on the whole monitor',
                                lambda s: len(s['windows']) == 1 and s['windows'][0]['fullscreen']
                                and s['windows'][0]['geometry'] == output['geometry'], 20)
    full = state['windows'][0]
    state = recorder.wait_state('while it is the active window it covers the monitor, taskbar included',
                                lambda s: s['outputs'][0]['covered_by'] == full['id'], 10)
    expect('a fullscreen window has no title bar', not window(state, full['id'])['server_side_decoration'], full)
    recorder.screenshot('19-fullscreen')
    check('launch a normal terminal while it is fullscreen', launch('foot --app-id=term.side', 'foot5'))
    state = recorder.wait_state('the new window takes focus and the monitor is no longer covered',
                                lambda s: len(s['windows']) == 2 and s['windows'][0]['app_id'] == 'term.side'
                                and s['windows'][0]['geometry'] and s['active_window'] == s['windows'][0]['id']
                                and s['outputs'][0]['covered_by'] is None, 20)
    side = state['windows'][0]
    expect('the other window is still fullscreen, behind it', window(state, full['id'])['fullscreen'], window(state, full['id']))
    recorder.screenshot('19-fullscreen-behind')
    qmp.chord('alt', 'tab')
    state = recorder.wait_state('Alt+Tab back to the fullscreen window covers the monitor again',
                                lambda s: s['active_window'] == full['id'] and s['outputs'][0]['covered_by'] == full['id'], 10)
    qmp.chord('meta_l', 'spc')
    state = recorder.wait_state('the launcher opens over a fullscreen window and is not hidden by it',
                                lambda s: s['keyboard_focus'] == 'layer:amity-launcher'
                                and s['outputs'][0]['covered_by'] is None, 10)
    qmp.type_text('q')
    time.sleep(0.5)
    qmp.chord('ctrl', 'q')
    time.sleep(1)
    state = recorder.state()
    expect('Ctrl+Q typed into the launcher does not close the window behind it',
           len(state['windows']) == 2 and state['keyboard_focus'] == 'layer:amity-launcher',
           [w['app_id'] for w in state['windows']])
    recorder.screenshot('19-launcher-over-fullscreen')
    qmp.chord('esc')
    state = recorder.wait_state('closing the launcher returns to the fullscreen window',
                                lambda s: s['keyboard_focus'] == f"window:{full['id']}"
                                and s['outputs'][0]['covered_by'] == full['id'], 10)
    qmp.chord('meta_l', 'up')
    state = recorder.wait_state('Super+Up takes the window out of fullscreen and maximizes it',
                                lambda s: not window(s, full['id'])['fullscreen']
                                and window(s, full['id'])['tiling'] == 'maximized'
                                and window(s, full['id'])['geometry'] == area
                                and s['outputs'][0]['covered_by'] is None, 10)
    expect('its title bar is back', window(state, full['id'])['server_side_decoration'], window(state, full['id']))
    qmp.chord('ctrl', 'q')
    state = recorder.wait_state('it closes', lambda s: len(s['windows']) == 1, 10)
    qmp.chord('ctrl', 'q')
    state = recorder.wait_state('and so does the other terminal', lambda s: len(s['windows']) == 0, 10)
    _ = side

    # --- virtual terminal switch: the session gives up the display and input, then takes them back
    qmp.chord('ctrl', 'alt', 'f2')
    switched, _ = check('Ctrl+Alt+F2 switches to another virtual terminal', 'sleep 2; cat /sys/class/tty/tty0/active', r'^tty2$', required=False)
    qmp.chord('ctrl', 'alt', 'f1')
    check('Ctrl+Alt+F1 returns to the session', 'sleep 2; cat /sys/class/tty/tty0/active', r'^tty1$', required=False)
    if switched:
        check('launch a terminal after the switch', launch('foot --app-id=term.vt', 'foot3'))
        state = recorder.wait_state('the session works again after the switch: a new window maps and takes focus',
                                    lambda s: len(s['windows']) == 1 and s['windows'][0]['geometry']
                                    and s['keyboard_focus'] == f"window:{s['windows'][0]['id']}", 20)
        recorder.screenshot('18-after-vt-switch')
        qmp.chord('ctrl', 'q')
        recorder.wait_state('and closes on Ctrl+Q', lambda s: len(s['windows']) == 0, 10)
    session_is_clean(recorder)


def open_from_launcher(recorder: Recorder, query: str, app_id: str, timeout: float) -> dict:
    """Start an application the way a user does: Super+Space, type, Enter."""
    qmp = recorder.qmp
    before = {w['id'] for w in recorder.state()['windows']}
    qmp.chord('meta_l', 'spc')
    recorder.wait_state(f'launcher opens for "{query}"', lambda s: s['keyboard_focus'] == 'layer:amity-launcher', 10)
    qmp.type_text(query)
    time.sleep(1)
    qmp.chord('ret')
    matches = re.compile(app_id, re.I)
    state = recorder.wait_state(f'a window of {app_id} appears',
                                lambda s: any(w['id'] not in before and matches.search(w['app_id']) and w['geometry']
                                              for w in s['windows']), timeout)
    return next(w for w in state['windows'] if w['id'] not in before and matches.search(w['app_id']))


def close_everything(recorder: Recorder, name: str, process: str) -> None:
    """Close the active application with the close shortcut; fall back to ending the process."""
    deadline = time.monotonic() + 30
    while recorder.state()['windows'] and time.monotonic() < deadline:
        recorder.qmp.chord('ctrl', 'q')
        time.sleep(3)
    closed = not recorder.state()['windows']
    recorder.expect(f'{name} closes on Ctrl+Q', closed, recorder.state()['windows'], required=False)
    recorder.check(f'{name} processes ended', f'pkill -u amity -f "{process}"; sleep 3; pgrep -u amity -f "{process}" | wc -l',
                   r'^0$', required=False)
    recorder.wait_state('no windows are left', lambda s: not s['windows'], 20)


def minimize_and_restore(recorder: Recorder, name: str, window_id: int) -> None:
    """Minimize the only window from its taskbar entry and bring it back."""
    output = recorder.state()['outputs'][0]
    size = (output['geometry'][2], output['geometry'][3])
    bar_y = output['work_area'][1] + output['work_area'][3] + 31
    recorder.qmp.click(output['geometry'][0] + 119, bar_y, size)
    recorder.wait_state(f'{name}: its taskbar entry minimizes the window',
                        lambda s: window(s, window_id)['minimized'], 10, required=False)
    recorder.qmp.click(output['geometry'][0] + 119, bar_y, size)
    recorder.wait_state(f'{name}: and restores it with focus',
                        lambda s: not window(s, window_id)['minimized'] and s['active_window'] == window_id, 10,
                        required=False)


def phase3_apps(recorder: Recorder) -> None:
    """Chrome, VS Code and Blender started from the launcher on amity-comp (needs the network)."""
    check, expect = recorder.check, recorder.expect
    check('DNS and HTTPS', "curl -sS -o /dev/null -w '%{http_code}' https://archive.ubuntu.com/ubuntu/dists/resolute/Release", r'200', timeout=60)
    check('apt update', 'apt-get update -q 2>&1 | tail -2', timeout=600)
    check('install Chrome', "command -v google-chrome-stable >/dev/null || { curl -fsSL -o /tmp/chrome.deb "
          "https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb && "
          "DEBIAN_FRONTEND=noninteractive apt-get install -y -q /tmp/chrome.deb 2>&1 | tail -3; }; google-chrome-stable --version",
          r'Google Chrome \d+', timeout=2400)
    check('install VS Code', "command -v code >/dev/null || { curl -fsSL -o /tmp/code.deb "
          "'https://code.visualstudio.com/sha/download?build=stable&os=linux-deb-x64' && "
          "DEBIAN_FRONTEND=noninteractive apt-get install -y -q /tmp/code.deb 2>&1 | tail -3; }; " + as_user('code --version'),
          r'\d+\.\d+\.\d+', timeout=2400)
    check('install Blender from Flathub',
          'flatpak info --system org.blender.Blender >/dev/null 2>&1 || '
          'flatpak install --system --noninteractive -y flathub org.blender.Blender 2>&1 | tail -3; '
          'flatpak info --system org.blender.Blender | grep -E "^ *(ID|Version|Runtime):" | tr -s " " | tr "\\n" ";"',
          r'org\.blender\.Blender', timeout=3600)
    # No first-run dialog, default-browser prompt or keyring prompt in front of the window.
    check('prepare Chrome profile', as_user("sh -c 'mkdir -p ~/.config/google-chrome && touch \"$HOME/.config/google-chrome/First Run\"'"))

    chrome = open_from_launcher(recorder, 'chrome', r'chrome', 90)
    recorder.screenshot('20-chrome', settle=20)
    chrome = window(recorder.state(), chrome['id'])
    recorder.record['chrome_window'] = chrome
    area = recorder.state()['outputs'][0]['work_area']
    cx, cy, cw, ch = chrome['geometry']
    expect('Chrome opened inside the work area, not under the taskbar',
           cx >= area[0] and cy >= area[1] and cx + cw <= area[0] + area[2] and cy + ch <= area[1] + area[3],
           {'window': chrome['geometry'], 'work_area': area}, required=False)
    minimize_and_restore(recorder, 'Chrome', chrome['id'])
    check('Chrome renders pages', 'pgrep -u amity -af "type=renderer" | wc -l', r'^[1-9]', required=False)
    check('Chrome runs in its own unit', as_user("systemctl --user list-units --no-legend --plain 'app-AmityOS-google*'"),
          r'app-AmityOS-google\S*\.service\s+loaded\s+active\s+running', required=False)
    close_everything(recorder, 'Chrome', '/opt/google/chrome/chrome')

    code = open_from_launcher(recorder, 'visual studio', r'code', 120)
    recorder.screenshot('21-vscode', settle=25)
    code = window(recorder.state(), code['id'])
    recorder.record['vscode_window'] = code
    minimize_and_restore(recorder, 'VS Code', code['id'])
    recorder.screenshot('21-vscode-restored', settle=5)
    check('VS Code processes', 'pgrep -u amity -f "/usr/share/code/code" | wc -l', r'^[1-9]')
    close_everything(recorder, 'VS Code', '/usr/share/code/code')

    blender = open_from_launcher(recorder, 'blender', r'blender', 240)
    recorder.screenshot('22-blender', settle=40)
    blender = window(recorder.state(), blender['id'])
    recorder.record['blender_window'] = blender
    check('Blender process', 'pgrep -u amity -x blender | wc -l', r'^[1-9]')
    check('Blender runs in the Flatpak sandbox', as_user('flatpak ps --columns=application') + ' | grep -c org.blender.Blender; true',
          r'^[1-9]', required=False)
    close_everything(recorder, 'Blender', 'blender')
    phase3_x11(recorder)
    recorder.record['applications_used_x11'] = {'chrome': chrome['x11'], 'vscode': code['x11'], 'blender': blender['x11']}
    session_is_clean(recorder)


def x11_window(recorder: Recorder, name: str, app_id: str, timeout: float) -> dict:
    matches = re.compile(app_id, re.I)
    state = recorder.wait_state(f'{name} opens an X11 window',
                                lambda s: any(w['x11'] and matches.search(w['app_id']) and w['geometry'] for w in s['windows']),
                                timeout)
    found = next(w for w in state['windows'] if w['x11'] and matches.search(w['app_id']))
    recorder.wait_state(f'{name}: the X11 window has the keyboard',
                        lambda s: s['keyboard_focus'] == f"window:{found['id']}", 20)
    return found


def phase3_x11(recorder: Recorder) -> None:
    """X11 applications through XWayland: one that draws its own frame, one with the compositor's title bar."""
    check, expect, qmp = recorder.check, recorder.expect, recorder.qmp
    output = recorder.state()['outputs'][0]
    size = (output['geometry'][2], output['geometry'][3])
    area = output['work_area']

    # Chrome as an X11 client. It draws its own frame.
    check('start Chrome as an X11 application',
          launch('google-chrome-stable --ozone-platform=x11 --no-first-run --no-default-browser-check about:blank', 'chrome-x11'))
    chrome = x11_window(recorder, 'Chrome (X11)', r'chrome', 120)
    recorder.screenshot('23-chrome-x11', settle=15)
    chrome = window(recorder.state(), chrome['id'])
    recorder.record['chrome_x11_window'] = chrome
    minimize_and_restore(recorder, 'Chrome (X11)', chrome['id'])
    before = window(recorder.state(), chrome['id'])['geometry']
    qmp.chord('meta_l', 'up')
    recorder.wait_state('Chrome (X11): Super+Up maximizes it to the work area',
                        lambda s: window(s, chrome['id'])['geometry'] == area, 15)
    qmp.chord('meta_l', 'down')
    recorder.wait_state('Chrome (X11): Super+Down restores its previous geometry',
                        lambda s: window(s, chrome['id'])['geometry'] == before, 15)
    close_everything(recorder, 'Chrome (X11)', '/opt/google/chrome/chrome')

    # VS Code as an X11 client that asks the window manager for a frame.
    check('VS Code setting: title bar from the window manager',
          "mkdir -p /home/amity/.config/Code/User && "
          "echo '{\"window.titleBarStyle\": \"native\"}' > /home/amity/.config/Code/User/settings.json && "
          "chown -R amity:amity /home/amity/.config/Code && cat /home/amity/.config/Code/User/settings.json", r'native')
    try:
        check('start VS Code as an X11 application', launch('code --ozone-platform=x11 --new-window', 'code-x11'))
        code = x11_window(recorder, 'VS Code (X11)', r'code', 150)
        time.sleep(15)
        code = window(recorder.state(), code['id'])
        recorder.record['vscode_x11_window'] = code
        recorder.screenshot('24-vscode-x11')
        expect('VS Code (X11) has the compositor title bar', code['server_side_decoration'], code, required=False)
        if code['server_side_decoration']:
            if code['tiling'] != 'floating':
                qmp.chord('meta_l', 'down')
                recorder.wait_state('VS Code (X11): floating', lambda s: window(s, code['id'])['tiling'] == 'floating', 15)
                time.sleep(2)
            x, y, w, h = window(recorder.state(), code['id'])['geometry']
            # Move by the title bar. The application chooses where it opens (it remembers its
            # last window), so drag towards the side that has room: the window then stays
            # inside the work area and "restore" below has nothing to re-fit.
            room_left, room_right = x - area[0], area[0] + area[2] - (x + w)
            dx = -min(40, room_left) if room_left >= room_right else min(40, room_right)
            dy = -min(5, y - area[1])
            qmp.pointer(x + 200, y + 18, size)
            qmp.button(True)
            for step in range(1, 6):
                qmp.pointer(x + 200 + dx * step / 5, y + 18 + dy * step / 5, size)
            qmp.button(False)
            expect('VS Code (X11): there is room to drag the window', dx != 0, {'geometry': [x, y, w, h], 'work_area': area})
            state = recorder.wait_state('VS Code (X11): dragging the title bar moves the window',
                                        lambda s: window(s, code['id'])['geometry'][:2] == [x + dx, y + dy], 15)
            x, y, w, h = window(state, code['id'])['geometry']
            # resize by the frame: the left edge inwards by 30 px
            qmp.pointer(x + 2, y + 300, size)
            qmp.button(True)
            for step in range(1, 4):
                qmp.pointer(x + 2 + step * 10, y + 300, size)
            qmp.button(False)
            state = recorder.wait_state('VS Code (X11): dragging the left frame edge resizes it, right edge in place',
                                        lambda s: window(s, code['id'])['geometry'] == [x + 30, y, w - 30, h], 15)
            x, y, w, h = window(state, code['id'])['geometry']
            recorder.screenshot('24-vscode-x11-resized', settle=4)
            # title bar controls
            qmp.click(x + w - 57, y + 18, size)
            recorder.wait_state('VS Code (X11): the title-bar maximize button fills the work area',
                                lambda s: window(s, code['id'])['geometry'] == area, 15)
            qmp.click(area[0] + area[2] - 57, area[1] + 18, size)
            recorder.wait_state('VS Code (X11): the restore button returns it',
                                lambda s: window(s, code['id'])['geometry'] == [x, y, w, h], 15)
            qmp.click(x + w - 95, y + 18, size)
            recorder.wait_state('VS Code (X11): the minimize button minimizes it',
                                lambda s: window(s, code['id'])['minimized'], 15)
            switch_to(recorder, code['id'])
            recorder.wait_state('VS Code (X11): Alt+Tab restores it where it was',
                                lambda s: not window(s, code['id'])['minimized']
                                and window(s, code['id'])['geometry'] == [x, y, w, h], 15)
            recorder.screenshot('24-vscode-x11-restored', settle=5)
            qmp.click(x + w - 19, y + 18, size)
            recorder.wait_state('VS Code (X11): the title-bar close button closes it',
                                lambda s: not any(other['id'] == code['id'] for other in s['windows']), 20, required=False)
        close_everything(recorder, 'VS Code (X11)', '/usr/share/code/code')
    finally:
        check('VS Code setting removed', 'rm -f /home/amity/.config/Code/User/settings.json; true')
    check('XWayland is still running', 'pgrep -u amity -x Xwayland | wc -l', r'^1$')


def phase3_multi(recorder: Recorder, host_url: str | None, vnc_port: int) -> None:
    """Two outputs: a second display head is connected, used and disconnected again."""
    check, expect, qmp = recorder.check, recorder.expect, recorder.qmp
    state = start_session(recorder, host_url)
    expect('starts with one output', len(state['outputs']) == 1, state['outputs'])
    first = state['outputs'][0]
    check('launch a terminal on the first output', launch('foot --app-id=term.one', 'foot1'))
    state = recorder.wait_state('window on the first output', lambda s: len(s['windows']) == 1 and s['windows'][0]['geometry'])
    one = state['windows'][0]

    head = Head(vnc_port)
    head.resize(1024, 768)
    state = recorder.wait_state('a second output appears when a monitor is connected', lambda s: len(s['outputs']) == 2, 30)
    second = next(o for o in state['outputs'] if o['name'] != first['name'])
    expect('the second output is placed to the right of the first',
           second['geometry'][:2] == [first['geometry'][2], 0] and second['geometry'][2:] == [1024, 768], second)
    expect('the first output keeps its taskbar; the second has the whole screen as work area',
           state['outputs'][0]['work_area'] == first['work_area'] and second['work_area'] == second['geometry'],
           [o['work_area'] for o in state['outputs']])
    expect('existing windows stay where they are', window(state, one['id'])['geometry'] == one['geometry'], window(state, one['id']))
    layout = (first['geometry'][2] + second['geometry'][2], max(first['geometry'][3], second['geometry'][3]))
    recorder.screenshot('30-output-1', head=0, settle=3)
    recorder.screenshot('31-output-2', head=1, settle=0)

    # A window opens on the output the pointer is on.
    qmp.pointer(second['geometry'][0] + 500, 300, layout)
    check('launch a terminal with the pointer on the second output', launch('foot --app-id=term.two', 'foot2'))
    state = recorder.wait_state('new window opens on the second output',
                                lambda s: len(s['windows']) == 2 and s['windows'][0]['output'] == second['name'], 20)
    two = state['windows'][0]
    x, y, w, h = two['geometry']
    expect('and lies inside it', x >= second['geometry'][0] and x + w <= second['geometry'][0] + second['geometry'][2]
           and y >= 0 and y + h <= second['geometry'][3], two['geometry'])
    qmp.chord('meta_l', 'up')
    state = recorder.wait_state('maximize fills the output the window is on',
                                lambda s: window(s, two['id'])['geometry'] == second['work_area'], 10)
    qmp.chord('meta_l', 'down')
    state = recorder.wait_state('restore', lambda s: window(s, two['id'])['geometry'] == two['geometry'], 10)

    # Drag the window from the second output to the first by its title bar.
    qmp.pointer(x + 60, y + 18, layout)
    qmp.button(True)
    for step in range(1, 9):
        qmp.pointer(x + 60 - step * 100, y + 18, layout)
    qmp.button(False)
    state = recorder.wait_state('a window can be dragged to the other output',
                                lambda s: window(s, two['id'])['output'] == first['name'], 10)
    expect('the dragged window moved by the drag distance', window(state, two['id'])['geometry'][0] == x - 800, window(state, two['id']))
    qmp.chord('meta_l', 'up')
    state = recorder.wait_state('maximize there fills the first output work area',
                                lambda s: window(s, two['id'])['geometry'] == first['work_area'], 10)
    qmp.chord('meta_l', 'down')
    recorder.wait_state('restore after the move', lambda s: window(s, two['id'])['tiling'] == 'floating', 10)

    # Move it back, then unplug the monitor it is on.
    qmp.pointer(x - 800 + 60, y + 18, layout)
    qmp.button(True)
    for step in range(1, 9):
        qmp.pointer(x - 800 + 60 + step * 100, y + 18, layout)
    qmp.button(False)
    state = recorder.wait_state('and dragged back', lambda s: window(s, two['id'])['output'] == second['name'], 10)

    # A third window on the second monitor, minimized there.
    qmp.pointer(second['geometry'][0] + 500, 300, layout)
    check('launch a third terminal on the second output', launch('foot --app-id=term.three', 'foot3'))
    state = recorder.wait_state('third window opens on the second output',
                                lambda s: len(s['windows']) == 3 and s['windows'][0]['app_id'] == 'term.three'
                                and s['windows'][0]['output'] == second['name'] and s['windows'][0]['geometry'], 20)
    three = state['windows'][0]
    tx, ty, tw, _th = three['geometry']
    qmp.click(tx + tw - 95, ty + 18, layout)
    state = recorder.wait_state('it is minimized; focus returns to the window used before',
                                lambda s: window(s, three['id'])['minimized'] and s['active_window'] == two['id'], 10)
    # The second window maximized on the second monitor.
    qmp.chord('meta_l', 'up')
    state = recorder.wait_state('the second window is maximized on the second output',
                                lambda s: window(s, two['id'])['geometry'] == second['work_area'], 10)
    recorder.screenshot('32-output-2-window', head=1)

    head.resize(0, 0)
    state = recorder.wait_state('the output disappears when the monitor is disconnected', lambda s: len(s['outputs']) == 1, 30)
    state = recorder.wait_state('a maximized window is maximized on the remaining monitor',
                                lambda s: window(s, two['id'])['tiling'] == 'maximized'
                                and window(s, two['id'])['geometry'] == first['work_area']
                                and window(s, two['id'])['output'] == first['name'], 10)
    expect('the first window did not move', window(state, one['id'])['geometry'] == one['geometry'], window(state, one['id']))
    expect('the minimized window is still minimized and listed', window(state, three['id'])['minimized'], window(state, three['id']))
    switch_to(recorder, three['id'])
    state = recorder.wait_state('restoring the window minimized on the lost monitor shows it on the remaining one',
                                lambda s: not window(s, three['id'])['minimized'] and s['active_window'] == three['id']
                                and window(s, three['id'])['output'] == first['name'], 10)
    rx, ry, rw, rh = window(state, three['id'])['geometry']
    expect('and it is fully inside the work area',
           rx >= first['work_area'][0] and ry >= first['work_area'][1]
           and rx + rw <= first['work_area'][0] + first['work_area'][2]
           and ry + rh <= first['work_area'][1] + first['work_area'][3], [rx, ry, rw, rh])
    expect('all three windows are still there', len(state['windows']) == 3, len(state['windows']))
    recorder.screenshot('33-after-unplug', head=0)
    check('close the terminals', 'pkill -u amity -x foot; sleep 2; pgrep -u amity -x foot | wc -l', r'^0$')
    session_is_clean(recorder)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial-port', type=int, required=True)
    parser.add_argument('--qmp-port', type=int, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--boot-timeout', type=int, default=240)
    parser.add_argument('--skip-apps', action='store_true')
    parser.add_argument('--suite', choices=['phase2', 'phase3', 'phase3-apps', 'phase3-multi', 'exec'], default='phase2')
    parser.add_argument('--vnc-port', type=int, default=0, help='VNC server of the second display head (phase3-multi)')
    parser.add_argument('--exec', action='append', default=[], dest='commands',
                        help='with --suite exec: a command to run on the guest (repeatable); {host} is the host URL')
    parser.add_argument('--screenshot', action='store_true', help='with --suite exec: take a screenshot at the end')
    parser.add_argument('--serve', type=Path, help='directory of build outputs to offer to the guest over HTTP')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    serial = Serial(args.serial_port, args.out / 'serial.log')
    qmp = Qmp(args.qmp_port)
    recorder = Recorder(args.out, serial, qmp)
    started = time.monotonic()
    try:
        serial.login(args.boot_timeout)
        recorder.record['boot_seconds_to_root_shell'] = round(time.monotonic() - started)
        if args.suite == 'phase2':
            phase2(recorder, not args.skip_apps)
        elif args.suite == 'exec':
            host = serve_directory(args.serve) if args.serve else ''
            for index, command in enumerate(args.commands):
                code, output = serial.run(command.replace('{host}', host), 600)
                print(f'--- [{index}] exit={code}: {command}', flush=True)
                print(output, flush=True)
            if args.screenshot:
                recorder.screenshot('exec')
        elif args.suite == 'phase3-apps':
            start_session(recorder, serve_directory(args.serve) if args.serve else None)
            phase3_apps(recorder)
        elif args.suite == 'phase3-multi':
            phase3_multi(recorder, serve_directory(args.serve) if args.serve else None, args.vnc_port)
        else:
            phase3(recorder, serve_directory(args.serve) if args.serve else None)
            if not args.skip_apps:
                phase3_apps(recorder)
        recorder.record['status'] = 'passed' if all(c['status'] == 'passed' for c in recorder.record['checks']) else 'passed-with-failed-optional-checks'
    except Exception as error:
        recorder.record['status'] = 'failed'
        recorder.record['error'] = f'{type(error).__name__}: {error}'
        try:
            recorder.screenshot('failure')
        except Exception:
            pass
        raise
    finally:
        recorder.save()
        try:
            serial.sock.sendall(b'systemctl poweroff\n')
        except OSError:
            pass


if __name__ == '__main__':
    main()
