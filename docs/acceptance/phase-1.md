# Phase 1 acceptance: restructure

Date: 2026-11-20.

## What was done

- **Behaviour specs written from the earlier implementation before it was deleted:**
  [spaces](../specs/spaces.md), [apps-lifecycle](../specs/apps-lifecycle.md),
  [tasks](../specs/tasks.md), [shell-behaviour](../specs/shell-behaviour.md),
  [session-restore](../specs/session-restore.md). They were derived by reading code, not
  by running it, and each lists open questions. They contain no copied code.
- **Retired code deleted** from the tree and from disk: everything in the Retire table of
  the [migration plan](../migration/plan.md), all earlier documents and verification
  evidence, old build outputs, retired dependency downloads and session caches. No tag,
  branch, archive or backup of it was made. F: went from 130 GiB to 205 GiB free.
- **Owner's documents exported first**, read-only and hash-verified, to the git-ignored
  `F:\AmityOS\user-data-export\` (20 files, 4,214,305 bytes; manifest inside).
- **New workspace:** `crates/` (amity-ui, amity-comp, amity-shell, amity-spaces,
  amity-apps, amity-tasks, amityd), `apps/` (settings, files, terminal, editor),
  `integrations/vscode`, `image/`, `tooling/`. Only `amity-ui` has code: the colour,
  type and radius tokens from design screenshot 04. Every other crate is an empty library
  with a one-line statement of its role and phase.
- **Docs:** README, architecture, roadmap and licensing rewritten for the Linux base.
- **`AmityOS.ps1`** keeps the F: storage rule. `build` and `test` run Cargo in WSL with
  the pinned toolchain; `run` reports that the image arrives in Phase 2.

## Observed results

| Check | Result |
|---|---|
| `.\AmityOS.ps1 build` (cargo build --workspace --locked, WSL Ubuntu 24.04, Rust 1.91.1) | exit 0 |
| `.\AmityOS.ps1 test` (cargo test --workspace --locked) | exit 0; 4 tests ran and passed, all in `amity-ui` |
| WCAG AA contrast (4.5:1) of Text, Body and Muted on Ground, Surface and Raised | passed (one of the 4 tests) |
| `.\AmityOS.ps1` (run) | stops with the Phase 2 message, as intended |
| Scan of the new tree for references to the retired kernel | none outside `docs/decisions/0001`, `docs/migration/plan.md` and `docs/specs/` |

## History purge

Method chosen by the owner: `main` republished as a single new root commit, then
force-pushed; the `restructure/linux-base` branch deleted locally and on the remote;
local unreachable objects pruned. The observed verification results are in the
"Purge verification" section below, added by the commit that follows the purge.

## Not done or not verified

- Nothing boots. There is no image, compositor, shell or daemon yet.
- The specs' test vectors were worked out from the code by hand; none was executed.
- `docs/decisions/0001` and `docs/migration/plan.md` necessarily describe the retired
  code. They are the record the brief asked for; the owner can ask for them to be cut down.
- Clones, forks or downloads made before the purge, and GitHub's cached views of old
  commits, are outside this repository's control. GitHub Support can remove cached views
  and run garbage collection on request.
