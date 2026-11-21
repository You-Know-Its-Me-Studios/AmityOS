# 0007: Applications: Flatpak first, native packages where needed

Status: accepted (default from the owner's brief).

## Decision

- **Flatpak + Flathub** are enabled by default. `amity-apps` uses per-Space Flatpak
  overrides (filesystem, network and device grants) and portals to implement per-Space
  app profiles and grants.
- **Native `.deb` packages** for tools that need host integration: Docker, the NVIDIA
  driver/CUDA, ROCm, and any developer toolchain that does not work sandboxed.
- **Amity Store** (screenshot 11) and the install/permission prompt (screenshot 22) are
  the front end for both sources.
- **First-run "Developer & Designer setup"** (screenshot 06) installs the toolchains the
  user selects. It also installs anything that cannot be redistributed in the image
  (see [0003](0003-graphics-stack.md)).
- **Wine/Proton** is optional, from the distro or Flathub. It is not built by AmityOS.
- **App lifecycle** (installed, enabled in a Space, autostart, running) is a clean state
  model in `amity-apps` with a D-Bus API, specified in `docs/specs/apps-lifecycle.md`.

## Licences

| Component | Licence | Linked? |
|---|---|---|
| Flatpak, libflatpak | LGPL-2.1-or-later | `amity-apps` calls the `flatpak` CLI or D-Bus (separate process), or links libflatpak dynamically only |
| APT/dpkg | GPL-2.0 | no (separate process via PackageKit or `apt`) |
| PackageKit | GPL-2.0 | no (D-Bus) |
