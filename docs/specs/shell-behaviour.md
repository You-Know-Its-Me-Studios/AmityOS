# Shell behaviour specification

This document records the interaction behaviour of the original AmityOS desktop shell: the taskbar, launcher, MRU window switcher, Desktop (Space) selector, window management, notices, tooltips and panel keyboard navigation. It was derived from the original AmityOS implementation (the no_std Rust compositor `userspace/src/bin/desktop.rs` and its helpers `desktop_shell.rs`, `switcher.rs`, `window_geometry.rs`, `session_policy.rs`, `config.rs` and `applications.rs`), with the taskbar, desktop and Spaces documents used only where they agree with the code. It is the behavioural reference for the Linux port (Smithay compositor plus iced shell). Visual styling such as colours, artwork, fonts and pixel layout is deliberately out of scope. Where the original code had an evident defect or quirk, this document says so and records the intended behaviour as an open question rather than silently changing it. Items marked "(inferred)" were deduced from code structure rather than stated by code comments or docs.

Session persistence and restore are specified separately in [session-restore.md](session-restore.md).

---

## 0. Conventions

| Term | Meaning |
|---|---|
| Space / Desktop | A named, storage-backed activity context. User-visible text calls it **Desktop**; code and protocol call it Space or workspace. Exactly one Space is *visible (active)* at a time. |
| Window | A top-level client surface managed by the compositor. Each window belongs to exactly one Space, fixed at creation from the owning process's Space. |
| App group | The taskbar entry for one application identity within the visible Space. |
| Built-in apps | Five catalogue slots with fixed IDs: 0 Files, 1 Text editor, 2 Terminal, 3 Settings, 4 Browser. Installed packages occupy further slots (up to 32 in total). |
| Windows group | A synthetic group (ID 253, title "Windows applications") for windows of Windows programs run through Wine. It is not a catalogue slot and is never persisted. |
| Logical px | Interface units at 100% scale. Physical px = logical × scale / 100, using integer division. The scale is 100, 125 or 150, chosen per Space or inherited from the global setting. |
| Tick | The original timer ran at 100 Hz, so one tick is 10 ms. All durations below are converted to milliseconds. |
| Work area | Full display width × (display height − taskbar height). For decorated windows the *content* work area height is also reduced by one title-bar height. The taskbar is 52 logical px and the title bar 36 logical px. |
| Z-order | A single global stacking list of windows (bottom → top). Only visible windows are drawn or hit-tested. Focusing a window moves it to the top, so z-order doubles as MRU order. |
| Visible window | A live window in the visible Space that is not minimized. |

---

## 1. Shell surface inventory

The taskbar runs along the bottom edge. From left to right it contains:

1. **Launcher button**, which opens the application launcher.
2. **App groups**, in the order defined in §2.1, scrollable when they overflow.
3. **Overflow ("More") button**, shown only when there are more groups than visible slots.
4. **Network indicator**, which opens a small network menu.
5. **Desktop selector button**, labelled "Desktop" and showing the visible Space's name. It opens the Desktop menu.
6. **Clock**, showing HH:MM in UTC. Its tooltip shows the full date.

The shell also has these transient surfaces, all drawn above windows:

- launcher popup
- task menus (chooser, context and overflow)
- Desktop menu
- network menu
- MRU switcher overlay
- desktop-icon context menu
- desktop modal dialogs
- notice toast
- tooltip

At most one of the launcher, task menu, Desktop menu, network menu and desktop context menu is open at a time. Opening one closes the others ("dismiss shell", §7.3).

---

## 2. Taskbar

### 2.1 Which apps appear

Groups are computed for the **visible Space S** every time the taskbar is used:

1. For each built-in app *a* in S's **task order** (a permutation of the five built-in slots), include *a* if any of these holds:
   - *a* is present and **enabled** in S's app catalogue **and** pinned in S; or
   - *a* is **pending** in S (§2.6); or
   - *a* has at least one taskbar-eligible window in S.
2. Then append, in catalogue order, every other catalogue app that is not already listed and is pending or has windows in S. This covers installed packages, which cannot be pinned (§2.3).
3. Then append the **Windows group** if S has at least one Wine window.

Rules that follow from this:

- A pinned app that is disabled in S's catalogue does not appear while idle.
- An unpinned app appears while it has windows or a pending launch, in its task-order position, and disappears when its last window closes.
- A window whose application identity could not be resolved belongs to no group and gets no taskbar entry (inferred).
- Windows created with the client "no taskbar entry" flag (menus, tooltips) never count.
- Windows of other Spaces never count.

### 2.2 Grouping

- Windows are grouped by **application identity**. The compositor resolves this when the window is created, from the authenticated process identity reported by the app manager, never from the window title. If no app identity is available it falls back to the executable name. A process named `wine` maps to the Windows group.
- Each process may own several windows (up to 16 per client). Each window counts separately in its group.
- The windows of a group are always listed in **MRU (z-order, topmost first)** order.

### 2.3 Pinning

- Pins are stored **per Space** as a set of built-in app IDs. A new Space pins all five built-in apps.
- Only built-in apps (IDs 0–4) can be pinned. The context menu omits Pin for installed apps and for the Windows group.
- The context-menu row reads **"Pin to this Desktop"** or **"Unpin from this Desktop"** and toggles the pin for the visible Space only. A save is then scheduled (debounced, see session-restore.md §4).
- Unpinning never changes the task order. Re-pinning restores the app at its previous order position.
- Unpinning a running app keeps its group until the last window closes. Re-pinning never launches a duplicate.
- Settings can also edit pins. A Settings submission carries a per-Space "pins edited" mask. For Spaces whose bit is clear, the compositor keeps its **live** pins, so a stale Settings draft cannot undo a pin change made on the taskbar meanwhile.

### 2.4 Ordering and persistence per Space

- Each Space keeps its own **task order**: a permutation of the five built-in app IDs. The default is Files, Text editor, Terminal, Settings, Browser. Validation rejects any non-permutation.
- **Drag to reorder.** The user presses on a group and moves the pointer more than 6 logical px horizontally or vertically (strictly greater) from the press point. While dragging, the group under the pointer shows an insertion marker. On release over another group, the dragged app is **moved to the target's position** in the full permutation, and the entries in between shift by one toward the vacated slot (see test vectors TV-12 and TV-13). A save is then scheduled.
  - Reordering only applies when both source and target are built-in apps. Installed apps and the Windows group keep their appended order (§2.1, step 2).
  - Releasing over no group does nothing: it is neither a click nor a reorder.
  - If movement stays within the threshold, release over the **same** group is a click (§2.6). Release elsewhere does nothing.
- The order is persisted per Space and survives restarts. Settings never changes task order. When applying a Settings draft, the compositor always keeps its live order for existing Spaces.

### 2.5 Overflow and scrolling

- **Visible slot capacity** = floor((display width − 404·s) / (48·s)), minimum 1, where s = scale/100 and each product is truncated to an integer. Example values are in TV-14.
- The taskbar shows groups `[offset, offset + capacity)`. The **mouse wheel over the taskbar** changes `offset` by the wheel delta, clamped to `[0, groups − capacity]`.
- When groups exceed capacity, a **More** button appears in the slot after the last visible group. Clicking it opens the **overflow menu** (heading "Applications"). That menu lists **every** group in taskbar order. Choosing a row acts exactly like clicking that group (§2.6).
- Keyboard focus on a group outside the visible window scrolls `offset` so the focused group is visible (§8).
- `offset` resets to 0 on Space switch and is re-clamped whenever groups disappear.

### 2.6 Click behaviour

A primary click (press and release on the same group without passing the drag threshold) on group *a* in the visible Space behaves as follows:

| Windows of *a* | Behaviour |
|---|---|
| 0 | Launch *a*'s default instance (§3.5), unless *a* is **pending**, in which case nothing happens. |
| 1 | If that window is the focused window and not minimized, **minimize** it. Otherwise **focus** it, which also restores it if minimized. |
| ≥ 2 | Open the **chooser** menu (§2.7). |

An app is **pending** in S when any of these holds:

- a launch for it is queued or in flight in S;
- it has a launched process that has not yet created its window; or
- one of its windows in S has not yet presented its first frame ("Starting").

The pending guard prevents double-clicks from starting duplicate processes. It only affects the zero-window case: once the launched window exists (status "Starting"), the group has one window and the one-window rule applies.

A **secondary press** on a group opens its **context menu** (§2.7) immediately, on press.

### 2.7 Task menus

All three menus share these mechanics:

- **Anchoring.** The menu opens above the taskbar, anchored to its group button. The overflow menu is anchored to the More button. Menus are kept horizontally on-screen.
- **Rows.** Visible rows = floor((display height − taskbar − 64·s) / (42·s)), minimum 1. Longer lists scroll: the wheel moves the offset, and keyboard selection keeps the selected row visible. A proportional scroll thumb indicates position.
- **Keyboard.** Up and Down move the selection, clamped with no wrap. Enter activates. Escape closes. The selection starts at row 0.
- **Pointer.** Hovering a row selects it. A **press** on a row activates it (activation is on press, not release). A press outside the menu, primary or secondary, closes it and is **consumed**, so it never reaches the window underneath.
- **Header.** The app title, or "Applications" for the overflow menu.
- **Live updates.** When a window dies, rows referring to it are removed and the selection index is clamped.
- **Side effects.** Opening any task menu closes the other shell surfaces and cancels pointer capture.

**Chooser** (primary click on a group with ≥ 2 windows, or Enter/Space on that group in panel focus):

1. One row per window, MRU order.
2. **New window**, omitted for the Windows group.

**Context menu** (secondary press, or Up/F10 in panel focus):

1. **New window**, omitted for the Windows group.
2. **Pin/Unpin from this Desktop**, built-in apps only.
3. One row per window, MRU order.
4. **Close window**, shown if the group has any window. Its detail line shows the title of the **most recently used** window, which is the one it targets.

**Overflow menu:** one row per group (§2.5).

Window rows show the window title, an app icon and a **status text**. The focused window's row is also marked with a check, so state is never shown by colour alone.

| Condition (first match) | Status |
|---|---|
| No frame presented yet | Starting |
| Minimized | Minimized |
| Focused window | Active |
| Otherwise | Open |

Row actions:

| Row | Action |
|---|---|
| Window | Focus that exact window, which restores it if minimized, provided it is still alive in the visible Space. |
| New window | Launch the app's default instance (§3.5). This is ignored while the app is pending; the row is shown as disabled then. |
| Pin/Unpin | Toggle the pin for the visible Space and schedule a save. |
| Close window | Send a **close request** to the MRU window. The application may refuse or prompt about unsaved work (§6.9). |
| Overflow app | Same as clicking that group (§2.6). |

### 2.8 Indicators

These are behavioural states. Their visual form is free.

| State | Meaning |
|---|---|
| Active | The group contains the focused window. It must be distinguishable from "running". |
| Running | The group has ≥ 1 window but not the focused one. |
| Pending | Shown instead of running/active while the app is pending (§2.6). The original deliberately used a **static** indicator, with no animation and no implied progress. |
| Count | Shown when a group has > 1 window: the number of windows. |
| Hover / pressed / menu-open | The group is under the pointer, pressed, or owns the open menu. |
| Drop target | During a drag reorder, the group under the pointer that would receive the drop. |
| Keyboard focus | A focus ring on the focused panel element while the panel has keyboard focus (§8). |

### 2.9 Other panel items

- **Launcher button**. A primary or secondary press opens the launcher. If the launcher is already open, the same press counts as "outside the launcher", so it closes it. The net effect is a toggle.
- **Network indicator**. Opens the network menu, which has two rows:
  - "Network settings…" opens Settings on its network page.
  - "Connect"/"Disconnect" toggles the adapter. It is disabled when no adapter is present.

  Keyboard: Up, Down and Tab move between the two rows. Enter or Space invokes. Escape closes. A press outside closes it. The indicator's tooltip is the textual network status, e.g. "Connected", "Disconnected", "Network access blocked".
- **Desktop selector button**. Opens the Desktop menu (§5).
- **Clock**. Displays UTC HH:MM and repaints on the minute. Its tooltip is the long date "Weekday, D Month YYYY · UTC".

---

## 3. Launcher

### 3.1 Opening and closing

| Action | Effect |
|---|---|
| Ctrl+Space (any other modifiers allowed) | Toggle the launcher. Either way the search text is cleared, the selection is reset to row 0, and the Desktop menu, task menu, context menu, network menu, panel focus and pointer capture are closed or cancelled. |
| Launcher button press | Open (toggle, §2.9). |
| Enter/Space on launcher button in panel focus | Open, and leave panel focus. |
| Escape | Close. |
| Press (primary or secondary) outside the launcher | Close. The press is consumed and never reaches the window below. |
| Launching an item | Close. |
| Space switch | Close. |

### 3.2 Result sources and matching

Results are recomputed from the current query `q`, compared in lowercase:

1. **Applications.** Every catalogue entry of the **visible Space** that is **enabled**, in catalogue order. An entry matches when `q` is empty, or its display name lowercased contains `q` as a substring, or its command string contains `q` (case-sensitive against the already-lowercased query).
2. **Windows programs** (Wine). Start-menu shortcuts found in the visible Space's Wine profile. The scan covers the top-level Start Menu folders and one level of subfolders, and is bounded to 24 programs and 16 folders. Shortcuts with control characters in their arguments, overlong paths, or whose target no longer exists are skipped. Programs are sorted by name case-insensitively and listed after all applications. A program matches when `q` is empty or its name lowercased contains `q`. The scan runs each time the launcher opens and is discarded when it closes.

Matching is plain substring with no prefix ranking or fuzzy matching. When nothing matches, the launcher shows "No matching applications" and Enter does nothing.

### 3.3 Keyboard

While the launcher is open it consumes all key presses:

| Key | Effect |
|---|---|
| Printable ASCII 0x20–0x7E | Append to the query (maximum 80 characters) and reset the selection to 0. |
| Backspace | Remove the last query character and reset the selection to 0. |
| Up / Down | Move the selection, clamped to `[0, results − 1]` with no wrap. |
| Enter | Launch the selected result. |
| Escape | Close. |

Quirk not to replicate: the original appended printable codes even when Ctrl or Alt was held, so Ctrl+Alt+1 typed "1" into the query. Non-ASCII text could not be typed into the query.

### 3.4 Pointer and paging

- The popup shows at most 8 rows, fewer if the screen is short. Visible rows = floor((popup height − 104·s) / (48·s)), minimum 1. Here popup height = min(104 + 48 × clamp(results, 1, 8), display height − taskbar − 24·s), in logical units.
- Scrolling is **page-wise**. The first visible row is floor(selection / visible) × visible.
- Hovering a row selects it. A press on a row launches it.

### 3.5 Launch semantics ("default instance")

- **Application** result:
  - If the app is pending in the visible Space, the launch is ignored.
  - Otherwise a **new process/window is always started**, even if the app already has windows. The launcher never just focuses an existing window.
  - The argument depends on the app: Files and Terminal receive the visible Space's **folder**; all other apps receive no argument.
  - The launch targets the **visible Space**.
- **Windows program** result. Starts the Wine runtime in the Space the scan came from, with the target `.exe` and the shortcut's arguments. Failure shows the notice "The Windows runtime could not be started."
- **Launch pipeline.** Launches go into a FIFO queue and are submitted one at a time. If a launch cannot be submitted within 30 s, it is dropped with a notice. All errors are reported through notices (§9, and session-restore.md §9).

---

## 4. MRU window switcher

### 4.1 Opening

- Chords: **Alt+Tab** or **Ctrl+Tab** opens the switcher stepping forward; adding **Shift** steps in reverse. It opens on key press, and only when no desktop modal dialog is open.
- Opening closes all other shell surfaces, cancels pointer capture and leaves panel focus.
- The **held modifier** is recorded as Alt if Alt is down, otherwise Ctrl. Only that modifier keeps the switcher alive.
- **Candidate list**: every live window of the **visible Space**, in z-order from topmost down, **including minimized windows**. The list is a snapshot taken at opening, held as stable window identities. If the list is empty, the switcher does not open.

### 4.2 Initial selection

- If the focused window is in the list, selection starts at it and immediately takes one step. Forward selects the next window (the previously used one). Reverse selects the last (least recently used) window.
- If no window is focused, for example when the desktop has focus or everything is minimized, selection starts at index 0 (the topmost window) without stepping.

### 4.3 While open

Nothing is focused, raised or restored while the user traverses the list.

| Input | Effect |
|---|---|
| Tab with recorded modifier held | Step forward with wraparound. Shift+Tab steps backward. |
| **Escape** or **Backspace** press | **Cancel.** Close the switcher and change nothing. Backspace exists because the original host frontend intercepted Escape-based chords; it is shown as the on-screen hint. |
| Any key event whose modifier state no longer includes the recorded modifier (including key releases and modifier-only changes) | **Commit**: focus the selected window, which un-minimizes it and raises it to the top. If it no longer exists or is no longer in the visible Space, the current focus is left unchanged. |
| Any other key | Consumed. |
| Primary or secondary mouse press | Cancel. |
| Other pointer input | Consumed. |
| A candidate window dies | It is removed from the list. Selection stays on the same window if it survives; otherwise the index is clamped. If the list becomes empty, the switcher closes. |

The overlay lists each candidate's title, app icon and a status text, "Minimized" or "Open". The selected row is kept in view: the offset equals selected − (visible − 1), floored at 0.

### 4.4 Scope and quirks

- Only the visible Space is considered. Other Spaces' windows are never candidates.
- MRU order is z-order (§6.3). Minimizing does not move a window in z-order, but focus moves to the next window, which is raised above it. A just-minimized window therefore becomes the second MRU entry (TV-5).
- The original included windows flagged "no taskbar" or "never activate" (client popups) in the list (inferred unintended behaviour; see Open questions).

---

## 5. Desktop (Space) selector and switching

### 5.1 Desktop menu

Opening it:

- **With the pointer** (a press on the selector button), the selection starts on the visible Space and the list scrolls so that row is visible.
- **From the keyboard** (Enter/Space in panel focus), the selection starts on the visible Space and the previous scroll position is kept.

Rows, in order:

1. Every Space, in configured order. Each row shows the name. The visible Space has a check mark. Project Spaces are distinguishable.
2. **Project tasks…** launches Terminal with the argument `tasks`.
3. **Network settings…** launches Settings with the argument `network`.
4. **Desktop settings…** launches Settings with the argument `desktops`.

Mechanics:

- Visible Space rows = floor((display height − taskbar − 172·s) / (36·s)), clamped to `[1, number of Spaces]`. The wheel scrolls the Space rows. The three footer rows are always visible.
- **Keyboard.** Up and Down move over all rows (Spaces then footer), clamped with no wrap. Enter switches to the selected Space or runs the selected footer action. Escape closes. The Space list scrolls to keep a selected Space visible.
- **Pointer.** A press on a Space row switches to it. A press on a footer row runs its action. A press outside closes the menu and is consumed.
- Choosing an action or a Space closes the menu.

### 5.2 Other ways to switch

| Trigger | Result |
|---|---|
| Ctrl+Alt+1 … Ctrl+Alt+9 | Switch to the 1st…9th Space in configured order. A missing index only closes menus. These shortcuts are inert while the launcher, a menu, panel focus or a dialog is active (§7.2). Spaces 10–32 are reachable only through the menu. |
| Trusted "open Space" request (from Settings/Files with the manager role) | Switch to the Space with the given ID, then open a **new** Files window at `/PROJECT` in it. |

### 5.3 Switching rules

Switching to index *i* performs these steps in order:

1. Cancel pointer capture, any drag and the MRU switcher. Close the network menu, Desktop menu and task menus. Reset taskbar scroll and panel row. Hide the tooltip.
2. If *i* is not a valid index, stop here.
3. Make *i* the visible Space. Refresh that Space's app catalogue and network status immediately. Close the launcher and desktop context menu. Reload the desktop icons from that Space's folder.
4. **Focus** the topmost *visible* (non-minimized) window of the new Space in z-order, or nothing if it has none. The previously focused window receives a focus-lost notification.
5. Schedule a save, because the visible Space is persisted.

Switching to the already-visible Space still runs every step, so it re-focuses the topmost visible window.

### 5.4 What switches with the Space

| Switches | Does not switch |
|---|---|
| Visible windows, taskbar groups, MRU switcher scope, hit testing | Running processes. They keep running, keep their memory and unsaved work, and **never move** to another Space. |
| Pins and task order | A window's Space, which is fixed at creation from its process's Space. |
| App catalogue (enabled apps), launcher results, Windows-program list | Pending launches. They complete into their own target Space even if it is no longer visible. |
| Wallpaper (Space override or global), accent, interface scale, reduced-effects preference (each overrides or inherits the global value) | Global preferences. Switching never changes them. |
| Desktop icons (the Space's folder) and network status | |
| Target of new launches (always the visible Space) | |
| Clipboard partition used by the shell's own dialogs (clipboards are per Space) | Each application's clipboard, which is always its own Space's |

Windows of hidden Spaces receive no input and are excluded from the taskbar and switcher. Apart from focus loss, they get no explicit "hidden" notification (inferred).

### 5.5 Space set limits

These rules are enforced by Settings and the compositor:

- 1 to 32 Spaces. The defaults are **Personal**, **Project** (folder `/HOME/PROJECT`, project flag set) and **Focus**.
- Names are 1–127 bytes of UTF-8. Folders are absolute paths.
- The last Space cannot be removed.
- Removing a Space that still has live windows, pending launches or other live processes is **refused as busy**.
- Removing a Space never deletes its storage; it only drops that Space's saved session.
- If the visible Space is removed, the first Space becomes visible. If a lower-index Space is removed, the active index shifts down.

---

## 6. Window management

### 6.1 Window kinds

| Kind | Behaviour |
|---|---|
| Decorated (default) | The compositor draws a title bar of 36 logical px with title, app icon and three controls, and owns move, resize, maximize and minimize. |
| Client-decorated | Used by Windows programs. The client draws its whole frame and positions itself through geometry requests. The compositor clamps the size to `[1, display]`, x to `[−w+1, display width − 1]` and y to `[−16, display height − 1]`. The −16 allows a maximized Windows frame's sizing border to sit just off-screen. There are no compositor title-bar controls or resize edges. Closing the window removes it immediately. |
| No-taskbar | Excluded from taskbar groups. |
| No-activate | Never receives focus, whether from a click or on creation. |

### 6.2 Focus rules

1. At most one window has keyboard focus. It is always a visible window of the visible Space (inferred invariant).
2. **Click to focus.** A primary or secondary press anywhere on a visible window, frame or content, focuses it unless it is no-activate. The same press is **also delivered** to the window when it lands in the client area. Presses on the title bar or a resize edge are handled by the compositor and not delivered.
3. A press on the empty desktop removes window focus. Keyboard input then drives the desktop icons (§11).
4. Focusing a window raises it to the top of the z-order, un-minimizes it, sends focus-lost to the previously focused window and focus-gained to the new one, and cancels pointer capture held by another window.
5. When the focused window is minimized or closed, focus moves to the topmost visible window of the visible Space, or to nothing.
6. Space switch: see §5.3, step 4.
7. **New windows.** A window is focused on creation only if all of these hold:
   - it is not no-activate;
   - it belongs to the visible Space;
   - it is not minimized; and
   - its launch intent says "new window" or "was the active window", **or** no window is currently focused.

   An ordinary launch counts as "new window". A restored window carries its saved active flag; see session-restore.md §8 for the full policy table.
8. Shell surfaces (launcher, menus, panel focus) take keyboard input without changing which window is focused. Only their actions change focus.
9. Pointer motion and wheel events go to the topmost visible window under the pointer **even if it is not focused**. Hover does not change focus.

### 6.3 Stacking

- Every focus operation raises the window to the top.
- A window created **without** activation in the **visible** Space is inserted **directly below the focused window**, so it never covers the window that has keyboard focus. Created in a hidden Space, it is placed on top of the global z-order, which only matters later.
- Minimized windows keep their z position but are not drawn.
- Z-order is not persisted (see session-restore.md §2.4).

### 6.4 Title-bar controls

The controls are hit zones of 38 logical px each, measured from the right edge of a decorated window's title bar. They activate on **press**.

| Zone (from right) | Action | Tooltip |
|---|---|---|
| 0–38 | Close request | "Close" |
| 38–76 | Maximize, or Restore when maximized. Ignored in fixed-layout Spaces. | "Maximize" / "Restore" |
| 76–114 | Minimize | "Minimize" |
| Remainder | Move or double-click handling | — |

- **Double-click** on the title bar toggles maximize/restore. It needs a second press on the same window less than 400 ms after the first, within 4 logical px in both axes. After a double-click is recognised, the click history is reset, so a third press starts fresh. Ignored in fixed-layout Spaces.
- A single title-bar press on a **non-maximized** window starts a move drag. On a maximized window it does nothing; the user must restore first.
- A secondary press on the title bar only focuses the window.

### 6.5 Move

- The window follows the pointer, keeping the grab offset.
- The position is clamped every frame so the **whole window, including the title bar, stays inside the work area**: x ∈ `[0, display width − w]` and y ∈ `[0, content work-area height − content h]`. Windows can never be dragged partly off-screen or under the taskbar.
- On button release the client is told its new geometry and a save is scheduled.

### 6.6 Resize

- **Resize edges** are a 5-logical-px band just inside each side of a decorated window's outer frame, title bar included. Edges combine at corners into diagonal resizes.
- Resize is disabled on maximized windows, client-decorated windows and in fixed-layout Spaces.
- The **opposite edge stays anchored**. Along each axis:
  - **Minimum**: the app minimum × scale, capped to the space available from the anchored edge to the work-area boundary. The minimum never forces the window off-screen.
  - **Maximum**: the distance from the anchored edge to the work-area boundary.
  - Vertical limits apply to the outer height (content + title bar); the outer bottom may not pass the top of the taskbar.
- App minimum content sizes, in logical px:

  | App | Minimum (w × h) |
  |---|---|
  | Files | 600 × 400 |
  | Text editor | 640 × 320 |
  | Terminal | 480 × 300 |
  | All others | 600 × 360 |

- The frame updates live. The **client is told the new size only on release** (inferred from the original: no intermediate resize messages). A save is scheduled on release.
- The size fit is `fit(requested, minimum, available) = clamp(requested, min(max(minimum,1), max(available,1)), max(available,1))`. The position fit is `clamp(requested, 0, max(available − max(size,1), 0))`. These never produce a zero or inverted size (TV-17).

### 6.7 Maximize and restore

- **Maximize** stores the current rectangle as the restore rectangle and sets the window to x = 0, y = 0, the full work-area width and the full content work-area height.
- **Restore** re-fits the stored rectangle to the **current** work area, applying minimum and available limits to the size and clamping the position.
- A maximized window always fills the work area. When the Space's scale changes, maximized windows are re-filled. Other windows are first scaled by new/old scale and then re-fitted.

### 6.8 Minimize and restore

- **Minimize** comes from the title-bar control, or from a primary click on a single-window group whose window is focused. It hides the window, cancels capture, moves focus per §6.2 rule 5 and schedules a save.
- A window is **restored** from minimized by any focus operation:
  - a taskbar click on a single-window group;
  - a chooser or context window row;
  - an MRU switcher commit; or
  - a Space switch, which does **not** restore minimized windows, because it focuses only non-minimized ones.
- Minimized windows remain in their group with status "Minimized", count toward the group's window count, and remain switcher candidates.

### 6.9 Close

- Close is a **request** sent to the application. It comes from the title-bar control, the context-menu "Close window" row, or Ctrl+Alt+Q (focused window only).
- The application may prompt about unsaved work and may cancel. The window disappears only when the client destroys it or its process exits.
- When a window disappears:
  - its group count drops, and the group vanishes if it is unpinned and now empty;
  - menu rows and switcher candidates for it are removed;
  - focus moves per §6.2 rule 5; and
  - a save is scheduled.
- Process exit is detected within about 100 ms.

### 6.10 Pointer capture and input routing

- A primary press in a window's client area **captures** the pointer to that window. Until all buttons are released, all pointer events go to it in window-local coordinates, even outside its bounds.
- Capture is cancelled when focus changes to another window, when any shell menu, launcher or switcher opens, and on Space switch. Cancelling sends the captured window a synthetic "all buttons released" event at the current position.
- Keyboard events go to the focused window only when it is visible and no shell surface or reserved shortcut consumed them (§7.2).
- Pure modifier-change events are never forwarded to applications.
- Pointer speed is a global percentage, 50–200%, applied to relative motion with sub-pixel remainder accumulation. Absolute pointer devices are used 1:1.

### 6.11 Initial placement of new windows

- **Size.** The client's requested size is fitted per axis to `[app minimum × scale, available − 16 px]`, where available is the window's Space's work area at that Space's scale.
- **Cascade position**, using the window's slot index *n*:
  - x = fit(130 + (38·n mod max(work width − 480, 1)))
  - y = fit(68 + (34·n mod 180)), with the available height reduced by 8 px

  Each value then goes through the position fit (§6.6). See TV-18.
- Restored windows use their saved geometry instead (session-restore.md §7).

### 6.12 Fixed-layout Spaces

A Space may be marked **fixed**. In it, move, resize, maximize/restore and title double-click are disabled. Focus, minimize, close and application input still work.

### 6.13 Limits

| Item | Limit |
|---|---|
| Live windows + launched-but-not-created + queued launches | 64. Beyond that: notice "Window resources are exhausted. Close an application and try again." |
| Queued launches | 32 |
| Windows per client process | 16 |
| Window title | 127 bytes |
| Spaces | 32 |
| Desktop icons listed | 64 entries of the Space folder |
| Minimum supported display | 1024 × 720. The maximum surface was 1920 × 1080. |

---

## 7. Keyboard shortcuts

### 7.1 Reserved shell shortcuts

These shortcuts are never delivered to applications.

| Shortcut | Action |
|---|---|
| Ctrl+Space | Toggle launcher |
| Alt+Tab / Alt+Shift+Tab | MRU switcher forward / reverse; commit on Alt release |
| Ctrl+Tab / Ctrl+Shift+Tab | Same, held modifier Ctrl. This was the primary chord in the original because the host captured Alt+Tab. |
| Tab / Shift+Tab (switcher open) | Step forward / back |
| Escape or Backspace (switcher open) | Cancel switcher |
| Ctrl+F6 | Toggle taskbar keyboard focus (§8) |
| Ctrl+Alt+1…9 | Switch to Space 1…9 |
| Ctrl+Alt+Q | Request close of the focused window |
| Escape | Close the open launcher, task menu, Desktop menu or desktop context menu |
| Ctrl+Shift+F12 | Toggle input diagnostics tracing (developer only; optional in the port) |

Plain F6, Tab and similar keys pass to applications, which use them for in-app control traversal.

### 7.2 Precedence

The original processed each key event through this ladder. The first matching stage consumes the event. Stages 7–17 apply to key presses only; key releases skip directly to stage 18.

1. **Network menu open**, and the event has no Ctrl or Alt: Escape closes; Up, Down or Tab toggles the row; Enter or Space invokes. Everything else is swallowed.
2. **Ctrl+Shift+F12**: diagnostics toggle (edge-triggered).
3. **Switcher open**: §4.3.
4. **Modifier-only change event**: dropped.
5. **Alt/Ctrl+Tab**, if no dialog is open: open the switcher.
6. **Ctrl+F6**, if no dialog is open: toggle panel focus. This also closes shell surfaces, cancels capture and resets the panel row to the launcher button.
7. **Desktop dialog open**: the dialog consumes all keys (§11).
8. **Escape** with the launcher, desktop context menu, Desktop menu or task menu open: close all of them.
9. **Ctrl+Space**: toggle the launcher.
10. **Desktop context menu open**: Up and Down select (4 rows, clamped); Enter activates.
11. **Desktop menu open**: §5.1.
12. **Task menu open**: §2.7.
13. **Panel focus**: §8.
14. **Launcher open**: §3.3.
15. **Ctrl+Alt+1…9**: switch Space.
16. **Ctrl+Alt+Q**: close request to the focused window.
17. **No focused window**: Enter opens the selected desktop item; arrows and Delete navigate icons; every other key press is swallowed.
18. Otherwise: deliver to the focused window, if visible.

---

## 8. Panel keyboard navigation

- **Ctrl+F6** enters panel focus, with the launcher button focused, and exits it again. Panel focus also ends on Escape, on any pointer press, on activating the launcher button, and whenever a window gains focus.
- **Focus stops**, with N = number of groups (all groups, including scrolled-out ones):

  | Stop | Element |
  |---|---|
  | 0 | Launcher button |
  | 1…N | App groups, in taskbar order |
  | N+1 | Network indicator |
  | N+2 | Desktop selector |

  The More button is not a stop; scrolling makes every group reachable.

| Key | Effect |
|---|---|
| Right or Tab | Next stop, wrapping from N+2 to 0. Shift+Tab also moves forward in the original (see Open questions). |
| Left | Previous stop, wrapping from 0 to N+2. |
| Enter or Space | Activate the stop. Launcher button: open the launcher and leave panel focus. Group: the primary-click behaviour (§2.6). Network: open the network menu. Desktop selector: open the Desktop menu. |
| Up or F10, on a group | Open that group's **context** menu. |
| Escape | Leave panel focus. |
| Anything else | Swallowed. Applications receive no keys while the panel has focus. |

- Menus opened from panel focus take the keys while open. Closing them with Escape returns to panel focus on the same stop (inferred from the original keeping panel focus set).
- After each move, the taskbar scrolls so the focused group is visible.
- If groups disappear, the focused stop index is clamped to the new range.

---

## 9. Notices (toast notifications)

- There is a **single notice slot**. A new notice **replaces** the current one immediately. There is no queue, history, action button or dismissal interaction.
- A notice is visible for **4 s** after it is posted, centred just above the taskbar, and is then removed.
- The original truncated notice text to 128 bytes (quirk; the port should not truncate meaningfully).
- Notices are informational only and never take focus.

Notices raised by the shell:

| Condition | Text |
|---|---|
| Message backlog to a stalled client exhausted | Application message backlog exhausted. Close the stalled application. |
| Wine runtime failed to start | The Windows runtime could not be started. |
| Wallpaper unreadable (once per failure streak) | Wallpaper could not be read. Choose an available BMP in Settings. |
| Session save would exceed capacity | Saved session capacity is unavailable. Close applications or turn off session restore for a Desktop; previous saved references are retained. |
| Launch into a Space without resolved storage identity | Desktop storage is not ready. Open recovery diagnostics. |
| Window limit reached | Window resources are exhausted. Close an application and try again. |
| Launch bookkeeping out of memory / queue full | Not enough memory to launch an application. |
| App not in the Space catalogue | Application is unavailable. Open Applications in Settings. |
| Launch not submitted within 30 s | Application launch timed out before submission. Try again. |
| Launch result lost (transport error) | Launch outcome is unknown. Check running applications before retrying. |
| Other launch, save or filesystem error | The error's standard message |
| Surviving client's Space no longer configured | A surviving application's Desktop is unavailable. Its process and unsaved work are retained. |
| No memory for window or surface | Not enough memory for another window. / Not enough memory for another window surface. |
| Window ID space exhausted | Window identity capacity exhausted. |
| Application reported allocation failure | "<App title> could not allocate memory. Close unused windows and retry." |
| Desktop dialog succeeded | Guest filesystem updated |
| Configuration unreadable at startup, storage listed instead | Desktop settings need recovery. Existing storage is available; original preferences remain unchanged. |
| Booted from volatile RAM storage | Hardware diagnostic session · RAM storage · Changes are lost at power-off. |

An unauthorised power request receives a direct reply instead of a toast: "Use Settings or the bundled Terminal for session power controls."

---

## 10. Tooltips

- A tooltip appears after the pointer has been **still for more than 600 ms** with **no buttons held**.
- Any pointer motion or button change hides it and restarts the timer. So do focus changes, window close and dismissing shell surfaces.
- It is suppressed while the launcher, any menu or the switcher is open.
- Placement: above the pointer, clamped on-screen.

Content by pointer location:

| Location | Text |
|---|---|
| Launcher button | Applications · Ctrl+Space |
| Network indicator | Network status label |
| Desktop selector | Switch Desktop |
| Clock | Weekday, D Month YYYY · UTC |
| App group | Application title, e.g. "Windows applications" for the Windows group |
| Title-bar controls | Close / Maximize or Restore / Minimize |
| Desktop icon (no window focused) | Full file name, never truncated |

---

## 11. Desktop surface (icons, context menu, dialogs)

This section is brief because it is mostly file-manager behaviour.

- **Icons.** The visible Space's folder is shown as icons. Index 0 is always "Home", the folder itself. Icons fill column by column; rows per column = floor((display height − taskbar − 40·s) / (100·s)).
- **Keyboard**, when no window is focused:
  - Up and Down move by one item.
  - Left and Right move by one column (± rows).
  - Movement is clamped to `[0, entry count]`.
  - Enter opens the selection.
  - Delete asks for confirmation, for real entries only.
- **Pointer.**
  - A press selects. A second press on the same item within 500 ms opens it.
  - A press on empty space clears both selection and window focus.
  - A secondary press opens the **context menu**: Open (Enter), Open Desktop folder, New folder…, Delete… (Del). Delete is disabled for Home or when nothing is selected. The menu is clamped on-screen above the taskbar.
- **Opening.**
  - A folder, or Home, or nothing selected, opens in a new Files window.
  - A file opens in a new Text editor window.
  - "Open Desktop folder" opens Files at the Space folder.
- **Dialogs.**
  - New folder: a text field pre-filled with "New folder", all text selected.
  - Delete confirmation: "This permanently deletes the selected item. Non-empty folders are kept."

  Both are modal and consume all keyboard input:
  - Enter confirms and Escape cancels.
  - Ctrl+C, Ctrl+X and Ctrl+V use the visible Space's clipboard.
  - **Ctrl+Shift+U**, then up to 6 hex digits, then Enter inserts that Unicode code point. Escape abandons the code and Backspace edits it.
  - Pointer presses on the confirm or cancel button act. Other pointer input goes to the text field.
  - On success the notice "Guest filesystem updated" appears and the icons refresh. On failure the dialog stays open and the error appears as a notice.

---

## 12. Accessibility-relevant behaviour

- **Full keyboard operability.** Every shell function is reachable without a pointer: launcher (Ctrl+Space), window switching (Alt/Ctrl+Tab), taskbar including menus, pin, new window and close (Ctrl+F6, arrows, Enter/Space, Up/F10), Desktop switching (menu or Ctrl+Alt+digit), window close (Ctrl+Alt+Q) and desktop icons.
- **Visible keyboard focus.** A distinct focus indicator marks the focused panel stop and the selected rows in all menus, the launcher and the switcher. Keyboard focus is distinct from hover and from selection.
- **State never by colour alone.**
  - Chooser rows carry text status (Starting, Minimized, Active, Open) and a check mark on the active row.
  - The visible Space has a check mark.
  - Window counts are numeric badges.
  - The pending state is a distinct indicator.
- **Contrast targets** carried over from the original design contract: 4.5:1 for text and 3:1 for essential non-text state.
- **No motion.** The original shell had no animations, and the pending indicator is deliberately static. A per-Space or global **reduced effects** preference removes translucency and shadows. The port must honour it for any animation it adds.
- **Interface scale.** 100, 125 or 150%, per Space or inherited. All hit targets, thresholds (drag 6 px, double-click distance 4 px, resize band 5 px) and limits scale with it.
- **Textual names for icon-only controls**, via tooltips (§10). The full date is available as text.
- **Cancel alternatives.** Backspace as well as Escape cancels the switcher.
- **Unicode entry** in shell dialogs through Ctrl+Shift+U code-point composition.
- **Gap.** The original exposed no screen-reader or accessibility-tree semantics. The port should add them (see Open questions).

---

## 13. Test vectors

Unless stated otherwise:

- the display is 1024 × 768 at 100% scale;
- one Space is visible;
- A, B, C, … are windows of distinct apps;
- z-order is written bottom → top; and
- "MRU" lists topmost first.

**TV-1. MRU after creation and one switch.**
A, B and C are opened in that order, each focused on creation, so z = [A, B, C] and C is focused.
Ctrl+Tab, then release Ctrl.
→ B focused, z = [A, C, B], MRU = [B, C, A].

**TV-2. Two steps.**
Continue from TV-1 (MRU [B, C, A]).
Ctrl+Tab, Tab, release.
→ The switcher starts at B, steps to C, then to A. A is focused. MRU = [A, B, C].

**TV-3. Reverse.**
z = [A, B, C], C focused, so MRU = [C, B, A].
Ctrl+Shift+Tab, release.
→ Selection starts at C and steps backward with wrap to A. A is focused. MRU = [A, C, B].

**TV-4. Cancel.**
MRU = [C, B, A].
Ctrl+Tab, Tab (A is now selected), Backspace, then release Ctrl.
→ Nothing changes: C stays focused and z is unchanged. The release after a cancel does nothing.

**TV-5. Minimized windows in MRU.**
z = [A, B, C], C focused.
Minimize C.
→ Focus goes to B, z = [A, C, B], C hidden.
Ctrl+Tab.
→ The candidate list is [B, C, A] and C is selected, shown as "Minimized".
Release.
→ C is un-minimized and focused, z = [A, B, C].

**TV-6. Wraparound returns to start.**
Three windows, C focused.
Ctrl+Tab, Tab, Tab (three steps in total), release.
→ The selection returns to C. C stays focused and z is unchanged.

**TV-7. Nothing focused.**
A is the only window and is minimized, so focus is none.
Ctrl+Tab.
→ The list is [A] and selection is index 0 with no initial step.
Release.
→ A is restored and focused.

**TV-8. Per-Space scope.**
Space 1 has A and B. Space 2 has C. Space 1 is visible.
Ctrl+Tab.
→ Candidates are [B, A] only. C is never offered.

**TV-9. Candidate dies during traversal.**
The list is [C, B, A] with B selected, and B's process exits.
→ The list becomes [C, A] and the selection clamps to index 1 (A).
If all candidates die, the switcher closes without changing focus.

**TV-10. Taskbar click with 0, 1 and many windows.**
Terminal pinned, with no windows.
- Click → Terminal launches. Clicking again before the launched process has created its window (pending, 0 windows) does nothing.
- Once its window exists (even before the first frame), Terminal has 1 window and is focused. Click → it is minimized.
- Click again → it is restored and focused.
- Open a second Terminal through context menu → New window. Click → the chooser opens with rows [T2 (Active), T1 (Open), New window].

**TV-11. Group visibility.**
Pins = {Files, Editor}. Settings is unpinned and has 1 window. Browser is pinned but disabled in the Space catalogue. An installed app "Notes" has 1 window.
Task order = default.
→ Groups shown: [Files, Editor, Settings, Notes].
Closing the Settings window removes its group.
Unpinning Files while it has no windows removes its group.

**TV-12. Reorder forward.**
Order [Files, Editor, Terminal, Settings, Browser].
Drag Files onto Terminal.
→ [Editor, Terminal, Files, Settings, Browser].

**TV-13. Reorder backward and pin independence.**
Order [Files, Editor, Terminal, Settings, Browser].
Drag Settings onto Editor.
→ [Files, Settings, Editor, Terminal, Browser].
Then unpin Settings and re-pin it.
→ The order is unchanged.

**TV-14. Overflow capacity.**

| Display | Scale | Capacity |
|---|---|---|
| 1024 | 100% | 12 |
| 1024 | 125% | 8 |
| 1024 | 150% | 5 |
| 1280 | 100% | 18 |
| 1920 | 100% | 31 |

With 7 groups at 1024/150%:
- slots show groups 0–4 and the More button appears in slot 5;
- the wheel offset range is [0, 2]; and
- the overflow menu lists all 7.

**TV-15. Drag threshold.**
Press on Editor and move 6 logical px right (exactly the threshold), then release over Editor.
→ This is a click.
Move 7 px instead.
→ This is a drag. Releasing over Editor itself means "move onto self", so the order is unchanged and there is no click.

**TV-16. Panel keyboard wrap.**
With 4 groups the stops are 0–6.
- Ctrl+F6 → stop 0.
- Left → stop 6 (Desktop selector).
- Right → stop 0.
- Right ×5 → stop 5 (network).
- Enter → the network menu opens.
- From stop 2, Up (or F10) → the context menu of group 2 opens.

**TV-17. Size and position fitting.**

| Expression | Result |
|---|---|
| fit_size(700, 600, 572) | 572 |
| fit_size(100, 900, 1008) | 900 |
| fit_size(100, 600, 620) | 600 |
| fit_size(900, 600, 620) | 620 |
| fit_size(MAX, MAX, 0) | 1 |
| fit_size(MIN, −1, −1) | 1 |
| fit_position(MIN, 100, 50) | 0 |
| fit_position(MAX, 100, 50) | 50 |
| fit_position(20, 100, MAX) | 0 |

**TV-18. Cascade placement.**
Editor requests 800 × 600 at 100%. The available area is 1024 × 680.
- Size → 800 × 600.
- Slot 0 → (130, 68).
- Slot 3 → x = clamp(244, 0, 224) = 224 and y = clamp(170, 0, 672 − 600) = 72, so (224, 72).

**TV-19. Move clamp.**
An 800 × 600 (content) window, with an available area of 1024 × 680.
- Dragging so that x = −100 gives x = 0.
- Dragging so that y = 500 gives y = 680 − 600 = 80.

**TV-20. Anchored resize.**
A Files window at x = 200, y = 100, content 700 × 450 (outer height 486) at 100%.
- Drag the left edge 200 px right → width = fit(500, 600, 900) = 600, left = 300. The minimum holds.
- Drag the left edge 300 px left → width = fit(1000, 600, 900) = 900, left = 0. It is clamped to the screen.
- Drag the bottom edge 200 px down → outer height = fit(686, 436, 616) = 616, so content height = 580 and the bottom sits at the taskbar top (716).

**TV-21. Title double-click timing.**
Two title presses 390 ms apart and 3 px apart → maximize.
Two presses 400 ms apart → no maximize; the second press starts a move.
In a fixed Space → neither.

**TV-22. Maximize and restore re-fit.**
A window at (300, 200, 700 × 450) is maximized, giving (0, 0, 1024 × 680).
The Space scale then changes to 150%.
→ The work area is recomputed: (0, 0, 1024 × 636). The window still fills it.
Restore.
→ The stored rectangle is re-fitted to the current work area.

**TV-23. Space switch focus.**
Space 1 has z = [A, B] with B minimized. Space 2 has [D, E].
- Switch to Space 2 → E focused.
- Switch back → A focused, because minimized B is skipped. B stays minimized.

**TV-24. Launcher search and paging.**
- q = "TER" (typed) matches "Terminal" (lowercased name).
- q = "edit" matches "Text editor" (name) and any app whose command contains "edit".
- With 10 results at 1024 × 768/100%: 8 rows are visible; selecting row 8 shows rows 8–9.
- At 1280 × 720/150%: 6 rows are visible.

**TV-25. Tooltip and notice timing.**
- Pointer still for 600 ms → no tooltip. At 610 ms → tooltip.
- A notice posted at t = 0 is visible until t = 4 s.
- A second notice at t = 1 s replaces it and is visible until t = 5 s.

---

## 14. Open questions

1. **Switcher candidates.** The original included no-taskbar and no-activate windows (client popups) as MRU candidates. Should the port exclude them? Recommended: yes.
2. **Ctrl+Tab reservation.** Ctrl+Tab was reserved because the original host frontend captured Alt+Tab. On Linux this would steal a common in-app shortcut (browser and editor tabs). Should the port keep only Alt+Tab, or make the chord configurable?
3. **Backspace cancel.** Should Backspace remain a switcher-cancel alias on Linux?
4. **Shift+Tab in panel focus.** It moved forward like Tab. Should it move backward?
5. **Pins and order for installed apps.** Pins and order covered only the five built-in apps, because the original used a fixed bitmask and permutation. Should the port generalise them to arbitrary app IDs, so installed apps can be pinned and reordered?
6. **Activation on press.** Menu rows and title-bar controls activated on press. The original in-app menu toolkit used press-and-release on the same item. Which should the port adopt for shell menus?
7. **Resize feedback.** The client learned its new size only at the end of a resize drag. Should the port send live configure events?
8. **Task-menu window rows.** These targeted windows by process; for multi-window clients this picked the process's first window in the visible Space. The port should target the exact window.
9. **Unresolved app identity.** A window whose identity cannot be resolved has no taskbar group but is still a switcher candidate. Should it get a generic group?
10. **Launcher text.** It accepted only printable ASCII and accepted keys with Ctrl/Alt held. Should the port support full Unicode search and ignore modified keys?
11. **Clock time zone.** The clock is UTC only, with no local time zone. Is that intended for the port?
12. **Notices.** There is one slot, a fixed 4 s lifetime, no history, and text truncated at 128 bytes. Should the port add a queue or history, or keep the single toast?
13. **Escape returning to panel focus.** After a menu opened from panel focus is dismissed, focus returns to the panel (inferred). Confirm this is intended.
14. **Key releases during shell surfaces.** Key releases reached the focused application even while the launcher or a menu had consumed the matching press (inferred quirk). The port should keep press and release pairs consistent.
15. **Accessibility tree.** There was no screen-reader support. Which accessibility interface should the port target (for example AT-SPI through iced or accesskit)?
16. **Ctrl+Alt+digit.** Only Spaces 1–9 are reachable this way. Should there be next/previous Space shortcuts?
