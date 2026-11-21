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
