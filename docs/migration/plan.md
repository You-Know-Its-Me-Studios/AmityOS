# Migration plan: AmityOS on a Linux base

Status: Phase 1 carried out ([acceptance](../acceptance/phase-1.md)). Decision:
[0001-linux-base](../decisions/0001-linux-base.md). Section 1 describes the repository as it
stood on 2026-11-20, before the restructure.

## 1. What exists today (verified)

Line counts are tracked files only (`git ls-files`), excluding `artifacts/`.

| Area | Language | Lines | Notes |
|---|---|---|---|
| `kernel/` | Rust, `no_std`, `x86_64-unknown-none` | 11,212 | boot, memory, scheduler, capabilities, drivers, diagnostic FAT adapter |
| `abi/` | Rust | 1,016 | syscall, IPC and service records |
| `filesystem/` | Rust | 4,304 | portable FAT/VFAT, journal, package format, protocol, open files, network policy |
| `userspace/src` (library) | Rust | 21,926 | runtime, services, graphics/UI/theme stack, Spaces policy (`managed_storage.rs`) |
| `userspace/src/bin` | Rust | 15,354 | compositor `desktop.rs`, Files, Editor, Terminal, Settings, Browser, services, 17 `*_probe` binaries |
| `wifi/` | Rust + C | 1,846 + 448 | MT7925 driver and hostap glue |
| `compatibility/` | C | 3,284 | Wine/musl/FreeType **adapters only**; the pinned upstream sources are untracked downloads in `.deps/` |
| `browser/` | C | 1,236 | NetSurf integration glue; upstream sources in `.deps/` |
| `tooling/` | Python + Rust + PowerShell | 20,041 + 1,097 + 389 | QEMU harnesses, release packaging, Wine overlay builder |
| `docs/` | Markdown | 11,569 | 78 files; nearly all are records of the original kernel |
| `boot/` | Limine config, protocol header, licence | 2,077 | |

Total owned Rust is about 56.7k lines, matching the brief's ~55k. Corrections to the brief:

- The window protocol is **v4** (multi-window clients, undecorated windows), not v3.
- The C "ports" in `compatibility/` and `browser/` are adapters and build overlays; the
  large upstream trees (Wine 11.0, musl 1.2.6, FreeType, NetSurf) are pinned archives in
  untracked `.deps/` (6.1 GiB). The Wine port is mostly patch logic in
  `tooling/build_wine_amity.py` and `tooling/compatibility_sources.py`.
- Not listed in the brief: SQLite + Mbed TLS for the browser, `userspace/src/apps/`
  (document, file picker, settings pages, notes sample), `filesystem/src/network_policy.rs`
  (per-Space network policy), `desktop_clipboard.rs` (Space-partitioned clipboard),
  sample packages (`notes_v1/v2`, `project_tool`), 129 icons in `assets/icons`.
- `artifacts/` is 69.7 GiB on disk but only 1,326 tracked files (verification records,
  research, manifests). It also holds **untracked user data**: the owner's persistent VM
  disk `artifacts/amity-user.img` and 21.4 GiB of verified rollback images. See section 4.
- `legacy/windows/` (703 MB, untracked) is relocated Windows app-data from the F: move,
  not code. Out of scope; left untouched.
- The working tree has **uncommitted** Windows-compatibility work (21 modified source files,
  3 untracked). It all falls under Retire. See section 4.
- Dev host and storage rule confirmed: `AmityOS.ps1` refuses anything outside
  `F:\AmityOS`, keeps a 30 GiB free reserve and an 84 GiB project budget
  (defaults in `tooling/storage_paths.py`). Before the restructure the project used about
  80 GiB of that budget.

## 2. Keep, port, retire

### Retire (deleted permanently; Linux or an existing component replaces it)

| Current code | Replaced by |
|---|---|
| `kernel/`, `abi/`, `boot/` (Limine) | Linux kernel (distro HWE kernel), GRUB/systemd-boot from the base image |
| `userspace` runtime: `allocator*.rs`, `native_alloc.rs`, `vm.rs`, `thread.rs`, `exception.rs`, `posix.rs`, `process.rs`, `cap.rs`, `local_service.rs`, `fileservice.rs`, `fs.rs` | Rust `std` on Linux |
| `filesystem/` (FAT engine, journal, read cache, protocol, open files), `storage_service.rs` journal/recovery | ext4/btrfs. Spaces *policy* is ported (below); `package.rs` (`.amity` format) is retired, its lifecycle states are ported |
| `network_service.rs`, `network*.rs`, `desktop_network.rs` internals, smoltcp, `tls.rs`, `tls_probe` | Linux networking, NetworkManager, rustls |
| `wifi/`, `wifi_mt7925.rs`, `wifi_service.rs`, `wifi_store.rs` | Linux `mt7925` driver + NetworkManager. The Settings Wi-Fi page concept is redesigned from screenshots 24 and 31 |
| `browser/`, `browser_runtime.rs`, `bin/browser.rs`, SQLite/Mbed TLS glue | Chromium/Chrome and Firefox |
| `compatibility/`, `wine*.rs`, `windows_shortcut.rs`, Wine tooling | Distro Wine/Proton as an optional package |
| Compositor and drawing: `gui.rs` protocol, `graphics.rs`, `ui.rs`, `theme.rs`, `font.rs`, `text.rs`, `icons.rs`, `switcher.rs` drawing, `bin/desktop.rs` rendering | `amity-comp` (Smithay) + `amity-ui` (iced) |
| All 17 `*_probe` binaries, `hello`, sample packages (`notes_*`, `project_tool`) | New test harness for the Linux image |
| `tooling/` QEMU harnesses, release/Wine/Wi-Fi/browser builders | `tooling/` for mkosi build, VM launch and image tests |
| `artifacts/verification`, `artifacts/research`, tracked manifests | Deleted; they document the old kernel |
| Old-kernel docs (76 of 78 files in `docs/`) | Rewritten docs; concept specs carried forward as new documents (section 3) |

### Port (product idea kept, rewritten on `std` + Linux)

| Concept | Current location | Port target |
|---|---|---|
| Spaces: private Home/Project/State, hard quotas, shares, retained storage | `managed_storage.rs` (quota arithmetic, share grants), `storage_service.rs` policy, `apps/space_environment.rs`, Settings → Desktops | `amity-spaces`: btrfs subvolume per Space + qgroups for hard quotas, bind mounts + Landlock for isolation, a systemd user slice per Space for resources |
| Per-Space network policy | `filesystem/src/network_policy.rs` | `amity-spaces` (per-slice nftables/cgroup socket policy) |
| Space-partitioned clipboard | `desktop_clipboard.rs` | `amity-comp` (per-Space Wayland data-device state) |
| App profiles, environment, grants | `appmanager_service.rs`, `applications.rs`, `config.rs`, `session_policy.rs` | `amity-apps`: Flatpak per-Space overrides and portals, plus native packages |
| App lifecycle: installed / enabled in a Space / autostart / running | `appmanager*.rs`, `filesystem/src/package.rs` (states only) | `amity-apps`, clean state model, D-Bus API |
| Project tasks: definitions, environment, output, cancel | `task_service.rs`, `project.rs`, `tasks.rs`, `bin/tasks.rs` | `amity-tasks`: transient systemd user units in the Space's slice |
| Session restore, window placement per Space | `desktop_shell.rs`, `window_geometry.rs`, session records in `config.rs` | `amity-comp` + `amity-shell`, stored per Space |
| Taskbar grouping and pinning, launcher, MRU switcher, Desktop selector | `bin/desktop.rs`, `desktop_shell.rs`, `switcher.rs` | `amity-shell` (behaviour ported, visuals from screenshots 01, 02, 10) |
| Settings, Files | `bin/settings.rs`, `apps/*_settings.rs`, `bin/files.rs`, `apps/file_picker.rs` | Native Amity apps on `amity-ui` (screenshots 07, 08, 21, 28–38) |
| Terminal | `bin/terminal.rs` | Native app on `alacritty_terminal` (screenshot 17); no new VT parser |
| Text editor | `bin/editor.rs`, `apps/document.rs` | Light native editor (screenshot 19); VS Code is the primary code editor |
| Design tokens | `theme.rs` | Replaced by tokens from screenshot 04 and the rest of the set |
| Icons, wallpaper | `assets/` | Keep the Amity emblem and wallpaper (both appear in screenshots 01 and 04); other icons only where they match |
| "Explain this" diagnostics | `docs/roadmap.md` step 10 | `amityd` feature over journald, systemd unit state and process state |

Logic is reused only where it truly ports (state machines, validation, quota arithmetic).
No `no_std` module is carried into the new tree.

### Keep as-is

Cargo workspace and pinned toolchain (`rust-toolchain.toml`, target becomes the host
Linux target), `dependencies.lock.json` pinning (contents replaced), the evidence-based
acceptance style, the `F:\AmityOS` storage rule (`tooling/storage_paths.py` and the
`AmityOS.ps1` guards), and a single `AmityOS.ps1` that builds and boots the image in a
VM from Windows.

## 3. How ported concepts survive the deletion

The brief deletes old code permanently in Phase 1 and purges it from history, but Phases 3
and 5 port behaviour from that code. To keep both rules, Phase 1 writes
**behavioural specs** before deleting anything: `docs/specs/spaces.md`,
`apps-lifecycle.md`, `tasks.md`, `shell-behaviour.md` (taskbar grouping, pinning, MRU
switcher, launcher, Desktop selector) and `session-restore.md`. Each spec states rules,
limits and edge cases taken from the code (quota arithmetic, share semantics, lifecycle
transitions, MRU ordering, placement clamping), with test vectors. Specs are new
documents, not copies of code; the ports are written fresh against them.

## 4. Conflicts and the owner's decisions (2026-11-20)

| Conflict | Decision |
|---|---|
| Uncommitted Windows-compatibility work (21 modified and 3 new source files) would be destroyed by the deletion, and the brief forbids keeping a copy | **Discard.** It was entirely in the Retire table |
| `artifacts/` also held user data that was not in git: the owner's persistent VM disk and 21.4 GiB of rollback images | **Export, then delete.** `/HOME`, `/SPACES` and `/CONFIG` were exported read-only and hash-verified to the git-ignored `F:\AmityOS\user-data-export\` (20 files, 4.2 MB); then all images were deleted |
| History purge: `git filter-repo` needs every retired path including historical names, and a missed path leaves old code in history | **New root commit.** `main` is republished as a single commit containing only the new tree, and force-pushed. History of surviving files is not kept |
| The pinned Windows QEMU runs without acceleration (TCG), so a full Ubuntu desktop would be very slow | **The owner enables Windows Hypervisor Platform.** Phase 2 launches with `-accel whpx` and falls back to TCG with a warning |
| Phase 3 (DRM/KMS) and Phase 7 (CUDA, ROCm, Unity, Unreal, Android emulator) need real hardware; QEMU on this host has no GPU passthrough | Recorded as "not run" until a test machine is available |

Not touched: `legacy/windows/` (relocated Windows app-data, not code) and `.relocation/`
(reports from the move to F:). Both are git-ignored.

## 5. Phase plan

| Phase | Output | Acceptance |
|---|---|---|
| 0 Plan | this file, decision records 0001–0007 | done |
| 1 Restructure | branch `restructure/linux-base`, specs (section 3), retired code deleted, new workspace (`crates/`, `apps/`, `integrations/`, `image/`, `tooling/`), updated README/architecture/roadmap, `docs/licensing.md`; merge; history purge; force-push | `cargo build` and `cargo test` pass in WSL; `git log --all -- kernel abi boot` empty; fresh clone has no retired code |
| 2 Image | `image/` mkosi config, pinned Ubuntu LTS + HWE kernel, disk image + ISO, `AmityOS.ps1` boot | boots in QEMU to a graphical session; network works; Chrome and VS Code install and launch; real-hardware boot steps documented |
| 3 Compositor + shell MVP | `amity-comp` (nested + DRM/KMS), XWayland, panel, launcher, MRU switcher, multi-monitor | VS Code, Chrome, Blender run on it |
| 4 Redesign | `docs/design/{inventory,system,gaps}.md`, `amity-ui` tokens/components, all 38 screenshots implemented | side-by-side comparisons in `docs/design/verification/` |
| 5 Spaces/apps/tasks | `amity-spaces`, `amity-apps`, `amity-tasks` | create Space, quota, share, launch in Space, switch without killing apps, restore after reboot |
| 6 AI runtime | `amityd`, provider trait, local model, assistant surface, "Explain this", VS Code extension | listed behaviours observed; the placeholder model is labelled as one |
| 7 Compatibility | `docs/acceptance/compatibility.md` | each app run or marked "not run" |
| 8 Product | installer, signed A/B or OSTree updates, first-run setup, AI privacy, crash opt-in | per feature |
