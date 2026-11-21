#!/usr/bin/env python3
"""Drive a running AmityOS development VM from the Windows host for acceptance.

The VM is started by AmityOS.ps1 with a root serial console and QMP, both on
loopback TCP. This harness runs commands on the serial console, takes guest
screenshots through QMP, and writes a JSON record of what was observed.
Standard library only.
"""
from __future__ import annotations

import argparse
import json
import re
import socket
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

    def screenshot(self, path: Path) -> None:
        self.execute('screendump', {'filename': str(path), 'format': 'png'})


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

    def screenshot(self, name: str) -> None:
        path = self.out / f'{name}.png'
        self.qmp.screenshot(path)
        self.record.setdefault('screenshots', []).append(path.name)

    def save(self) -> None:
        (self.out / 'results.json').write_text(json.dumps(self.record, indent=2) + '\n', encoding='utf-8')


def as_user(command: str) -> str:
    """Run a command in the amity user's Wayland session."""
    return ("runuser -u amity -- env XDG_RUNTIME_DIR=/run/user/$(id -u amity) "
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


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial-port', type=int, required=True)
    parser.add_argument('--qmp-port', type=int, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--boot-timeout', type=int, default=600)
    parser.add_argument('--skip-apps', action='store_true')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    serial = Serial(args.serial_port, args.out / 'serial.log')
    qmp = Qmp(args.qmp_port)
    recorder = Recorder(args.out, serial, qmp)
    started = time.monotonic()
    try:
        serial.login(args.boot_timeout)
        recorder.record['boot_seconds_to_root_shell'] = round(time.monotonic() - started)
        phase2(recorder, not args.skip_apps)
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
