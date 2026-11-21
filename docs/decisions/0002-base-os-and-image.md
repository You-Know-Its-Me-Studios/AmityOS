# 0002: Ubuntu LTS base, built reproducibly with mkosi

Status: accepted (default from the owner's brief); confirmed and implemented in Phase 2.

## Decision

- **Base:** Ubuntu **26.04 LTS** (resolute). Confirmed as the current LTS (26.04.1) from
  Ubuntu's release metadata on 2026-11-20. It ships kernel 7.0. Whether NVIDIA CUDA,
  ROCm, the Unity Editor and Unreal support 26.04 is checked in Phase 7; if one does not,
  the blocker is recorded here and 24.04 LTS is the fallback. The archive snapshot is
  not pinned yet (see the Phase 2 acceptance note).
- **Kernel:** the distro HWE kernel at first. A custom config or our own patches come
  later and are published under GPL-2.0.
- **Image build:** mkosi v27.1 (pinned in `dependencies.lock.json`) from WSL Ubuntu
  24.04, with all caches and outputs under `F:\AmityOS`. mkosi runs in WSL2 without
  changes to the WSL system: the build happens on a loop-mounted ext4 file on F:, and
  the build tools come from an mkosi tools tree. The output is a GPT disk image
  (ESP + btrfs root, systemd-boot, unified kernel image), stored as compressed qcow2.
  The installable ISO comes with the installer in Phase 8.

## Why Ubuntu

It is the best-supported target for CUDA, ROCm, the Unity Editor, Unreal and Android
Studio. NVIDIA and AMD publish Ubuntu packages first.

## Licences

| Component | Licence | Linked into proprietary code? |
|---|---|---|
| Ubuntu packages | mixed (mostly GPL/LGPL/MIT/BSD) | No: separate programs |
| Linux kernel | GPL-2.0 | No: separate program; our patches are published |
| mkosi | LGPL-2.1-or-later | No: build tool only |
