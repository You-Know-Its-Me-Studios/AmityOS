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

The project is being rebuilt on the Linux base. The base image boots in a VM to a
placeholder desktop; the AmityOS compositor, shell and services are not written yet.

| Phase | State |
|---|---|
| 0 Plan | done: [migration plan](docs/migration/plan.md), [decisions](docs/decisions/) |
| 1 Restructure | done: new Cargo workspace builds and tests in WSL; [acceptance](docs/acceptance/phase-1.md) |
| 2 Bootable image | done in a VM: Ubuntu 26.04 base boots under QEMU to a graphical session; network, Chrome and VS Code work; [acceptance](docs/acceptance/phase-2.md). Not run on real hardware |
| 3 compositor and shell, 4 redesign, 5 Spaces/apps/tasks, 6 AI runtime, 7 compatibility matrix, 8 product layer | not started: [roadmap](docs/roadmap.md) |

## Layout

```
crates/amity-ui        shared design tokens and components
crates/amity-comp      Wayland compositor (Smithay)
crates/amity-shell     panel, launcher, switcher, notifications, assistant surface
crates/amity-spaces    Spaces daemon and client library
crates/amity-apps      app manager daemon
crates/amity-tasks     project tasks daemon
crates/amityd          Amity AI runtime daemon and provider trait
apps/                  settings, files, terminal, editor
integrations/vscode    VS Code extension
image/                 OS image configuration (mkosi), packages, branding, installer
tooling/               build, VM launch and test harness
docs/                  architecture, decisions, specs, migration, design, acceptance
```

## Working on it

The development host is Windows with WSL Ubuntu 24.04. All work, builds, caches, VM disks
and downloads stay under `F:\AmityOS`; there is no C: fallback.

```powershell
cd F:\AmityOS
.\AmityOS.ps1 build    # cargo build of the workspace, in WSL, pinned toolchain
.\AmityOS.ps1 test     # cargo test of the workspace
.\AmityOS.ps1 image    # build the OS image with mkosi (about 15 minutes the first time)
.\AmityOS.ps1          # boot the image in a VM (QEMU with WHPX); -ResetVm starts a fresh VM disk
.\AmityOS.ps1 vmtest   # boot a disposable VM without a window and run the image checks
```

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
