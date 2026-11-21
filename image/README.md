# AmityOS image

mkosi configuration for the AmityOS disk image
([decision 0002](../docs/decisions/0002-base-os-and-image.md)). Build it with
`.\AmityOS.ps1 image`; outputs go to `image/out/` (git-ignored) on F:.

The image is Ubuntu packages plus the AmityOS programs from this repository. The build
script (`tooling/image_build.sh`) compiles `amity-comp` and `amity-shell` in release mode
in the build root, strips their debug information, and adds them with the default
wallpaper as an extra tree: `/usr/bin/amity-comp`, `/usr/bin/amity-shell`,
`/usr/share/amityos/wallpapers/`.

| Path | Purpose |
|---|---|
| `mkosi.conf` | base distribution, package list (including the libraries the AmityOS programs load), boot settings |
| `mkosi.repart/` | partition layout of the built image (ESP + btrfs root) |
| `mkosi.extra/` | files copied into the image: session unit and start script, first-boot partition growth, Flathub setup, NetworkManager override |
| `mkosi.postinst.chroot` | branding, development account, locale, enabled services |
| `mkosi.profiles/devel/` | development-only additions: a root shell on the serial port and the compositor's debug socket for the test harness, and `no_timer_check` for the QEMU/WHPX VM. Never ship an image built with this profile |

The session: `amity-session.service` logs the user in on the first virtual terminal and
runs `/usr/libexec/amity-session`, which starts `amity-comp`. The compositor starts
XWayland and `amity-shell`. A build placed in `/usr/local/bin` takes precedence over the
one in the image, which is how the test harness tries a new build without rebuilding the
image (`.\AmityOS.ps1 vmtest -Suite phase3 -DevBuild`).

The installer is Phase 8.
