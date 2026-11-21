# 0002: Ubuntu LTS base, built reproducibly with mkosi

Status: accepted (default from the owner's brief); the exact release is pinned in Phase 2.

## Decision

- **Base:** the current Ubuntu LTS. Per our knowledge at planning time, that is
  **26.04 LTS** (April 2026). Phase 2 confirms that it is current, then pins the archive
  snapshot in `dependencies.lock.json`. If NVIDIA CUDA, ROCm, the Unity Editor or Unreal
  do not yet support 26.04 at that point, we use 24.04 LTS and record the blocker here.
- **Kernel:** the distro HWE kernel at first. A custom config or our own patches come
  later and are published under GPL-2.0.
- **Image build:** mkosi from WSL Ubuntu 24.04, with all caches and outputs under
  `F:\AmityOS` (`image/`, `.cache/`). Outputs are a bootable raw disk image for QEMU and
  an installable ISO. If mkosi cannot run in WSL2 (for example because of loop device
  or namespace limits), fall back to debootstrap + `systemd-repart` from a pinned script,
  and record the blocker here.

## Why Ubuntu

It is the best-supported target for CUDA, ROCm, the Unity Editor, Unreal and Android
Studio. NVIDIA and AMD publish Ubuntu packages first.

## Licences

| Component | Licence | Linked into proprietary code? |
|---|---|---|
| Ubuntu packages | mixed (mostly GPL/LGPL/MIT/BSD) | No: separate programs |
| Linux kernel | GPL-2.0 | No: separate program; our patches are published |
| mkosi | LGPL-2.1-or-later | No: build tool only |
