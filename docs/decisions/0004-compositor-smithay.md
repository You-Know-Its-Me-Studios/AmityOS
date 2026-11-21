# 0004: amity-comp, a Rust Wayland compositor on Smithay

Status: accepted (default from the owner's brief).

## Decision

`crates/amity-comp` is built on Smithay (MIT). It has two backends:

- **winit/X11 (nested):** runs inside a window on the dev host (WSLg) for fast
  iteration and screenshot capture in Phase 4.
- **DRM/KMS + libinput + GBM:** runs on real hardware and in the QEMU image.

It provides xdg-shell and layer-shell for the shell surfaces, XWayland, multiple
monitors, Space-aware window placement and session state, a per-Space clipboard, and the
blur and translucency in the design ("Glass" `#0E0C22` at 74 %, screenshot 04). Blur is
done by the compositor because clients cannot sample what is behind them.

## Constraints

- Do not copy code from cosmic-comp, niri, Hyprland, wlroots compositors or any other
  GPL/LGPL compositor. Their source can inform behaviour only through public
  documentation. This keeps `amity-comp` licensable as proprietary.
- Smithay is pinned to an exact version in `Cargo.lock` and `dependencies.lock.json`.

## Licences

| Component | Licence | Linked? |
|---|---|---|
| Smithay | MIT | yes |
| wayland-rs | MIT | yes |
| libinput, libseat (system libraries) | MIT | yes (dynamic) |
| XWayland | MIT | no (separate process) |
