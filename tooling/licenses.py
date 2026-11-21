#!/usr/bin/env python3
"""Licence ledger and gate for the Rust dependencies.

Reads `cargo metadata` output and, for every third-party crate, records its
licence, whether the owner's licensing rules allow it, and which AmityOS
programs it is linked into. Writes docs/licensing/crates.md and fails when a
crate that is linked into an AmityOS program is not allowed.

    AmityOS.ps1 licenses            (runs cargo metadata in the build root, then this)
    python tooling/licenses.py METADATA.json [--write FILE] [--check]

Standard library only. The rules are in docs/licensing.md.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path

# Licences the owner's brief names as fine to link ("MIT, Apache-2.0, BSD, Zlib and MPL-2.0").
NAMED = {'MIT', 'Apache-2.0', 'BSD-2-Clause', 'BSD-3-Clause', 'Zlib', 'MPL-2.0'}
# Permissive licences the brief does not name. They carry no copyleft obligation; they are
# listed separately so the owner can see exactly what was accepted beyond the named set.
OTHER_PERMISSIVE = {
    'ISC', '0BSD', 'BSL-1.0', 'CC0-1.0', 'Unlicense', 'MIT-0', 'Unicode-3.0', 'Unicode-DFS-2016',
    'OFL-1.1', 'Ubuntu-font-1.0', 'NCSA', 'BSD-1-Clause', 'CDLA-Permissive-2.0',
    'Apache-2.0 WITH LLVM-exception',
}
ALLOWED, ACCEPTED, REFUSED = 0, 1, 2
CLASS_TEXT = {ALLOWED: 'named in the brief', ACCEPTED: 'permissive, not named', REFUSED: 'NOT ALLOWED'}

# Crates whose manifest has no usable `license` field: the licence was read from the
# crate's own licence file. Format: name -> (SPDX expression, where it was read).
MANUAL: dict[str, tuple[str, str]] = {}


def tokens(expression: str) -> list[str]:
    return re.findall(r'\(|\)|[^\s()]+', expression.replace('/', ' OR '))


def classify(expression: str | None) -> tuple[int, str]:
    """Return (class, the alternative chosen) for an SPDX licence expression.

    `A OR B` is as good as its best alternative; `A AND B` as bad as its worst part.
    """
    if not expression:
        return REFUSED, 'no licence stated'
    stream = tokens(expression)
    position = 0

    def peek() -> str | None:
        return stream[position] if position < len(stream) else None

    def take() -> str:
        nonlocal position
        position += 1
        return stream[position - 1]

    def factor() -> tuple[int, str]:
        if peek() == '(':
            take()
            result = alternatives()
            if peek() != ')':
                raise IndexError('unbalanced parenthesis')
            take()
            return result
        name = take()
        if (peek() or '').upper() == 'WITH':
            take()
            name = f'{name} WITH {take()}'
        if name in NAMED:
            return ALLOWED, name
        if name in OTHER_PERMISSIVE:
            return ACCEPTED, name
        return REFUSED, name

    def conjunction() -> tuple[int, str]:
        rank, chosen = factor()
        while (peek() or '').upper() == 'AND':
            take()
            other_rank, other = factor()
            rank, chosen = max(rank, other_rank), f'{chosen} AND {other}'
        return rank, chosen

    def alternatives() -> tuple[int, str]:
        best = conjunction()
        while (peek() or '').upper() == 'OR':
            take()
            other = conjunction()
            if other[0] < best[0]:
                best = other
        return best

    try:
        result = alternatives()
    except IndexError:
        return REFUSED, f'unparsable: {expression}'
    return result if position == len(stream) else (REFUSED, f'unparsable: {expression}')


def build_ledger(metadata: dict) -> tuple[list[dict], list[str]]:
    packages = {package['id']: package for package in metadata['packages']}
    nodes = {node['id']: node for node in metadata['resolve']['nodes']}
    members = set(metadata['workspace_members'])

    def is_proc_macro(package_id: str) -> bool:
        return any('proc-macro' in target['kind'] for target in packages[package_id]['targets'])

    def enabled_optional(package_id: str) -> set[str]:
        """Names of the optional dependencies that the enabled features switch on.

        `cargo metadata` lists an optional dependency in the graph even when it is only
        mentioned by a weak feature (`dep?/feature`) and therefore not compiled, so the
        enabled features decide.
        """
        package = packages[package_id]
        optional = {(d.get('rename') or d['name']) for d in package['dependencies'] if d['optional']}
        enabled: set[str] = set()
        for feature in nodes[package_id]['features']:
            if feature in optional:
                enabled.add(feature)  # the implicit feature named after the dependency
            for item in package['features'].get(feature, []):
                if item.startswith('dep:'):
                    enabled.add(item[4:])
                elif '/' in item and not item.split('/', 1)[0].endswith('?'):
                    enabled.add(item.split('/', 1)[0])
        return enabled

    def followed_kinds(package_id: str, dependency: dict) -> set:
        """The kinds (None for normal, 'build') under which a dependency is really used."""
        package = packages[package_id]
        enabled = enabled_optional(package_id)
        kinds = set()
        for kind in {entry['kind'] for entry in dependency['dep_kinds']} - {'dev'}:
            declarations = [d for d in package['dependencies']
                            if (d.get('rename') or d['name']).replace('-', '_') == dependency['name']
                            and d['kind'] == kind]
            if not declarations or any(not d['optional'] or (d.get('rename') or d['name']) in enabled
                                       for d in declarations):
                kinds.add(kind)
        return kinds

    def closure(root: str) -> tuple[set[str], set[str]]:
        """(linked, build-time only) third-party packages reachable from a workspace member."""
        linked: set[str] = set()
        build_only: set[str] = set()

        def walk(package_id: str, at_build_time: bool) -> None:
            target = build_only if at_build_time else linked
            if package_id in target or (at_build_time and package_id in linked):
                return
            target.add(package_id)
            for dependency in nodes[package_id]['deps']:
                kinds = followed_kinds(package_id, dependency)
                child = dependency['pkg']
                if None in kinds:
                    # A procedural macro runs in the compiler; neither it nor its
                    # dependencies end up in the program.
                    walk(child, at_build_time or is_proc_macro(child))
                if 'build' in kinds:
                    walk(child, True)

        walk(root, False)
        return linked - {root}, build_only - linked

    programs = sorted(
        (packages[member]['name'], member) for member in members
        if any('bin' in target['kind'] for target in packages[member]['targets'])
    )
    linked_into: dict[str, set[str]] = {}
    build_time: set[str] = set()
    for name, member in programs:
        linked, build_only = closure(member)
        for package_id in linked:
            linked_into.setdefault(package_id, set()).add(name)
        build_time |= build_only
    # Members without a program yet (libraries only) still count: their dependencies will be linked.
    for member in members - {member for _, member in programs}:
        linked, build_only = closure(member)
        for package_id in linked:
            linked_into.setdefault(package_id, set())
        build_time |= build_only

    rows = []
    for package_id, package in packages.items():
        if package_id in members:
            continue
        if package_id not in linked_into and package_id not in build_time:
            continue  # only reachable through dev-dependencies of third-party crates
        licence = package.get('license')
        note = ''
        if not licence and package['name'] in MANUAL:
            licence, source = MANUAL[package['name']]
            note = f'licence read from {source}'
        if package.get('source') is None:
            note = (note + '; ' if note else '') + 'vendored in this repository with a patch'
        rank, chosen = classify(licence)
        is_linked = package_id in linked_into
        rows.append({
            'name': package['name'], 'version': package['version'], 'licence': licence or 'none stated',
            'chosen': chosen, 'rank': rank, 'linked': is_linked,
            'programs': sorted(linked_into.get(package_id, ())), 'note': note,
        })
    rows.sort(key=lambda row: (row['name'].lower(), row['version']))
    return rows, [name for name, _ in programs]


def render(rows: list[dict], programs: list[str]) -> str:
    linked = [row for row in rows if row['linked']]
    by_class = Counter(row['rank'] for row in linked)
    by_licence = Counter(row['chosen'] for row in linked)
    lines = [
        '# Rust dependencies and their licences',
        '',
        'Generated by `tooling/licenses.py` from `cargo metadata` (Linux x86-64 target, `Cargo.lock`',
        'versions). Do not edit by hand: run `.\\AmityOS.ps1 licenses`. The rules are in',
        '[licensing.md](../licensing.md).',
        '',
        f'- Programs built from this workspace: {", ".join(f"`{name}`" for name in programs) or "none yet"}.',
        f'- Third-party crates linked into them: **{len(linked)}**. '
        f'Used only at build time (build scripts, procedural macros): {len(rows) - len(linked)}.',
        f'- Linked crates under a licence named in the brief: {by_class[ALLOWED]}; '
        f'under another permissive licence: {by_class[ACCEPTED]}; not allowed: **{by_class[REFUSED]}**.',
        '',
        'For a crate offered under several licences, "Used under" is the one AmityOS takes.',
        '',
        '## Licences of the linked crates',
        '',
        '| Used under | Crates | Status |',
        '|---|---|---|',
    ]
    for licence, count in sorted(by_licence.items(), key=lambda item: (-item[1], item[0])):
        rank = classify(licence)[0]
        lines.append(f'| {licence} | {count} | {CLASS_TEXT[rank]} |')
    lines += ['', '## Linked crates', '', '| Crate | Version | Licence | Used under | Status | Linked into | Note |',
              '|---|---|---|---|---|---|---|']
    for row in linked:
        where = ', '.join(row['programs']) or 'libraries only so far'
        lines.append(f"| {row['name']} | {row['version']} | {row['licence']} | {row['chosen']} | "
                     f"{CLASS_TEXT[row['rank']]} | {where} | {row['note']} |")
    lines += ['', '## Build-time only', '',
              'These run while compiling (build scripts and procedural macros) and are not part of any',
              'shipped program, so the linking rules do not apply to them.', '',
              '| Crate | Version | Licence |', '|---|---|---|']
    for row in rows:
        if not row['linked']:
            lines.append(f"| {row['name']} | {row['version']} | {row['licence']} |")
    return '\n'.join(lines) + '\n'


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('metadata', type=Path, help='output of: cargo metadata --format-version 1 --locked '
                                                    '--filter-platform x86_64-unknown-linux-gnu')
    parser.add_argument('--write', type=Path, help='write the ledger to this Markdown file')
    parser.add_argument('--check', action='store_true', help='fail when a linked crate is not allowed')
    args = parser.parse_args()

    # cargo may print progress lines before the JSON document.
    text = args.metadata.read_text(encoding='utf-8-sig')
    metadata = json.loads(text[text.index('{"packages"'):] if '{"packages"' in text else text)
    rows, programs = build_ledger(metadata)
    if args.write:
        args.write.parent.mkdir(parents=True, exist_ok=True)
        args.write.write_text(render(rows, programs), encoding='utf-8', newline='\n')
    linked = [row for row in rows if row['linked']]
    refused = [row for row in linked if row['rank'] == REFUSED]
    accepted = sorted({row['chosen'] for row in linked if row['rank'] == ACCEPTED})
    print(f'{len(linked)} linked crates, {len(rows) - len(linked)} build-time only; '
          f'permissive licences beyond the brief: {", ".join(accepted) or "none"}')
    for row in refused:
        print(f"NOT ALLOWED: {row['name']} {row['version']}: {row['licence']} (linked into {', '.join(row['programs'])})")
    return 1 if args.check and refused else 0


if __name__ == '__main__':
    sys.exit(main())
