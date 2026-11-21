# Licensing

AmityOS is a commercial product. Rules:

- Never link GPL code into a component that may stay closed: `amity-comp`, `amity-shell`,
  `amity-ui`, `amity-spaces`, `amity-apps`, `amity-tasks`, `amityd`, and the Amity apps.
- MIT, Apache-2.0, BSD, Zlib and MPL-2.0 may be linked. LGPL may be linked dynamically
  only.
- Linux kernel changes are GPL-2.0 and are published when distributed.
- NVIDIA, CUDA and ROCm components are installed at first run, not bundled in a
  distributed image, until their redistribution terms are recorded here.
- Every new dependency is added to this file in the same change that adds it.

## Owned code

All crates in `crates/` and `apps/` are AmityOS code (`LicenseRef-AmityOS-Proprietary` in
`Cargo.toml`, `publish = false`).

## Linked dependencies

As of Phase 1, `Cargo.lock` contains only the eleven workspace crates. **No third-party
crate is linked yet.**

| Dependency | Version | Licence | Used by | Linked into a proprietary component? |
|---|---|---|---|---|
| Rust standard library | 1.91.1 | MIT OR Apache-2.0 | all crates | yes (permitted) |

## Bundled assets

| Asset | Licence | Notes |
|---|---|---|
| `assets/wallpapers/amity-default.png` and the Amity emblem cut from it | owner-supplied artwork; rights remain with the owner | see `assets/icons/BRAND-SOURCE.md` |
| `assets/icons/source/*.svg` | MIT (`assets/icons/LICENSE`) | original designs |

## Operating system image

The image is assembled from Ubuntu 26.04 packages. They are separate programs; none is
linked into an owned component. The full package list with versions for each build is the
image manifest (`image/out/*.manifest.json`; the Phase 2 one is in
`docs/acceptance/phase-2/image-manifest.json`). Each package's licence text is installed
under `/usr/share/doc/<package>/copyright` in the image.

| Component | Licence | Notes |
|---|---|---|
| Linux kernel (Ubuntu HWE 7.0) | GPL-2.0 | unmodified Ubuntu build; no AmityOS patches |
| `linux-firmware` | mixed, per-file redistributable firmware licences | from the Ubuntu archive |
| systemd, systemd-boot | LGPL-2.1-or-later | |
| NetworkManager | GPL-2.0-or-later / LGPL-2.1-or-later | |
| Mesa, XWayland, PipeWire, WirePlumber | MIT | |
| xdg-desktop-portal, Flatpak | LGPL-2.1-or-later | |
| labwc (placeholder compositor until `amity-comp`) | GPL-2.0-only | separate program; to be removed in Phase 3 |
| foot (placeholder terminal) | MIT | to be replaced by the Amity terminal |
| Inter, JetBrains Mono, Noto fonts | OFL-1.1 | |

Not in the image: NVIDIA drivers, CUDA, ROCm, Chrome and VS Code. The user installs
those; Chrome and VS Code were installed inside a test VM only.

## Development tools (not distributed)

| Tool | Version | Licence |
|---|---|---|
| Rust toolchain | 1.91.1 | MIT OR Apache-2.0 |
| QEMU for Windows (VM launch) | 8.2.0 | GPL-2.0 (separate program) |
| edk2/OVMF firmware bundled with that QEMU (VM boot) | as bundled | BSD-2-Clause-Patent |
| mkosi (image build) | v27.1 | LGPL-2.1-or-later |

## Planned components

Their licences are recorded in decision records
[0002](decisions/0002-base-os-and-image.md) to [0007](decisions/0007-apps-and-packaging.md)
and move into the tables above when the code that uses them lands: Smithay (MIT), iced
(MIT), wgpu and cosmic-text (MIT/Apache-2.0), alacritty_terminal (Apache-2.0), llama.cpp
(MIT), the fonts Space Grotesk, Inter and JetBrains Mono (OFL-1.1), and the Ubuntu base
(mixed; separate programs).
