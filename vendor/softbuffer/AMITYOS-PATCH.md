# softbuffer, patched for AmityOS

| | |
|---|---|
| Upstream | https://github.com/rust-windowing/softbuffer |
| Version | 0.4.8, as published on crates.io |
| Licence | MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`) |
| Used by | `iced_tiny_skia`, the software renderer of `amity-shell` |

## What is changed

One thing, in two places of `src/backends/wayland/buffer.rs`: Wayland buffers are
created with `wl_shm::Format::Argb8888` instead of `wl_shm::Format::Xrgb8888`. Both
places are marked with an `AmityOS patch` comment. Nothing else differs from the
published crate (its `Cargo.lock` and `Cargo.toml.orig` are not copied).

## Why

Upstream declares every buffer opaque. The compositor then ignores the alpha channel,
so a surface cannot be translucent or non-rectangular: the taskbar's rounded shape gets
an opaque box behind it, and the launcher and switcher overlays black out the screen.

`iced_tiny_skia` already writes premultiplied pixels in B, G, R, A byte order and clears
the background with a replacing blend, which is exactly what `ARGB8888` means on
Wayland. Declaring the real format is therefore all that is needed.

## Applications affected

Every program in this workspace that renders with `iced_tiny_skia`. A window whose
background is fully opaque looks the same as before.

## When to remove this copy

- when softbuffer offers a way to request an alpha channel on Wayland and
  `iced_tiny_skia` uses it, or
- when the shell renders through wgpu only.

Then delete `vendor/softbuffer`, the `[patch.crates-io]` entry and the `exclude` line in
the workspace `Cargo.toml`, and the row in `docs/licensing.md`.

## Updating

Copy the new upstream release over this directory, reapply the two-line change, and
update the version above.
