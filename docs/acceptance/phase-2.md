# Phase 2 acceptance: bootable image

Date: 2026-11-20 (run recorded 2026-11-21 02:14 UTC). Image:
`amityos-0.1.0-devel.qcow2`, SHA-256
`ee148294f2e3a44380febfb97ca2a6c2a801445aab7d5e3ffaf3efe2831c1af9`.

## What was built

- **Image:** Ubuntu 26.04.1 LTS (resolute) base, HWE kernel 7.0.0-44, systemd 259,
  476 packages ([manifest](phase-2/image-manifest.json)). Built by the pinned mkosi
  v27.1 (`.\AmityOS.ps1 image`), GPT disk with a 1 GB ESP and a btrfs root; systemd-boot
  with a unified kernel image. Branded as AmityOS in `os-release` and the login banner.
- **Build environment:** mkosi runs as root in WSL on a loop-mounted ext4 file stored on
  F: (`.cache\image-build\work.ext4`, 40 GB), because a Linux root filesystem cannot be
  assembled on the Windows drive mount. Build tools come from an mkosi tools tree, not
  from the WSL system. Nothing is installed into WSL.
- **Session:** `amity-session.service` starts the graphical session on tty1 as the
  development user `amity`. Until `amity-comp` exists it runs a placeholder Wayland
  compositor (labwc) with a terminal (foot). XWayland, PipeWire, xdg-desktop-portal,
  NetworkManager and Flatpak (with Flathub enabled on first network use) are installed.
- **VM launch:** `.\AmityOS.ps1` boots the image in the pinned Windows QEMU 8.2 with
  WHPX acceleration, UEFI, virtio disk/network/GPU/input, 4 CPUs and 8 GB by default.
  The VM disk is a qcow2 overlay in `image\out\vm\`, so the built image stays pristine.
- **Harness:** `.\AmityOS.ps1 vmtest` boots a fresh disposable VM without a window and
  drives it through a root serial console and QMP (`tooling/vm_harness.py`).

## Observed results (`.\AmityOS.ps1 vmtest`, WHPX, [results.json](phase-2/results.json))

| Check | Result |
|---|---|
| Boot to a root shell on the serial console | 12 seconds |
| `os-release` | `NAME="AmityOS"` |
| Kernel | 7.0.0-44-generic |
| `systemctl is-system-running` | `running` (no failed units) |
| Root filesystem | btrfs; grown on first boot to fill the 64 GiB virtual disk |
| Graphical session | `amity-session.service` active; Wayland socket present; guest screenshot shows the terminal ([01-session.png](phase-2/01-session.png)) |
| Networking | DHCP address 10.0.2.15 (QEMU user NAT); NetworkManager `connected`; HTTPS request to archive.ubuntu.com returned 200 |
| Flathub | system remote `flathub` configured by `amity-flathub.service` |
| Chrome | official `.deb` downloaded and installed in the guest; Google Chrome 158.0.8127.36; launched in the session, 17 processes; screenshot shows the window with the Google page loaded ([02-chrome.png](phase-2/02-chrome.png)) |
| VS Code | official `.deb` downloaded and installed; launched in the session (XWayland), 11 processes; screenshot shows the Welcome window ([03-vscode.png](phase-2/03-vscode.png)) |
| Normal launch path `.\AmityOS.ps1` | QEMU started with WHPX and a GTK window titled "QEMU (AmityOS)", and kept running. The guest screen in that window was not inspected; it uses the same image and arguments as `vmtest` apart from the display |

Method: commands on the guest's serial console and QMP screenshots of the guest display.
No host input automation. The three screenshots were opened and checked.

## Problems found and fixed on the way

- **mkosi DNS in WSL:** `/etc/resolv.conf` is a symlink mkosi's sandbox cannot follow.
  The build script runs mkosi in a private mount namespace with a regular `resolv.conf`.
- **WHPX:** OVMF on flash devices fails ("Failed to emulate MMIO access"), and `-cpu max`
  crashes the firmware. The launcher loads the firmware as a ROM and uses a
  `qemu64`-based CPU model. UEFI variables are therefore not persistent in the VM.
- **Ubuntu 26.04 no longer installs `login` by default:** without it the serial login and
  the session's PAM stack failed. Added to the package list.
- **Locale not generated**, and **systemd-networkd waiting for links NetworkManager owns**
  (boot ended `degraded`). Both fixed in the image post-install script.

## Not done or not verified

- **Real hardware: not run.** Steps are in [hardware-boot.md](../hardware-boot.md).
- **No installable ISO.** The brief's base-OS target lists one; it needs the installer,
  which is Phase 8. Today the disk image is written to a drive directly.
- **Builds are not yet reproducible bit for bit.** Package versions are recorded in the
  manifest, but the Ubuntu archive snapshot is not pinned (`Snapshot=` in mkosi), so a
  later rebuild picks up newer packages.
- **No GPU acceleration in the VM** (software rendering). NVIDIA, CUDA and ROCm are
  untested; they need real hardware.
- **Audio, Bluetooth, Wi-Fi, suspend and multi-monitor** were not exercised.
- **The development image is not safe to ship:** fixed `amity`/`amity` account and, in the
  `devel` profile, a root shell on the serial port. The `release` profile removes the
  serial shell; the account is replaced by first-run setup in Phase 8.
- **The release profile image was not built or booted** in this phase.
- The session is a placeholder compositor, not AmityOS's own (Phase 3).
