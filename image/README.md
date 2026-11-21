# AmityOS image

mkosi configuration for the AmityOS disk image
([decision 0002](../docs/decisions/0002-base-os-and-image.md)). Build it with
`.\AmityOS.ps1 image`; outputs go to `image/out/` (git-ignored) on F:.

| Path | Purpose |
|---|---|
| `mkosi.conf` | base distribution, package list, boot settings |
| `mkosi.repart/` | partition layout of the built image (ESP + btrfs root) |
| `mkosi.extra/` | files copied into the image: session unit, first-boot partition growth, Flathub setup, NetworkManager override |
| `mkosi.postinst.chroot` | branding, development account, locale, enabled services |
| `mkosi.profiles/devel/` | development-only additions: root shell on the serial port for the test harness |

The installer is Phase 8.
