# Licensing

AmityOS is a commercial product. Rules:

- Never link GPL code into a component that may stay closed: `amity-comp`, `amity-shell`,
  `amity-ui`, `amity-ipc`, `amity-spaces`, `amity-apps`, `amity-tasks`, `amityd`, and the
  Amity apps.
- MIT, Apache-2.0, BSD, Zlib and MPL-2.0 may be linked. LGPL may be linked dynamically
  only.
- Linux kernel changes are GPL-2.0 and are published when distributed.
- NVIDIA, CUDA and ROCm components are installed at first run, not bundled in a
  distributed image, until their redistribution terms are recorded here.
- Every new dependency is recorded in the same change that adds it.

## Owned code

All crates in `crates/` and `apps/` are AmityOS code (`LicenseRef-AmityOS-Proprietary` in
`Cargo.toml`, `publish = false`).

One part of `crates/amity-comp` did not start as AmityOS code: the compositor core was
copied from Smithay's example compositor "anvil" at Smithay v0.7.0 and reworked. It is
MIT-licensed (© 2017 Victor Berger and Victoria Brekenfeld); the licence text is kept as
`crates/amity-comp/LICENSE-SMITHAY.txt` and must ship with the compositor. See
[decision 0004](decisions/0004-compositor-smithay.md).

## Rust dependencies

The full list, one row per crate with version, licence and the programs it is linked
into, is generated: **[licensing/crates.md](licensing/crates.md)**.

```powershell
.\AmityOS.ps1 licenses    # regenerates the list and fails on a licence the rules forbid
```

The generator (`tooling/licenses.py`) reads `cargo metadata` for the Linux target with the
versions pinned in `Cargo.lock`. It separates crates that end up in a program from those
that only run while compiling (build scripts, procedural macros), and it fails when a
linked crate has no licence the rules allow. Run it whenever `Cargo.lock` changes.

**For the owner to confirm.** The brief names MIT, Apache-2.0, BSD, Zlib and MPL-2.0.
One linked crate is available only under a different permissive licence: `libloading`
(ISC), which loads the system's Wayland and xkbcommon libraries at run time. ISC is
equivalent in effect to the two-clause BSD licence: no copyleft, nothing that restricts
keeping AmityOS closed. The gate accepts it but reports it as "permissive, not named"
in the generated list instead of treating it as covered by the brief. The same applies to
the other licences in `OTHER_PERMISSIVE` in `tooling/licenses.py` (0BSD, BSL-1.0,
CC0-1.0, Unicode-3.0 and similar) should a future dependency need one. To refuse one,
remove it from that set; the gate then fails until the crates that need it are replaced.

### Direct dependencies

| Dependency | Licence | Used by | Why |
|---|---|---|---|
| Rust standard library | MIT OR Apache-2.0 | all crates | |
| smithay, smithay-drm-extras | MIT | `amity-comp` | compositor building blocks ([0004](decisions/0004-compositor-smithay.md)) |
| libdisplay-info (Rust bindings) | MIT | `amity-comp` | monitor names from EDID |
| xkbcommon, x11rb, xcursor | MIT, MIT OR Apache-2.0, MIT | `amity-comp` | keyboard layouts, XWayland window management, cursor themes |
| tracing, tracing-subscriber, profiling, thiserror, bitflags | MIT, MIT OR Apache-2.0 | `amity-comp` | logging and utilities |
| iced | MIT | `amity-shell`, `amity-ui` | UI toolkit ([0005](decisions/0005-ui-toolkit-iced.md)) |
| iced_layershell | MIT | `amity-shell` | layer-shell surfaces for iced |
| image (PNG and JPEG decoders only) | MIT OR Apache-2.0 | `amity-shell` | the wallpaper |
| chrono | MIT OR Apache-2.0 | `amity-shell` | the clock |
| serde, serde_json | MIT OR Apache-2.0 | `amity-ipc`, `amity-comp` | shell channel messages |

### Patched third-party code in this repository

| Component | Licence | Change |
|---|---|---|
| `vendor/softbuffer` (softbuffer 0.4.8) | MIT OR Apache-2.0 | two lines: Wayland buffers are created with an alpha channel. Reason and removal conditions: `vendor/softbuffer/AMITYOS-PATCH.md` |

### System libraries loaded at run time (dynamic linking)

| Library | Licence | Loaded by |
|---|---|---|
| glibc, libgcc | LGPL-2.1-or-later; GPL-3.0-or-later with the GCC Runtime Library Exception | every program |
| libwayland-server, libwayland-client, libwayland-egl, libxkbcommon | MIT | `amity-comp`, `amity-shell` |
| libseat, libinput, libgbm and the Mesa EGL/GLES libraries, libdisplay-info, pixman | MIT | `amity-comp` |
| libudev (systemd) | LGPL-2.1-or-later | `amity-comp` |

LGPL libraries are linked dynamically only, as the rules require.

## Bundled assets

| Asset | Licence | Notes |
|---|---|---|
| `assets/wallpapers/amity-default.png` and the Amity emblem cut from it | owner-supplied artwork; rights remain with the owner | see `assets/icons/BRAND-SOURCE.md`; installed in the image as the default wallpaper |
| `assets/icons/source/*.svg` | MIT (`assets/icons/LICENSE`) | original designs |
| `crates/amity-comp/resources/cursor.rgba`, `numbers.png` | MIT (from Smithay's anvil) | fallback pointer image when no cursor theme is installed; the digit sheet is unused |

## Operating system image

The image is assembled from Ubuntu 26.04 packages plus the AmityOS programs. The packages
are separate programs; none is linked into an owned component except through the dynamic
libraries listed above. The full package list with versions for each build is the image
manifest (`image/out/*.manifest.json`; the Phase 2 manifest is in
`docs/acceptance/phase-2/`, and the package names and versions of the Phase 3 image are in
`docs/acceptance/phase-3/image-packages.json`). Each package's licence text is installed
under `/usr/share/doc/<package>/copyright` in the image.

| Component | Licence | Notes |
|---|---|---|
| Linux kernel (Ubuntu HWE 7.0) | GPL-2.0 | unmodified Ubuntu build; no AmityOS patches |
| `linux-firmware` | mixed, per-file redistributable firmware licences | from the Ubuntu archive |
| systemd, systemd-boot | LGPL-2.1-or-later | |
| NetworkManager | GPL-2.0-or-later / LGPL-2.1-or-later | |
| Mesa, XWayland, PipeWire, WirePlumber | MIT | |
| xdg-desktop-portal, Flatpak | LGPL-2.1-or-later | |
| foot (terminal until the Amity terminal exists) | MIT | separate program |
| adwaita-icon-theme (pointer cursors and application icons until the AmityOS icon set) | LGPL-3.0-or-later / CC-BY-SA-3.0 | data files, not linked; replaced in Phase 4 |
| Inter, JetBrains Mono, Noto fonts | OFL-1.1 | |

No longer in the image: labwc (GPL-2.0, the placeholder compositor of Phase 2), grim and
wlr-randr.

Not in the image: NVIDIA drivers, CUDA, ROCm, Chrome, VS Code and Blender. The user
installs those; they were installed inside test VMs only.

## Development tools (not distributed)

| Tool | Version | Licence |
|---|---|---|
| Rust toolchain | 1.91.1 | MIT OR Apache-2.0 |
| QEMU for Windows (VM launch) | 8.2.0 | GPL-2.0 (separate program) |
| edk2/OVMF firmware bundled with that QEMU (VM boot) | as bundled | BSD-2-Clause-Patent |
| mkosi (image build) | v27.1 | LGPL-2.1-or-later |
| Smithay source checkout (API reference while developing) | v0.7.0 | MIT |

## Planned components

Their licences are recorded in decision records
[0002](decisions/0002-base-os-and-image.md) to [0007](decisions/0007-apps-and-packaging.md)
and move into the tables above when the code that uses them lands: wgpu and cosmic-text
(MIT/Apache-2.0), alacritty_terminal (Apache-2.0), llama.cpp (MIT), and the font Space
Grotesk (OFL-1.1).
