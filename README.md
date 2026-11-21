# AmityOS

AmityOS is a commercial operating system for developers and designers. It aims to be the
best place to do technical creative work: software, web, mobile and game development, and
UI/UX and visual design. The Amity AI assistant is built into the whole system.

AmityOS is built on the **Linux kernel** and a mainstream Linux base (Ubuntu LTS), which
supply hardware drivers, GPU support (NVIDIA, AMD, Intel, Vulkan, CUDA, ROCm) and
application compatibility. Everything AmityOS owns is written in **Rust**: the Wayland
compositor and desktop shell, system services, Spaces, the app manager, Settings and the
other Amity apps, and the Amity AI runtime. See
[decision 0001](docs/decisions/0001-linux-base.md).

## Status

The project is being rebuilt on the Linux base. The image boots in a VM to the AmityOS
desktop: its own Wayland compositor and a first shell (taskbar, launcher, window
switcher) on which Chrome, VS Code and Blender run. The look is not the design yet
(Phase 4), and Spaces, the app manager, tasks and the Amity AI runtime are not written.

| Phase | State |
|---|---|
| 0 Plan | done: [migration plan](docs/migration/plan.md), [decisions](docs/decisions/) |
| 1 Restructure | done: new Cargo workspace builds and tests in WSL; [acceptance](docs/acceptance/phase-1.md) |
| 2 Bootable image | done in a VM: Ubuntu 26.04 base boots under QEMU to a graphical session; network, Chrome and VS Code work; [acceptance](docs/acceptance/phase-2.md). Not run on real hardware |
| 3 Compositor and shell MVP | done in a VM and nested under WSLg: window management, XWayland, taskbar, launcher, Alt+Tab switcher, two monitors; Chrome, VS Code and Blender run; [acceptance](docs/acceptance/phase-3.md). Not run on a real GPU or real monitors |
| 4 redesign, 5 Spaces/apps/tasks, 6 AI runtime, 7 compatibility matrix, 8 product layer | not started: [roadmap](docs/roadmap.md) |

## Layout

```
crates/amity-ui        shared design tokens and components
crates/amity-comp      Wayland compositor (Smithay): windows, outputs, input, XWayland
crates/amity-shell     taskbar, launcher, window switcher; later notifications and the assistant
crates/amity-ipc       message types of the private compositor-shell channel
crates/amity-spaces    Spaces daemon and client library
crates/amity-apps      application catalogue and launching; later the app manager daemon
crates/amity-tasks     project tasks daemon
crates/amityd          Amity AI runtime daemon and provider trait
apps/                  settings, files, terminal, editor
integrations/vscode    VS Code extension
image/                 OS image configuration (mkosi), packages, branding, installer
tooling/               build root, image build, VM launch, test harness, licence gate
vendor/                third-party code carried with a patch (each with an AMITYOS-PATCH.md)
docs/                  architecture, decisions, specs, migration, design, acceptance
```

## Working on it

The development host is Windows with WSL Ubuntu 24.04. All work, builds, caches, VM disks
and downloads stay under `F:\AmityOS`; there is no C: fallback.

```powershell
cd F:\AmityOS
.\AmityOS.ps1 build      # cargo build of the workspace, pinned toolchain, in the build root
.\AmityOS.ps1 test       # cargo test of the workspace
.\AmityOS.ps1 nested     # run compositor + shell as a window on this desktop (WSLg) and check them
.\AmityOS.ps1 image      # build the OS image: compiles the AmityOS programs, then mkosi
.\AmityOS.ps1            # boot the image in a VM (QEMU with WHPX); -ResetVm starts a fresh VM disk
.\AmityOS.ps1 vmtest -Suite phase3         # disposable VM, no window: compositor, shell, apps
.\AmityOS.ps1 vmtest -Suite phase3-multi   # the same VM with a second monitor plugged in and out
.\AmityOS.ps1 licenses   # regenerate docs\licensing\crates.md; fails on a forbidden licence
```

Rust code is compiled inside a **build root**: an Ubuntu 26.04 tree with the development
libraries the compositor needs, created by `tooling/dev.sh` on first use and kept, like
every other build output, in a Linux filesystem image on F:. Nothing is installed into
WSL itself. `vmtest -DevBuild -KeepVm` tries freshly built programs in the existing test
VM without rebuilding the image.

The VM needs the Windows Hypervisor Platform feature; `-Accel tcg` runs without it, slowly.
The development image logs in as `amity` (password `amity`). Booting a real machine:
[docs/hardware-boot.md](docs/hardware-boot.md).

## Rules

- Never claim something works without having run it and seen it work. Acceptance notes in
  [docs/acceptance](docs/acceptance/) say what was tested and what was not.
- Dependencies are pinned (`Cargo.lock`, `dependencies.lock.json`) and their licences are
  recorded in [docs/licensing.md](docs/licensing.md). GPL code is never linked into owned
  components.
- Product behaviour carried over from the earlier prototype is specified in
  [docs/specs](docs/specs/).
