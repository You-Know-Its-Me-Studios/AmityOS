#!/usr/bin/env python3
"""Developer entry point used by AmityOS.ps1 inside WSL.

Builds and tests the Cargo workspace with the pinned toolchain from .deps/, with
every cache, temporary file and output on F:\\AmityOS (tooling/storage_paths.py).
"""
import subprocess
import sys

import storage_paths

COMMANDS = {
    'lock': ['generate-lockfile', '--offline'],
    'build': ['build', '--workspace', '--locked'],
    'test': ['test', '--workspace', '--locked'],
}


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in COMMANDS:
        raise SystemExit('usage: amity_dev.py lock|build|test')
    storage_paths.configure(2 * storage_paths.GIB)
    cargo = storage_paths.rust_tool('cargo')
    result = subprocess.run([str(cargo), *COMMANDS[sys.argv[1]]], cwd=storage_paths.ROOT)
    raise SystemExit(result.returncode)


if __name__ == '__main__':
    main()
