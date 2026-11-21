# Phase 3 acceptance: compositor and shell MVP

Date: 2026-11-21. Image: `amityos-0.1.0-devel.qcow2`, SHA-256 `1432402f330be6feb4ae6d52e660cace38032c590b436732a929c1e5946288c8`.

Acceptance criterion from the brief: *"amity-comp runs nested and on DRM/KMS. It provides
window management, XWayland, a panel or taskbar, a launcher, the MRU switcher, and
multi-monitor support. VS Code, Chrome and Blender run on it."*

**Result: met in a virtual machine and nested under WSLg. Not run on real hardware.**
Everything below was observed in the runs named; nothing is inferred from the code.

## What was built

- **`amity-comp`**, the AmityOS Wayland compositor, on Smithay 0.7. Its core started from
  Smithay's MIT-licensed example compositor and was reworked
  ([decision 0004](../decisions/0004-compositor-smithay.md)). AmityOS's own parts:
  window management (placement, focus and use order, minimize, maximize, snapping to
  halves, fullscreen, resize by the frame, following monitor changes), compositor-drawn
  title bars, the shortcuts of design screenshot 27, software rendering when no hardware
  renderer exists, and the link to the shell.
- **`amity-shell`**, a first shell on iced: desktop background, taskbar (launcher button,
  one entry per window, clock), launcher (search over installed applications), Alt+Tab
  overlay. A separate process that the compositor starts, supervises and restarts
  ([decision 0008](../decisions/0008-shell-process-and-channel.md)). It is functional,
  not yet the design: Phase 4 brings the look of the screenshots.
- **`amity-ipc`**: the versioned messages of the private compositor-shell channel.
- **`amity-apps`**: application catalogue from desktop entries (native packages and
  Flatpak exports), search, and launching each application as its own systemd user
  service.
- **Image:** the two programs and the wallpaper are part of the image; the Phase 2
  placeholder compositor (labwc, GPL) is gone. `amity-session.service` starts the
  compositor on the first virtual terminal.
- **Tooling:** a build root for compiling Rust against the image's libraries without
  installing anything into WSL; VM suites that drive the guest through its virtual
  keyboard and pointer and read the compositor's state; a second virtual monitor that can
  be plugged in and out; a nested check; a licence gate over all Rust dependencies.

## Results against the acceptance criterion

| Criterion | Result | Evidence |
|---|---|---|
| Runs nested | Passed under WSLg | [nested.log](phase-3/nested.log) |
| Runs on DRM/KMS | Passed in the QEMU VM (virtio GPU through the kernel's KMS, libinput, libseat; software rendering) | [single/results.json](phase-3/single/results.json) |
| Window management | Passed: see the table below | same |
| XWayland | Started in both modes; an X11 application was used through XWayland (Chrome and VS Code started as X11 clients) | same |
| Taskbar | Passed | same, [screenshots](phase-3/single/) |
| Launcher | Passed | same |
| MRU switcher | Passed | same |
| Multi-monitor | Passed with a second virtual monitor, hot-plugged and unplugged | [multi/results.json](phase-3/multi/results.json) |
| VS Code, Chrome and Blender run on it | Passed: all three started from the launcher, drew their windows, minimized, restored and closed | [single/results.json](phase-3/single/results.json), screenshots 20 to 22 |

## Runs

All on the image named above, built from the commit this note belongs to. The compositor and
shell under test are the ones inside the image (`/usr/bin/amity-comp`, `/usr/bin/amity-shell`;
the harness checks that nothing overrides them).

| Run | Command | Result | Record |
|---|---|---|---|
| One monitor, then applications | `.\AmityOS.ps1 vmtest -Suite phase3` on a new VM | **151 of 151 checks passed**; root shell 15 s after power-on | [single/results.json](phase-3/single/results.json) |
| Two monitors | `.\AmityOS.ps1 vmtest -Suite phase3-multi -KeepVm` | **39 of 39 checks passed** | [multi/results.json](phase-3/multi/results.json) |
| Nested | `.\AmityOS.ps1 nested` (WSLg on the development machine) | **5 of 5 checks passed** | [nested.log](phase-3/nested.log) |

VM: QEMU 8.2 with WHPX, 4 CPUs, 8 GB, virtio GPU without 3D acceleration, 1280 x 800
(second monitor 1024 x 768). The compositor logged that it renders in software.

Method: the guest's root serial console for commands; keyboard and pointer events
injected through QEMU into the guest's virtio keyboard and tablet, so they take the same
path as real devices (kernel, libinput, compositor); the compositor's state read from its
development-only debug socket; screenshots of the guest display through QEMU. No
automation of the Windows host's own input. The screenshots listed below were opened and
checked by eye.

## What was observed, by feature

Each row names what the suite did and saw. The check names in the result files are the
same phrases.

| Feature | Observed | Screenshot |
|---|---|---|
| Session start | `amity-session.service` active; `amity-comp` is the session compositor; its libraries all resolve; XWayland display `:0` up; shell connected; taskbar reserves 76 px at the bottom (work area 1280 x 724) | [10](phase-3/single/10-session.jpg) |
| Placement and focus | First window at the first cascade position (130, 68), inside the work area, with keyboard focus; second window cascaded (168, 102) and focused | [11](phase-3/single/11-two-windows.jpg) |
| Click to focus | A click on the window behind focuses and raises it | |
| MRU switcher | Alt+Tab switches to the previous window; holding Alt keeps the switcher open with the windows in use order; Tab steps and wraps; Shift+Tab steps back; Escape cancels and leaves focus unchanged | [12](phase-3/single/12-switcher.jpg) |
| Maximize, restore, snap | Super+Up fills the work area; Super+Down restores the previous geometry; Super+Left and Super+Right fill the left and right half; a snapped window can be maximized and still returns to its floating geometry | [14](phase-3/single/14-snapped.jpg) |
| Title bar (compositor-drawn) | Maximize, restore and minimize buttons work; minimizing moves focus to the next window and keeps the window in the list; Alt+Tab restores it where it was; dragging the bar moves the window; the close button closes it | |
| Resize by the frame | Dragging the bottom-right corner enlarges the window with the top-left corner fixed; dragging the left edge moves that edge and keeps the right one in place | [15](phase-3/single/15-resized.jpg) |
| Close | Ctrl+Q closes the active window and focus moves to the remaining one; with no window left nothing has focus | |
| Launcher | Super+Space opens and closes it; the taskbar button opens it; typing "foot" lists the terminal first; Enter starts it and the launcher closes; the new window has the keyboard | [16](phase-3/single/16-launcher.jpg) |
| Applications as services | The launched terminal runs as `app-AmityOS-foot-<n>.service`, `loaded active running`; it has `WAYLAND_DISPLAY`, `DISPLAY` and `XDG_CURRENT_DESKTOP=AmityOS`; the shell's private socket variables are not in its environment | |
| Taskbar | A press on the entry of the focused window minimizes it; a press on a minimized window's entry restores and focuses it | |
| Keyboard hand-back | With the launcher open the window stays the active window; Escape returns the keyboard to it | |
| Fullscreen | A window that asks for it opens fullscreen on the whole monitor without a title bar and covers the taskbar while active; a new window takes focus and the taskbar is back; Alt+Tab to the fullscreen window covers again; the launcher opens above it; Ctrl+Q typed into the launcher does not close the window behind; Super+Up leaves fullscreen for maximized, title bar back | [19a](phase-3/single/19-fullscreen-behind.jpg), [19b](phase-3/single/19-launcher-over-fullscreen.jpg) |
| Virtual terminals | Ctrl+Alt+F2 switches to tty2 and Ctrl+Alt+F1 back; afterwards a new window maps, takes focus and closes | |
| Chrome 158 | Installed from Google's `.deb`; started from the launcher ("chrome", Enter); native Wayland window, inside the work area; 11 renderer processes; runs as `app-AmityOS-google_chrome-<n>.service`; taskbar minimize and restore; closes on Ctrl+Q | [20](phase-3/single/20-chrome.jpg) |
| VS Code 1.147 | Installed from Microsoft's `.deb`; started from the launcher ("visual studio", Enter); native Wayland window showing its first-run sign-in dialog; 11 processes; minimize, restore, close | [21](phase-3/single/21-vscode.jpg) |
| Blender 5.3 | Installed from Flathub; started from the launcher ("blender", Enter); native Wayland window with the compositor's title bar, opened maximized, splash screen drawn; runs in the Flatpak sandbox; closes on Ctrl+Q | [22](phase-3/single/22-blender.jpg) |
| XWayland | Chrome with `--ozone-platform=x11`: X11 window with keyboard focus, taskbar minimize and restore, Super+Up and Super+Down. VS Code with `--ozone-platform=x11` and a window-manager title bar: gets the compositor's title bar; moved by the bar; resized by the left frame edge; maximize, restore, minimize and close buttons; Alt+Tab restores it where it was. XWayland still running afterwards | [24](phase-3/single/24-vscode-x11.jpg) |
| Two monitors | A second monitor appears when connected, to the right of the first; the first keeps its taskbar; existing windows stay; a window opens on the monitor the pointer is on, inside it; maximize fills that monitor; a window is dragged to the other monitor and back; maximize there fills that monitor's work area | [31](phase-3/multi/31-output-2.jpg), [32](phase-3/multi/32-output-2-window.jpg) |
| Monitor unplugged | With one window maximized and one minimized on the second monitor: the monitor disappears; the maximized window is maximized on the remaining monitor; the window on the first monitor does not move; the minimized window stays minimized and listed, and restoring it shows it fully inside the remaining work area; all three windows are still there | [33](phase-3/multi/33-after-unplug.jpg) |
| Stability | In every run: no panic, no shell exit and no restart of the session unit in the journal from session start to the end of the suite | |

Screenshots are JPEG re-encodings (quality 85) of the PNG captures, to keep the repository
small; all of them were opened and checked. The captures themselves stay in
`image\out\acceptance\` on the development machine.

## Unit tests

`.\AmityOS.ps1 test`: 59 tests, all passing. `amity-comp` 23 (the spec's
fitting and cascade test vectors, tiling rectangles, drag limits, resize edges, monitor
relocation, switcher stepping, title-bar zones and double-click, shortcut mapping);
`amity-apps` 19 (desktop-entry parsing, `Exec` expansion and quoting, catalogue priority
and masking, search ranking, launch command lines, a real process started and reaped);
`amity-ipc` 7 (message round trips, framing, bounds); `amity-shell` 6; `amity-ui` 4.

`.\AmityOS.ps1 licenses`: 221 linked crates, 52 used only at build time, none under a
licence the rules forbid ([licensing/crates.md](../licensing/crates.md)).

## Code review

Two independent review passes (your `code-reviewer` agent). The first covered all new
code; the second covered what was written in response to the first.

**First pass: 12 findings (3 major, 9 minor) and 5 questions.** It confirmed the privilege
model (only the shell can use layer-shell; the private sockets do not leak to
applications; launch arguments cannot be injected through desktop-entry fields).

| Finding | What was done |
|---|---|
| Major: an application could make the shell exit for good by sending huge window titles or opening very many windows | Fixed: titles and ids are bounded before they reach the shell, the list is capped, and a test builds the largest possible message. The spec's per-application window limits are still not enforced |
| Major: fullscreen windows hid the taskbar, launcher and other windows no matter which window was active | Reworked: fullscreen is now window-manager state; covered by the fullscreen checks above |
| Major: unplugging a monitor left maximized windows the wrong size and could restore minimized windows off-screen | Fixed; covered by the unplug checks above |
| Minor: a shell surface created while no monitor exists crashed the compositor | Fixed (the surface is refused); not exercised |
| Minor: Ctrl+Q and the Super+arrow shortcuts acted on the hidden window while the launcher was open | Fixed; unit-tested and checked in the VM |
| Minor: any application could switch off all compositor shortcuts, including the way to a text console | Changed: such requests are no longer granted to anyone until there is a per-application permission (Phase 5) |
| Minor: a shell that exits without a crash message could go unnoticed by the suite | Fixed: the exit status is logged and the suite looks for both messages |
| Minor: a window minimized before its first content never received its first configure | Fixed; not exercised |
| Minor: dragging a window that is minimized or closed mid-drag could bring it back | Fixed (the drag ends); not exercised |
| Minor: a failing `systemd-run` was silent, and a missing working directory broke the launch | Partly fixed: the failure is logged and a missing directory falls back to the home directory. The launcher still closes without showing the error |
| Minor: the debug socket can stall the compositor for up to 200 ms per connection | Reduced to 50 ms and documented. Development image only |
| Minor (tooling): four checks that could not fail or missed their point | Three fixed. The file-descriptor inheritance check still looks only at the environment of a systemd-launched application |
| Question: do minimized X11 windows drop out of the window list? | No: read in Smithay's source and checked in the VM with Chrome and VS Code |
| Questions on the drag limit and on result ranking differing from the spec | Deliberate; recorded in section 15 of the spec and in the table below |

**Second pass: 5 minor findings**, all in the fullscreen and monitor code written after the
first pass: two fullscreen windows meeting on one monitor after an unplug; small windows
being enlarged when moved to another monitor; the resize band taking clicks meant for a
menu that overlaps the frame; move and resize requests on a fullscreen window; two
requests that were not answered with a configure. All five were fixed.

**Not reviewed:** the fixes for the second pass, and the changes made after it (limiting
new windows to the work area, rounding in the resize code, the X11 tests). They are covered
by the unit tests and the acceptance runs above, which were made after them, but no
reviewer has read them.

## Problems found and fixed on the way

- **The compositor refused to start without a hardware renderer** (any VM without 3D
  acceleration). It now falls back to software rendering and logs it.
- **Ubuntu 26.04 ships libdisplay-info 0.3**; Smithay's helper wants 0.2. The monitor-name
  lookup is done with the 0.3 bindings directly.
- **Shell surfaces could not be translucent**, and **shadows darkened the taskbar a little
  more on every redraw**: two defects of the toolkit's software rendering path, found by
  looking at screenshots. The first is fixed by a two-line patch in a vendored copy of
  `softbuffer`; the second is avoided by not using shadows yet
  ([decision 0005](../decisions/0005-ui-toolkit-iced.md)).
- **A pre-release of a windowing library broke the build** of `iced_layershell`; the
  working version is pinned (`dependencies.lock.json`).
- **Clicks went to the wrong place after a window moved under a resting pointer**, found
  by the two-monitor test. The compositor now re-evaluates pointer focus when the scene
  changes.
- **Chrome opened taller than the work area**, under the taskbar. New windows are now
  limited to the work area.
- **The guest kernel sometimes panicked at boot under WHPX** ("IO-APIC + timer doesn't
  work") when the host was busy. The development image boots with `no_timer_check`; the
  harness now recognises a boot panic at once instead of waiting out its timeout.
- **The pinned QEMU build crashes on any `-smbios type=11` option**, which ruled out the
  cleaner per-VM way of passing that kernel option.
- **The nested compositor could not create its X11 socket** under WSLg (read-only socket
  directory). The build root now gives it a writable one.

## Not done or not verified

- **Real hardware: not run.** No physical GPU, monitor, keyboard, mouse, touchpad or
  touchscreen has been used with the compositor. Hardware-accelerated rendering (the
  normal path on a real machine), real EDID, monitor hot-plug on real connectors,
  HiDPI scaling, variable refresh rate and multi-GPU machines are all untested. The VM
  used software rendering throughout.
- **Multi-monitor was tested with two virtual monitors only**, side by side, at scale 1.
  There is no way yet to arrange monitors, choose a mode or set a scale (Settings,
  Phase 4). A fullscreen window on a monitor that is then unplugged is handled in the
  code but was not exercised.
- **XWayland:** covered by Chrome and VS Code as X11 clients only. No other X11
toolkit (GTK 2, Qt on X11, Java, Wine) was tried, nor X11 menus on a second monitor,
clipboard or drag and drop between X11 and Wayland applications.
- **Not implemented in this phase:** application grouping, pinning and menus in the
  taskbar; application icons (initials stand in); the design's launcher content
  (assistant prompt, projects, actions); notifications, control centre, lock screen;
  workspaces and Spaces; snap layouts beyond left and right halves; screenshots and screen
  sharing (no screen-capture protocol or portal backend yet); pointer shapes other than
  the default arrow for compositor-drawn parts (resize edges give no visual hint); window
  titles in compositor-drawn title bars; rounded corners, blur, shadows and animation
  (Phase 4); keyboard layout and repeat settings (US layout only); touch and tablet
  tuning; input methods; session save and restore (Phase 5).
- **Accessibility** has not been assessed (Phase 4): contrast of the token colours is
  unit-tested, nothing else.
- **Performance** was not measured. Software rendering in a VM is not representative.
- **Applications were exercised only as far as stated**: start, first window, minimize,
  restore, close. No editing, browsing, rendering, GPU work, file dialogs, drag and drop,
  clipboard between applications, audio or video.
- **The release-profile image** was again not built or booted; only the development
  profile.
- Open items carried over from Phase 2: no installable ISO, archive snapshot not pinned,
  development account `amity`/`amity`.

## Differences from the recorded behaviour

`docs/specs/shell-behaviour.md` records the earlier prototype's behaviour. Section 15 of
that document lists, per section, what is implemented and where this phase deliberately
differs. The differences that change what a user experiences:

| Topic | Earlier behaviour | Now | Why |
|---|---|---|---|
| Launcher shortcut | Ctrl+Space | Super+Space | the design's shortcut sheet (screenshot 27) |
| Switcher | Alt+Tab and Ctrl+Tab | Alt+Tab only | Ctrl+Tab belongs to browser and editor tabs |
| Close window | Ctrl+Alt+Q | Ctrl+Q | the shortcut sheet |
| Dragging a window | whole window stays on screen | title bar stays reachable; part of the window may leave the screen | needed to move windows between monitors; usual on other desktops |
| Launcher results | plain substring order | ranked: name prefix first | better results as the list grows |

Each is a one-line policy and can be reversed.

## For the owner to decide or do

1. **GitHub Support request from Phase 1 is still open** on your side: GitHub served the
   purged old commit `077a4c3` by hash after the history rewrite. Only GitHub Support can
   garbage-collect it.
2. **Licences beyond the brief's list.** One linked crate, `libloading`, is ISC-licensed
   (equivalent to two-clause BSD). The gate accepts ISC and a few other permissive licences
   and reports them separately ([licensing.md](../licensing.md)). Confirm or refuse.
3. **Differences from the recorded behaviour** in the table above: keep or reverse.
4. **Renderer for the shell.** The shell renders in software today, not through wgpu as
   decision 0005 said, and carries a two-line patch to a third-party crate for
   translucency. Shadows, blur and motion in Phase 4 need a decision: stay on the software
   renderer and patch its shadow defect, or move to wgpu.
5. **Things noticed and not acted on** (none blocks the acceptance criterion):
   - The pointer does not change shape over resize edges, and applications cannot ask the
     compositor for named cursors (the terminal logs that it falls back to its own).
   - Compositor-drawn title bars show no window title.
   - When `systemd-run` fails after it has been started, the launcher closes without
     telling the user.
   - The spec's limits of 64 windows and 16 per application are not enforced.
   - Applications that are only D-Bus activatable (no `Exec` line) are not listed.
   - VS Code and Chrome chose native Wayland by themselves; whether that should be forced
     either way for some applications is undecided.
   - `tooling/vm_harness.py` has grown to cover three suites in one file.
