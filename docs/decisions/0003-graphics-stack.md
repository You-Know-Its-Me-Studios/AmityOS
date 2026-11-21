# 0003: Graphics, audio and portal stack

Status: accepted (default from the owner's brief).

## Decision

| Layer | Choice | Licence | Notes |
|---|---|---|---|
| AMD/Intel GPU | Mesa (radeonsi, RADV, iris, ANV) | MIT | from the Ubuntu archive |
| NVIDIA GPU | NVIDIA open kernel modules + NVIDIA userspace | GPL-2.0/MIT (modules), proprietary (userspace) | Installed at first run, not bundled, until redistribution terms are recorded in `docs/licensing.md` |
| Vulkan | Khronos loader + vendor ICDs | Apache-2.0 | |
| X11 apps | XWayland, started by `amity-comp` on demand | MIT | Unity, older tools |
| Audio, screen capture | PipeWire + WirePlumber | MIT | |
| Portals | xdg-desktop-portal + an Amity portal backend for screenshot/screencast/file chooser where the design needs it (screenshots 21, 26) | LGPL-2.1+ (portal, separate process) | Backend is our own Rust code |
| Compute | CUDA (NVIDIA), ROCm/HIP (AMD) | proprietary / mixed | First-run install; tested in Phase 7 where the hardware exists |

## Consequences

No GPL code is linked into owned components. The portal backend talks D-Bus to
xdg-desktop-portal. QEMU development uses virtio-gpu (no GPU compute). CUDA and ROCm
acceptance needs real hardware.
