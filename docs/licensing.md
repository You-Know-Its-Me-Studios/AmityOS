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

## Development tools (not distributed)

| Tool | Version | Licence |
|---|---|---|
| Rust toolchain | 1.91.1 | MIT OR Apache-2.0 |
| QEMU for Windows (VM launch) | 8.2.0 | GPL-2.0 (separate program) |
| OVMF firmware (VM boot) | 2024.02 | BSD-2-Clause-Patent |

## Planned components

Their licences are recorded in decision records
[0002](decisions/0002-base-os-and-image.md) to [0007](decisions/0007-apps-and-packaging.md)
and move into the tables above when the code that uses them lands: Smithay (MIT), iced
(MIT), wgpu and cosmic-text (MIT/Apache-2.0), alacritty_terminal (Apache-2.0), llama.cpp
(MIT), the fonts Space Grotesk, Inter and JetBrains Mono (OFL-1.1), and the Ubuntu base
(mixed; separate programs).
