# 0008: The shell is a separate process with a private channel to the compositor

Status: accepted (Phase 3).

## Decision

`amity-shell` (taskbar, launcher, window switcher, desktop background) is its own
process, a Wayland client of `amity-comp`. The compositor starts it, restarts it when it
exits, and gives it two connected sockets that no other process can obtain:

1. **A privileged Wayland connection** (`WAYLAND_SOCKET`). Only this connection may use
   `wlr-layer-shell`, the protocol for surfaces that sit above or below windows and
   reserve screen space. Ordinary applications cannot create panels, overlays or
   full-screen covers. `AMITY_COMP_OPEN_LAYER_SHELL` opens the protocol to every client
   for development with third-party panels.
2. **The shell channel** (`AMITY_SHELL_FD`): one line of JSON per message, defined in
   `crates/amity-ipc` with a protocol version. The compositor sends the window list in
   most-recently-used order, the switcher state and launcher requests; the shell sends
   activate, minimize and close requests.

Window management policy (focus, stacking, placement, maximize, snapping, the switcher's
logic and all global shortcuts) stays in the compositor. The shell draws and reports
what the user chose. A shell crash therefore loses no window state: windows stay where
they are, shortcuts keep working, and the shell comes back within a second. After five
exits in a minute the compositor stops restarting it and logs an error.

## Why not something else

| Alternative | Why not |
|---|---|
| Draw the shell inside the compositor | Ties the compositor to a UI toolkit and makes every shell bug a session crash. The design's surfaces (launcher, control centre, assistant) are application-sized UI. |
| D-Bus for the channel | Any session process can talk to the session bus, so it would need its own access control; it also adds a start-up ordering dependency. D-Bus remains the choice for the service APIs (`amityd`, Spaces, apps, tasks), which are meant to be called by other programs. |
| A private Wayland protocol extension | The right long-term form. `iced_layershell` does not expose its Wayland connection to bind extra globals today. Revisit when the shell needs per-surface data such as window thumbnails. |
| `ext-foreign-toplevel-list` and `wlr-foreign-toplevel-management` | Would let third-party taskbars work, but exposes every window's title to any client. May be added later behind the same privilege check. |

## Privileged protocols

| Protocol | Who may use it |
|---|---|
| `wlr-layer-shell` | The shell only |
| `wlr-data-control` (clipboard access without focus), virtual keyboard, input method | Clients that are not in a sandbox (no Wayland security context) |
| `wp-security-context` | Clients that are not in a sandbox; Flatpak uses it to mark sandboxed clients |
| `keyboard-shortcuts-inhibit` | Offered, never granted for now. An application that could switch off the compositor's shortcuts could trap the user in its window. Granting it (remote-desktop and virtual-machine viewers want it) becomes a per-application permission with the app manager in Phase 5 |

## What applications can and cannot do to the shell

Window titles and application ids come from applications and end up in the taskbar and
the switcher. The compositor bounds them before they reach the shell: control characters
are replaced, each is cut at 127 bytes, and at most the 256 most recently used windows are
listed (`amity-ipc`: `bounded_text`, `MAX_TEXT_BYTES`, `MAX_WINDOWS`). A test builds the
largest possible list and checks that it stays far below the channel's line limit, so no
application can make the shell exit by sending oversized data.

While the launcher has the keyboard, shortcuts that act on the window behind it (close,
maximize, snap) are not shortcuts: the keys go to the launcher. The shortcuts that open
and close shell surfaces, the switcher and the virtual-terminal keys always work.

## Applications are started as systemd user services

The launcher starts each application with `systemd-run --user` as
`app-AmityOS-<desktop file id>-<n>.service` in `app.slice`, following systemd's
convention for desktop environments. Each application gets its own control group and
journal stream and does not depend on the shell process. Spaces (Phase 5) attach
resource limits and isolation to these units. Without a user service manager, as in a
nested development session, the program is started directly.

## Consequences

- `crates/amity-ipc` is shared by both sides; a protocol change is one reviewed change.
  Unknown message types are ignored, so either side can be newer.
- The shell runs unconnected when started by hand (no window list), which keeps UI work
  possible without the compositor.
- A development-only debug socket (`AMITY_COMP_DEBUG_SOCKET`, set in the development
  image) returns the compositor's state as JSON for the test harness. It is read-only,
  owner-only, and absent from release images.

## Licences

| Component | Licence | Linked? |
|---|---|---|
| serde, serde_json | MIT OR Apache-2.0 | yes |
| iced_layershell, layershellev | MIT | yes (shell) |
