# Booting AmityOS on real hardware

**Status: not run.** These steps have not been tried on a physical machine. The image has
only been booted in QEMU ([phase-2 acceptance](acceptance/phase-2.md)). Expect problems
and report what you see.

## What you need

- An x86-64 PC with UEFI firmware. Legacy BIOS boot is not supported.
- **Secure Boot turned off.** The boot loader (systemd-boot) and the kernel image are not
  signed yet; signing arrives with the installer and updates in Phase 8.
- A USB stick or spare SSD of at least 8 GB. **Everything on it is erased.**
- Do not use a disk that holds data you need. There is no installer yet: the image is
  written over the whole disk.

## 1. Build an image without the development console

The default development image opens a root shell on the serial port. Build the release
profile, which does not:

```powershell
cd F:\AmityOS
.\AmityOS.ps1 image -ImageProfile release
```

This writes `image\out\amityos-<version>.qcow2`.

## 2. Convert it to a raw disk image

```powershell
F:\AmityOS\.deps\qemu-windows-8.2.0\runtime\qemu-img.exe convert -O raw `
    F:\AmityOS\image\out\amityos-0.1.0.qcow2 F:\AmityOS\image\out\amityos-0.1.0.raw
```

The raw file is about 4 GB.

## 3. Write it to the USB stick or SSD

- **Windows:** use a tool that writes raw disk images, such as Rufus in "DD image" mode
  or balenaEtcher. Choose `amityos-0.1.0.raw` and the target drive. Check the drive
  letter and size twice.
- **Linux:** `sudo dd if=amityos-0.1.0.raw of=/dev/sdX bs=4M conv=fsync status=progress`,
  where `/dev/sdX` is the target disk (check with `lsblk`).

## 4. Boot

1. Plug the disk in, open the firmware boot menu (often F12, F8 or Esc) and pick the
   disk's UEFI entry.
2. On the first boot the root partition grows to fill the disk. This takes a few seconds.
3. The session starts automatically as the development user `amity` (password `amity`).
   Change the password with `passwd`. The first-run setup that replaces this account is
   Phase 8.

## What to expect today

| Area | State |
|---|---|
| Kernel and firmware | Ubuntu 26.04 HWE kernel (7.0) with `linux-firmware`, so common storage, network and Wi-Fi chips should load drivers |
| Graphics | Mesa drivers for AMD and Intel. On NVIDIA cards the open-source `nouveau` driver is used; the NVIDIA driver is not installed (see [decision 0003](decisions/0003-graphics-stack.md)) |
| Desktop | A placeholder Wayland compositor with a terminal. The AmityOS compositor and shell are Phase 3 and 4 |
| Network | Wired networks connect automatically. Wi-Fi: `nmcli device wifi connect "<name>" password "<password>"` |
| Apps | `flatpak install flathub <app>`; `.deb` packages with `sudo apt install ./file.deb` |
| Updates | `sudo apt update && sudo apt upgrade` (image-based signed updates are Phase 8) |

## Reporting results

Record the machine model, GPU, what booted and what failed, and the output of
`journalctl -b -p warning`. Results belong in `docs/acceptance/` so the hardware state is
tracked from observed runs only.
