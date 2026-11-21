# 0001: Build AmityOS on the Linux kernel

Status: accepted (owner decision, 2026-11-20). Supersedes the former rule that no
Linux/Windows kernel may supply the guest foundation and no existing OS kernel or desktop
is reused.

## Decision

AmityOS runs on the Linux kernel from a mainstream distribution base
([0002](0002-base-os-and-image.md)). Everything AmityOS owns is written in Rust: the
compositor and shell, system services, Spaces, the app manager, Settings and the other
Amity apps, the Amity AI runtime, and any kernel modules written later. The original
from-scratch kernel and everything that depended on it are deleted
([migration plan](../migration/plan.md)).

## Reasons

- **Customers' tools must run natively:** VS Code, Chrome/Chromium, Firefox, Unity Hub and
  Editor, Unreal Editor, Blender, Godot, Android Studio, Docker, Git and normal toolchains.
  They target Linux (or Windows/macOS), never a new kernel ABI. A from-scratch kernel
  could run none of them. The Wine work on the old kernel took months to run two
  utilities.
- **GPU compute and graphics:** CUDA and ROCm ship only for Linux (and Windows) with
  vendor kernel drivers. Vulkan, Mesa and the NVIDIA open kernel modules all assume Linux.
- **Hardware:** Linux supplies drivers for the machines customers own. The old kernel ran
  only in QEMU with one Wi-Fi chipset.
- **Where the value is:** the product is the Amity experience: Spaces, the shell, the AI
  layer, and design and developer workflows. It is not a scheduler or a filesystem.

## Consequences

- Rust remains the language for owned code. The Linux kernel accepts Rust for new drivers
  and modules; we confirm the exact kernel version and toolchain requirements before we
  write our first module (none is planned before Phase 8).
- Changes to the Linux kernel are GPL-2.0 and must be published when distributed.
  Proprietary components (shell, `amityd`, Spaces, apps) never link GPL code
  ([licensing](../licensing.md)).
- The old kernel, its acceptance records and its evidence are removed from the repository
  and its history. Clones made before the purge and any GitHub caches of old commits are
  outside the repository's control; GitHub Support can remove cached views on request.
- Product concepts carry forward as specifications: Spaces, app lifecycle, project tasks,
  taskbar/launcher/switcher behaviour, and "Explain this".
