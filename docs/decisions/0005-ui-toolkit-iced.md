# 0005: iced for the shell and Amity apps; amity-ui for tokens and components

Status: accepted (default from the owner's brief, checked against the design screenshots).

## Decision

The shell and Amity apps use **iced** (MIT, pure Rust, GPU-rendered through wgpu, with
text by cosmic-text). One shared crate, `crates/amity-ui`, holds the design tokens and
every component.

## Fit with the screenshots

Reviewed in Phase 0: 01 Desktop, 04 Shell Design Language and 07 Settings (all 38 are
reviewed in Phase 4). The design is a dark theme of flat surfaces (Ground `#07071A`,
Surface `#0E0D22`, Raised `#131230`). It uses rounded containers on a 6/9/14/20/24 px
radius scale, 1 px borders, accent colours (Violet `#8B5CF6`, Blue `#60A5FA`, Orchid
`#F0ABFC`), switches, dropdowns, search fields, list rows, a focus ring, and a translucent
taskbar and launcher. iced draws all of these with its standard primitives and custom
widgets. The one effect iced cannot draw is **backdrop blur** behind the translucent
"Glass" surfaces; that belongs in the compositor ([0004](0004-compositor-smithay.md)).
No requirement forces another toolkit.

## Implementation notes (Phase 3)

- **Versions.** iced 0.14 with `iced_layershell` 0.19 (MIT), which lets an iced program
  own several layer-shell surfaces: desktop background, taskbar, launcher, switcher.
- **Renderer: software for now.** The shell renders with iced's software renderer
  (`tiny-skia`), not wgpu. It works on every machine including VMs without a GPU, gives
  identical pixels everywhere (useful for the Phase 4 comparisons), and keeps the build
  small. This departs from "GPU-rendered through wgpu" above. Phase 4 decides between
  wgpu and the software renderer when motion and shadows are implemented.
- **Two defects of the software path**, both found by looking at screenshots:
  1. *No transparency.* `softbuffer` declares every Wayland buffer opaque, so a
     translucent or rounded surface gets an opaque box. Fixed with a two-line patch in a
     vendored copy (`vendor/softbuffer`, see `AMITYOS-PATCH.md` there).
  2. *Shadows darken the surface.* `iced_tiny_skia` 0.14 draws a quad's shadow without
     the clip of a partial redraw, so each redraw darkens everything else again. Not
     patched; `amity-ui` uses no shadows until the renderer decision.
- **Pinned pre-release.** `iced_layershell` 0.19.1 does not compile against
  `winit-core` 0.31.0-beta.4; `Cargo.lock` pins `winit-core` and `winit-common` to
  0.31.0-beta.2, the versions it was released with.
- **`amity-ui`** has the tokens without any dependency (the compositor uses them) and,
  behind its `iced` feature, the styles the shell needs. Phase 4 completes it.
- **Icons.** Application icons are not drawn yet; a tile with the application's initials
  stands in. Phase 4 adds icon lookup (freedesktop icon themes) and the AmityOS icon set.

## Fonts

All three fonts in the design are under the SIL Open Font License 1.1, which allows
bundling and commercial use: Space Grotesk (display), Inter (UI) and JetBrains Mono
(code).

## Licences

| Component | Licence | Linked? |
|---|---|---|
| iced | MIT | yes |
| wgpu | MIT/Apache-2.0 | yes |
| cosmic-text | MIT/Apache-2.0 | yes |
| Space Grotesk, Inter, JetBrains Mono | OFL-1.1 | bundled font files |
| iced_layershell, layershellev | MIT | yes |
| tiny-skia | BSD-3-Clause | yes |
| softbuffer (vendored, patched) | MIT OR Apache-2.0 | yes |
