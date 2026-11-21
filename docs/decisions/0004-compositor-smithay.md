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

## Implementation notes (Phase 3)

- **Starting point.** The compositor core (backends, protocol handlers, rendering loop)
  started from Smithay's own example compositor, `anvil`, at tag v0.7.0 (commit
  `a166cf4c`). It is MIT-licensed like Smithay; its licence text is
  `crates/amity-comp/LICENSE-SMITHAY.txt`. This is within the constraint above: no code
  comes from a GPL or LGPL compositor. AmityOS's own parts are window management
  (`wm.rs`, `layout.rs`), title bars (`shell/ssd.rs`), shortcuts and focus
  (`input_handler.rs`), the shell link (`shell_link.rs`) and the debug endpoint.
- **Software rendering.** When the primary GPU has no hardware renderer (a VM without 3D
  acceleration, a GPU without a driver), the compositor renders with Mesa's software
  renderer instead of refusing to start. The log says so.
- **Window management** follows `docs/specs/shell-behaviour.md` (placement, focus,
  minimize, maximize, the switcher) with the shortcuts of design screenshot 27. Snapping
  to the left and right half is implemented; snap layouts and workspaces are not yet.
  A new window is never larger than the work area. Windows with the compositor's title
  bar are resized by a band inside their frame; windows that draw their own frame resize
  themselves. Section 15 of the spec lists what is implemented and what differs.
- **The rules are pure functions** in `layout.rs` (fitting, cascade, tiling rectangles,
  drag limits, resize edges, monitor relocation, switcher stepping) with unit tests,
  including the spec's test vectors. The stateful part (`wm.rs`) is covered by the VM
  suites, which read the compositor's state after real key and pointer events.
- **Fullscreen** belongs to the window manager. A fullscreen window covers its monitor,
  taskbar included, only while it is the active window and neither the launcher nor the
  switcher is open. Switch to another window and the monitor is drawn normally again,
  with the fullscreen window still fullscreen-sized behind it; switch back and it covers
  again. Maximizing or snapping a fullscreen window takes it out of fullscreen first.
- **Monitor layout changes.** When a monitor is added, moved or removed, each window
  follows its monitor: it shifts with a monitor that moved, and goes to a remaining
  monitor when its own is gone, keeping its arrangement (a maximized window is maximized
  there, a fullscreen one fullscreen, a minimized one reappears there when restored).
  The rule is a pure function with tests (`layout::relocate`).
- **Pointer focus follows the scene.** When a window opens, closes, moves or is raised
  under a pointer that does not move, the compositor re-evaluates what is under the
  pointer, so the next click goes to what is visible there.
- **Monitor names** are read from EDID with `libdisplay-info` 0.3, which Ubuntu 26.04
  ships; `smithay-drm-extras` 0.1 binds 0.2, so its helper is not used.
- **Privileged protocols** and the shell process: decision
  [0008](0008-shell-process-and-channel.md).
- **Not done yet:** blur and rounded window corners (Phase 4), per-Space state
  (Phase 5), output configuration (scale, arrangement, mode) from Settings, screen
  capture and screen sharing, tablet and touch tuning, HDR, variable refresh rate.

## Licences

| Component | Licence | Linked? |
|---|---|---|
| Smithay | MIT | yes |
| wayland-rs | MIT | yes |
| libinput, libseat (system libraries) | MIT | yes (dynamic) |
| XWayland | MIT | no (separate process) |
| smithay-drm-extras | MIT | yes |
| libdisplay-info, libgbm, pixman, libxkbcommon, libwayland (system libraries) | MIT | yes (dynamic) |
| libudev (system library) | LGPL-2.1-or-later | yes (dynamic, as the licensing rules require) |
