# Session restore specification

This document specifies how the original AmityOS desktop saved and restored the user's session: which window, application, document, geometry, focus and ordering state was recorded per Space (Desktop); when it was written; how it was replayed at startup and after a compositor restart; how failures and invalid data were handled; and how window identity was kept stable. It was derived from the original AmityOS implementation, chiefly the compositor `userspace/src/bin/desktop.rs` and the configuration codec `userspace/src/config.rs`, plus `session_policy.rs`, `window_geometry.rs`, `applications.rs` and the client side of `gui.rs`. `docs/spaces-session-progress.md` was consulted only where it agrees with the code. It is the behavioural reference for the Linux port (Smithay compositor plus iced shell). The port may use any storage format, but it must preserve the behaviour below unless an open question is resolved otherwise. Items marked "(inferred)" were deduced from code structure rather than stated by code comments or docs.

Interactive shell behaviour (taskbar, switcher, window management) is specified in [shell-behaviour.md](shell-behaviour.md).

---

## 1. Terms and identities

| Term | Definition |
|---|---|
| **SpaceId** | A 64-bit storage identity assigned by the storage service. It is unique among Spaces. **0 means unresolved** (a legacy or draft entry) and never grants authority. A Space's list position is UI order only; every binding uses the SpaceId. |
| **AppId** | A 64-bit immutable application identity issued by the application manager and authenticated per process. 1–5 are the built-in apps: Files = 1, Text editor = 2, Terminal = 3, Settings = 4, Browser = 5, i.e. catalogue slot + 1. Values ≥ 0x1000 are installed packages. Values 6–0xFFF are invalid. 0 means "no identity" and appears only in legacy records. |
| **WindowId** | A 64-bit nonzero session-stable window identity, allocated by the compositor from a monotonic counter `next_window_id` (§3.3). |
| **Catalogue slot** | A runtime index of an app in a Space's catalogue. It is never part of persisted identity and is re-derived from the AppId at every restore. |
| **Logical geometry** | Window rectangle in logical px (100% scale), measured as content area. The title bar is excluded; x and y are the outer top-left corner within the work area. Physical = logical × Space scale / 100. |
| **Retained reference** | A saved window record read at startup that has not yet been matched by a created window (§5, §6). |

---

## 2. What is saved

### 2.1 Global state

| Item | Values |
|---|---|
| Accent | 3 choices |
| `grid` flag | On or off. Stored and carried through migrations, but no current code reads it (inferred: an obsolete preference). |
| **Visible Space** | Stored by SpaceId. If that SpaceId is unknown at load, the first Space is used. |
| Interface scale | 100, 125 or 150 |
| Reduced effects | On or off |
| Pointer speed | 50–200 % |
| Global wallpaper | Absolute path, plus Fill or Fit mode |
| `next_window_id` | Counter, see §3.3 |

### 2.2 Per Space

| Item | Notes |
|---|---|
| SpaceId | Unique and nonzero before any save |
| Name | 1–127 bytes UTF-8 |
| Folder | Absolute path. Used for desktop icons and as the default argument for Files and Terminal. |
| **Pins** | Set of pinned built-in apps. Default: all pinned. |
| **Task order** | Permutation of the five built-in apps. Default: Files, Text editor, Terminal, Settings, Browser. |
| Fixed-layout flag | Disables move/resize/maximize |
| Project flag | Presentation only |
| Wallpaper override and mode | Empty means inherit the global value |
| **Restore-session flag** | Default **on**. When off, the Space's windows are neither saved nor relaunched. |
| Appearance overrides | Accent: inherit or one of 3. Scale: inherit, 100, 125 or 150. Reduced effects: inherit, off or on. |

### 2.3 Per window record

At most **64** records in total, across all Spaces.

| Field | Content |
|---|---|
| AppId | Identity of the owning application |
| SpaceId | The window's Space |
| WindowId | Stable window identity |
| Flags | bit 0 **minimized**; bit 1 **maximized**; bit 2 **active** (the globally focused window at save time). No other bits are allowed. |
| x, y, w, h | Logical geometry. For a maximized window this is the **pre-maximize (restore) rectangle**, not the full-screen one. `w ≤ 0` means "no saved geometry" (§7). |
| Argument | ≤ 1023 bytes UTF-8 with no NUL: the application's last reported state, otherwise its launch argument |

What the argument means depends on the application:

| App | Argument |
|---|---|
| Files | Current folder path, updated on navigation |
| Text editor | Current document path. Empty when untitled or after closing the document. |
| Terminal | Current working directory |
| Browser | Current URL |
| Settings | Its launch argument, such as `network`, `desktops` or `applications <package path>`. Settings reports no state of its own (inferred from the source). |
| Installed packages | Whatever state string they report |

Applications report state as a single short message, or as ordered fragments of at most 208 bytes each. A fragmented state must total at most 1023 bytes, arrive in order and be valid UTF-8 without NUL. Any out-of-order, oversize or invalid sequence is discarded without changing the stored argument. A valid state update schedules a save.

### 2.4 Not saved

- **Unsaved document contents or any other process memory.** Restoration reopens documents from disk. A cold restart cannot preserve unsaved work; only a warm compositor restart does (§10).
- **Stacking (z-order)** and MRU order, apart from the single *active* flag. There is also no per-Space "last focused window" for hidden Spaces. Records are written in compositor window-slot order, which is not the stacking order (inferred; see Open questions).
- **Windows in Spaces whose restore-session flag is off.**
- **Windows without an AppId.** This includes Windows (Wine) program windows, which are started directly rather than through the application manager (inferred).
- Clipboard contents, which are volatile and cleared on compositor restart.
- Launcher, menu, switcher and taskbar-scroll state.
- Queued *new* launches that have not yet obtained a process. Pending restores are kept as retained references instead (§5).

---

## 3. Identity rules

### 3.1 Space identity

- A window's Space is the **authenticated Space of its owning process**, fixed at process creation. It is never taken from a client request, window title or executable name, and it never changes when the visible Space changes.
- Saving is **refused while any Space has SpaceId 0**. At startup, legacy Spaces without an id are bound to storage by list index and name. If binding fails, a notice is shown and persistence waits.
- Renaming or reordering Spaces never affects window bindings, because everything is keyed by SpaceId.
- Removing a Space from the configuration drops that Space's saved records. Live processes in it make the removal fail as busy (shell-behaviour.md §5.5).

### 3.2 Application identity

- The AppId comes from the application manager's authenticated process identity. The catalogue slot is looked up from the AppId in the **target Space's** catalogue at restore time.
- Legacy records (AppId 0) take AppId = legacy slot + 1 on migration.

### 3.3 WindowId allocation

When a window is created, the compositor chooses its WindowId:

1. A **requested** id, from the saved record being restored or from a reattaching client, is accepted if all of these hold:
   - it is nonzero;
   - it is **below `next_window_id + 64`**; and
   - **no live window** currently uses it.
2. Otherwise the window gets `next_window_id`.
3. Then `next_window_id = max(next_window_id, id + 1)`. If that would overflow, window creation fails with the notice "Window identity capacity exhausted."

On save, any record without an id is assigned sequential ids starting at `max(highest record id + 1, next_window_id)`. The stored counter is that start value plus the number of ids assigned. The counter is therefore monotonic and ids are not reused within one configuration lineage.

When the configuration is loaded, any record with WindowId 0, AppId 0, or WindowId ≥ the stored counter makes the **whole file invalid** (§11). Duplicate nonzero WindowIds are also invalid.

### 3.4 Matching

A saved record and a window **match** when **WindowId, AppId and SpaceId are all equal**. Matching is used to:

- drop a retained reference once its window has been created (§5); and
- recover the saved argument for a client that reattaches after a compositor restart (§10).

Titles, process IDs and slot numbers are never used for matching.

### 3.5 Configuration privacy (inferred relevance)

Ordinary applications that read desktop configuration receive a **projection** containing only their own Space and only that Space's records. The projection has effective (resolved) appearance values and is re-indexed to a single entry. Such clients can never write it back over the global configuration. Only manager roles (desktop, app manager, storage manager) read and write the full file.

---

## 4. When the session is saved

| Trigger | Timing |
|---|---|
| Window created | Debounced |
| Geometry committed: end of move or resize, maximize/restore, creation-time fitting, restore-time layout | Debounced |
| Minimize | Debounced |
| Window closed or owning process exited | Debounced |
| Application reports new state (document, folder, cwd, URL) | Debounced |
| Pin toggled or taskbar reordered | Debounced |
| Visible Space switched | Debounced |
| Settings applies a configuration | **Immediate**. The save result is returned to Settings. |
| Shutdown or power-off request | **Immediate and synchronous**, with storage flushed. If the save fails, power-off is **aborted** and the error is shown as a notice. |
| Startup migration needed (old format or newly bound SpaceIds) | Once at startup after binding, and again just after the first frame if still pending |

Debouncing and retry rules:

- **Debounce**: the save runs **750 ms after the last triggering change**. Each new change restarts the timer.
- **Deferral**: while a launch is in flight (submitted and awaiting its process ID), debounced saves wait. This keeps the record set consistent with launch bookkeeping.
- **Busy storage**: retried after 100 ms.
- **Other write errors**: shown as a notice. The previous file stays intact and there is no automatic retry until the next change (inferred).

These are **not** save triggers in the original, which is a quirk: focus changes, and restoring a minimized window by focusing it. Both are captured only by the next save that happens for another reason, or at shutdown. See Open questions.

A save rewrites the whole configuration atomically from the compositor's point of view and then synchronises storage. In recovery (read-only) mode every save is refused (§11).

---

## 5. Building the saved record set

Each save rebuilds the record list from scratch:

1. **Live windows**, in compositor slot order. A window is included if it has an AppId and its Space has restore-session on. Each record holds:
   - its AppId, SpaceId and WindowId;
   - flags: minimized; maximized; active if it is the currently focused window;
   - geometry converted to logical units as truncate(physical × 100 / Space scale). The restore rectangle is used if the window is maximized;
   - its current argument.
2. **Launched but not yet created windows**: a known process ID with no window yet, an AppId, and restore-session on. The launch state is stored as is. That state is either the saved record being restored, or a new-launch state with w = h = 0 and the launch argument.
3. **Retained references**: saved records from startup that have not been matched (§3.4) by a created window. A retained reference is merged back if all of these hold:
   - its WindowId is nonzero;
   - its Space still exists and has restore-session on; and
   - its WindowId is not already in the list.

   This keeps windows that failed to relaunch (missing app, launch error, out of memory, process died before creating its window) recoverable on a later boot.
4. If the total would exceed **64** records, the save is **refused**. The previous file is left untouched and this notice is shown: "Saved session capacity is unavailable. Close applications or turn off session restore for a Desktop; previous saved references are retained."
5. Otherwise the configuration is validated (§11) and written.

---

## 6. Restore at startup (cold start)

### 6.1 Sequence

1. **Load** the configuration (§11). In recovery mode, the Space list is replaced by the storage service's Space list, every Space gets folder `/PROJECT`, and the first Space is visible. No window records exist, so nothing is restored.
2. **Bind** any SpaceId-0 Spaces to storage, then run the migration save if needed (§4).
3. Load the visible Space's application catalogue and its desktop icons.
4. For each saved record, **in saved order**:
   1. Resolve its SpaceId. A legacy record takes it from its Space index.
   2. If no configured Space has that id, **skip** the record. It is not retained.
   3. If that Space has restore-session **off**, **skip** it. It is dropped at the next save.
   4. Add it to the **retained references**.
   5. If **any live process of the same AppId already exists in that Space**, do not relaunch. A surviving process is expected to reattach (§10). This check is per application, not per window; see Open questions.
   6. Otherwise **queue a restore launch** (§6.2) with the record's argument.
5. Enter normal operation. Restore launches go through the same FIFO launch queue as user launches. They are submitted **one at a time**: the next is submitted only after the previous one has returned a process ID or failed. Windows may still be created in any order, because processes start asynchronously.

### 6.2 Restore launch

1. Capacity checks. If live windows, launched-but-uncreated processes and queued launches already total 64, or the queue already holds 32 launches, or memory is exhausted: show a notice and abandon this attempt. The reference stays retained.
2. If the target Space has SpaceId 0, show "Desktop storage is not ready. Open recovery diagnostics." and abandon.
3. Refresh the **target Space's** catalogue and look up the AppId. If it is not found, the app was uninstalled or is unavailable in that Space. The attempt is **skipped silently**, apart from a diagnostic trace, and the reference stays retained.
4. Queue the job with the record as its launch state, targeting the record's Space. That Space may be hidden.
5. Submit the job to the application manager with the saved argument. The job must be submitted within **30 s**; otherwise it is dropped with a notice. Busy responses are retried continuously within that window.
6. When the reply returns a process ID:
   - If the process has **already created its window**, apply the restore immediately (step 7).
   - Otherwise hold the state as *pending* until the window is created.
7. **Apply the restore to the window**:
   - The argument is set from the record, unless the application has already reported its own state, in which case that state wins.
   - Geometry and flags come from §7.
   - The saved WindowId is adopted per §3.3.
   - The matching retained reference is dropped.
   - The client receives its geometry.
   - Focus and stacking follow §8.

Each saved record gets **one launch attempt per compositor start**. There is no retry loop.

---

## 7. Geometry restoration and clamping

Inputs:

- the record's (x, y, w, h) in logical units, plus its flags;
- the scale *s* of the record's Space **at restore time**;
- the current display size W × H.

Algorithm:

1. If **w ≤ 0**, the record has no saved geometry. The window keeps the default cascade placement (shell-behaviour.md §6.11) and counts as a **new window** for focus (§8).
2. Convert to physical units: physical(v) = truncate(v × s / 100), saturated to the 32-bit signed range.
3. Compute the available content area: width W_a = W, and height H_a = H − truncate((36 + 52) × s / 100). These are the title bar and the taskbar.
4. Size each axis: size = fit(requested, minimum, available), as defined in shell-behaviour.md §6.6.
   - width = fit(max(physical(w), 1), app_min_w × s / 100, W_a − 16)
   - height = fit(max(physical(h), 1), app_min_h × s / 100, H_a − 16)

   The minimum never exceeds the available space, so the result is always ≥ 1 and fits on screen. App minimum sizes are in shell-behaviour.md §6.6.
5. Position: x = clamp(physical(x), 0, max(W_a − width, 0)) and y = clamp(physical(y), 0, max(H_a − height, 0)). The window is always fully on-screen.
6. Set minimized from flag bit 0 and maximized from flag bit 1.
7. The **restore rectangle** is the fitted (x, y, width, height).
8. If maximized, the window becomes (0, 0, W_a, H_a).
9. Client-decorated windows (Windows programs) are not laid out by the compositor. They position themselves.

Consequences:

- Saving in logical units means a record restores proportionally under a different scale.
- A record saved on a larger display is clamped into a smaller one, and malformed values (huge, negative, zero) never produce an inverted or off-screen window.
- Clamped values are written back at the next save.

The same clamping applies when a Space's scale changes at runtime. Live geometry is multiplied by new/old scale and then re-fitted, and maximized windows are refilled to the new work area.

---

## 8. Focus, stacking and minimized state on restore

A newly created window (restored or not) is **activated** (focused and raised) if and only if all of these hold:

- it is not a no-activate window;
- its Space is the **visible** Space;
- it is **not minimized**; and
- one of these holds:
  - its intent is unknown;
  - its intent is "new window";
  - its intent is "was active"; or
  - **no window is currently focused**.

Intent sources, in priority order:

1. **The window's own launch state**, i.e. the restore record or pending launch. "New window" means w ≤ 0; "was active" means flag bit 2 is set.
2. If the window is created **before its launch reply arrives**, the **first in-flight launch** whose AppId **and** SpaceId both match the window supplies its intent. This is a presentation hint only. Arguments, WindowId and geometry still wait for the authenticated reply.
3. Otherwise the intent is unknown, and an unknown intent activates.

Stacking and follow-up rules:

- A window created **without activation in the visible Space** is inserted **directly below the focused window**, so an inactive restored window never covers the focused one. Any other window is placed on top of the global stacking order.
- If a window was focused at creation but its launch reply then marks it **minimized**, focus moves to the topmost visible window.
- Restored minimized windows stay minimized and are never focused.
- Windows restored into **hidden Spaces** never take focus. When the user later switches to such a Space, its topmost visible window gets focus. Stacking is not persisted, so that is effectively the latest-created window (inferred).

Net effect of a cold start into the visible Space:

- If the saved active window is restored, it ends focused and on top, regardless of the order in which windows appear.
- If it cannot be restored, the first non-minimized window to appear in the visible Space gets focus.

---

## 9. Failure handling

| Situation | Behaviour | Saved record |
|---|---|---|
| Record's SpaceId unknown when the file is loaded | Record silently dropped at load; the rest of the file is accepted | Lost |
| Space has restore-session off | Not relaunched | Dropped at next save |
| Space has unresolved id (0) | Not relaunched; saving refused until resolved | Unchanged on disk |
| AppId not in target Space's catalogue (uninstalled or unavailable) | Silently skipped (diagnostic trace) | **Retained** |
| Catalogue entry for a slot missing at launch | Notice "Application is unavailable. Open Applications in Settings." | Retained |
| Live process of that AppId already in that Space | Not relaunched; awaits reattachment | Retained until matched |
| Window limit (64), queue limit (32) or launch memory exhausted | Notice | Retained |
| Not submitted within 30 s | Notice "Application launch timed out before submission. Try again." | Retained |
| Launch rejected | Notice with the error's message | Retained |
| Reply lost or transport failure (outcome unknown) | Notice "Launch outcome is unknown. Check running applications before retrying."; never retried automatically | Retained |
| Process exits before creating its window | Pending state discarded | Retained |
| Window created | Retained reference removed | Replaced by live record |
| Saved geometry invalid, obsolete or larger than display | Clamped (§7) | Rewritten clamped at next save |
| Saved scale differs from current Space scale | Logical units rescaled (§7) | Rewritten at next save |
| Retained + live records > 64 | Save refused with capacity notice | Previous file intact |
| Configuration unreadable or invalid | Recovery mode (§11) | Original file intact; nothing saved |
| Surviving client reattaches but its Space is no longer configured | Notice "A surviving application's Desktop is unavailable. Its process and unsaved work are retained."; process kept alive without a window | — |
| New (non-reattach) window from a process whose Space is not configured | Window refused; close request sent to the client | — |

---

## 10. Warm compositor restart (reattachment)

The original compositor could be replaced while applications kept running:

- **Placement is pushed to clients.** Whenever a window's logical placement or flags change, the compositor sends the client its WindowId, logical rectangle and flags (minimized, maximized, active). Updates are coalesced to the latest value per window. For a maximized window the rectangle is the restore rectangle.
- **Clients re-offer their windows.** After the compositor is replaced, each surviving client re-creates its window with a **reattach marker**, its WindowId, its last placement, its title and its last reported state.
- **The new compositor:**
  - does not relaunch saved records whose app already has a live process in that Space (§6.1, step 4.5);
  - treats the offered placement as a restore record and lays it out with the same clamping (§7);
  - takes the **argument** from the saved record matching (WindowId, AppId, SpaceId), if one exists;
  - honours the WindowId per §3.3; and
  - drops the matching retained reference.
- **Live state is preserved.** Process memory, including unsaved documents, survives because the process never stopped.
- Windows in restore-disabled Spaces also reattach, because they are live, but they are still not saved.
- Clients built for the older protocol cannot reattach and must be reopened.

For the Linux port, Wayland clients normally do not survive a compositor restart; see Open questions.

---

## 11. Configuration loading, validation and recovery mode

**Loading.**

| Situation | Result |
|---|---|
| File missing (manager role) | Defaults, writable |
| Read error, bad length, bad checksum, unknown format or failed validation | **Temporary defaults in read-only recovery mode**, with a notice. The **original file is kept untouched** and every save is refused until the file is repaired. |
| Ordinary (non-manager) process | Receives its Space projection only (§3.5), always read-only |

**Validation.** Any failure rejects the whole file.

*Global:*

- 1–32 Spaces;
- ≤ 64 records;
- visible index within range;
- accent 0–2;
- scale ∈ {100, 125, 150};
- pointer speed 50–200;
- wallpaper mode Fill or Fit;
- global wallpaper an absolute path longer than `/`;
- counter ≠ 0.

*Per Space:*

- SpaceId unique when nonzero;
- name non-empty UTF-8;
- folder absolute;
- pins only built-in apps;
- task order a valid permutation;
- wallpaper override empty or absolute;
- appearance values in range (inherit allowed).

*Per record:*

- AppId valid (0 only with a legacy slot below 5; never 6–0xFFF);
- flags only bits 0–2;
- WindowId unique when nonzero;
- argument valid UTF-8.

*Current format additionally:*

- AppId ≠ 0;
- WindowId ≠ 0 and WindowId < stored counter;
- reserved bytes zero;
- boolean bytes 0 or 1.

The file is bounded to 160 KiB and carries a length and a CRC-32.

---

## 12. Limits

| Item | Limit |
|---|---|
| Saved window records (all Spaces) | 64 |
| Spaces | 32 (≥ 1) |
| Live windows + uncreated launches + queued launches | 64 |
| Queued launches | 32 |
| Requested WindowId acceptance window | `next_window_id + 64` |
| Argument / state string | 1023 bytes UTF-8, no NUL; fragments ≤ 208 bytes |
| Space name | 127 bytes; folder and wallpaper paths 1023 bytes |
| Window title (not saved) | 127 bytes |
| Configuration file | 160 KiB |
| Save debounce / busy retry | 750 ms / 100 ms |
| Launch submission deadline | 30 s |
| Launch attempts per saved record | 1 per compositor start |

---

## 13. Related behaviour: autostart and legacy migration

- **Autostart** is not part of the saved session. Each second, any app in the **visible** Space's catalogue that is flagged both *autostart* and *enabled* is launched once per (Space, app) per compositor lifetime. The launch only happens if that app has no window, no pending launch and no live process in the Space. The deduplication list is bounded to 1024 entries.
- **Legacy formats** (six earlier versions) were readable. Older versions stored physical geometry, which was converted to logical by dividing by the global scale on migration. Their AppIds became slot + 1, WindowIds were assigned 1…n in record order, and SpaceIds were bound from storage by index and name. Migration was written on the first successful save. Whether the port must import any original configuration is an open question.

---

## 14. Test vectors

The display is 1024 × 768 unless stated otherwise. Scale is the record's Space scale. Geometry is (x, y, w, h).

**SR-1. Save conversion with truncation.**
A live Files window at 150% has physical geometry (16, 16, 1008, 620) and is not maximized.
→ The saved logical record is (10, 10, 672, 413).

**SR-2. Round-trip drift.**
SR-1's record is restored at 150% on the same display.
- Physical values: (15, 15, 1008, 619).
- Width = fit(1008, 900, 1008) = 1008. Height = fit(619, 600, 620) = 619.
- x = clamp(15, 0, 16) = 15. y = clamp(15, 0, 17) = 15.

→ The restored geometry is (15, 15, 1008, 619). It drifted by 1 px because both conversions truncate. See Open questions.

**SR-3. Clamping to a scaled work area.**
A Files record (80, 70, 680, 460) is restored at 150%.
- Available area: 1024 × 636.
- Physical values: (120, 105, 1020, 690).
- Width = fit(1020, 900, 1008) = 1008. Height = fit(690, 600, 620) = 620.
- x = clamp(120, 0, 16) = 16. y = clamp(105, 0, 16) = 16.

→ The restored geometry is (16, 16, 1008, 620).
The same record at 100% is restored unchanged as (80, 70, 680, 460).

**SR-4. Smaller display.**
A Text editor record (1500, 900, 800, 500) was saved on 1920 × 1080 at 100% and is restored on 1024 × 768 at 100%.
- Available area: 1024 × 680.
- Width 800 and height 500 are kept.
- x = clamp(1500, 0, 224) = 224. y = clamp(900, 0, 180) = 180.

→ The restored geometry is (224, 180, 800, 500).

**SR-5. Maximized record.**
A Files record (80, 70, 680, 460) with flags = maximized is restored at 100%.
- The restore rectangle is (80, 70, 680, 460).
- The window is (0, 0, 1024, 680).

The next save writes (80, 70, 680, 460) with the maximized flag, plus the active flag if the window is focused.

**SR-6. No saved geometry.**
A record has w = 0, as written for a launch that had not yet created its window.
→ The window uses cascade placement and counts as a new window, so it is activated if it is in the visible Space and not minimized.

**SR-7. WindowId allocation.**
`next_window_id` = 12 and live windows use ids {5, 7}. Each row starts from that state.

| Requested | Assigned | Counter after |
|---|---|---|
| 9 | 9 | 12 |
| 3 (free, below counter) | 3 | 12 |
| 7 (in use) | 12 | 13 |
| 0 | 12 | 13 |
| 75 (< 12 + 64) | 75 | 76 |
| 76 (≥ 12 + 64) | 12 | 13 |

**SR-8. Id assignment on save.**
Records have WindowIds [3, 0, 7, 0] and the counter is 5.
- Assignment starts at max(7 + 1, 5) = 8.
- The stored ids become [3, 8, 7, 9].
- The stored counter becomes 10.

**SR-9. Merging retained references.**
- Space P (id 1) has restore on. Space Q (id 3) has restore off. Space 9 no longer exists.
- Live records: WindowIds 1 and 2, both in P.
- Retained references: id 2 (P, duplicate), id 3 (P), id 4 (Space 9), id 5 (Q) and id 0 (P).

→ The saved WindowIds are [1, 2, 3].

**SR-10. Capacity refusal.**
There are 60 live records and 5 non-duplicate retained references, 65 in total.
→ The save is refused, the capacity notice is shown and the previous file is left intact.
After the user closes 1 window (59 + 5 = 64), the next save succeeds.

**SR-11. Startup with mixed failures.**
The visible Space is Personal. Saved records, in order:

| Record | App | Space | Notes |
|---|---|---|---|
| R1 | Editor | Personal | active flag set |
| R2 | Files | Personal | |
| R3 | Terminal | Project | Project has restore off |
| R4 | Browser | id 9 | Space no longer exists |
| R5 | package 0x1001 | Personal | package not installed |

→ R4 is dropped at load. R3 is skipped and disappears at the next save. R1 and R2 are launched in that order, one submission at a time. R5 is skipped silently and kept as a retained reference, so the next save writes R1, R2 and R5.

**SR-12. Cold-start focus independent of arrival order.**
Continuing SR-11, suppose Files (R2) creates its window first.
- Nothing is focused yet, so R2 is activated.
- Then Editor (R1, "was active") is created and activated on top.

→ Editor is focused and Files sits below it.

In the opposite arrival order, Editor is activated first. Files is then inactive, nothing-focused does not apply, and Files is inserted below Editor. The final state is the same.

If R1 were missing, Files would end up focused.

**SR-13. Focus policy table.**
Columns: the window's AppId and SpaceId, the visible SpaceId, whether the window is minimized, whether nothing is focused, its saved intent, and the in-flight launch intent written as (AppId, SpaceId, new, active).

| AppId | SpaceId | Visible | Minimized | Nothing focused | Saved intent | In-flight intent | Activate? |
|---|---|---|---|---|---|---|---|
| 4 | 1 | 1 | no | no | none | (4, 1, new = no, active = no) | **no** |
| 2 | 1 | 1 | no | no | none | (2, 1, new = no, active = yes) | yes |
| 4 | 1 | 1 | no | no | none | (4, 1, new = yes, active = no) | yes |
| 4 | 1 | 1 | no | no | none | (2, 1, new = no, active = no), unrelated app | yes (intent unknown) |
| 4 | 1 | 1 | no | no | none | (4, 2, …), other Space | yes (intent unknown) |
| 4 | 1 | 1 | no | no | (new = no, active = yes) | (4, 1, new = no, active = no) | yes (saved intent wins) |
| 4 | 2 | 1 | no | no | none | (4, 2, new = yes, active = yes) | **no** (hidden Space) |
| 4 | 1 | 1 | yes | yes | none | (4, 1, new = yes, active = yes) | **no** (minimized) |

**SR-14. Legacy geometry migration.**
A legacy record was stored at a global scale of 150% with physical (−301, 90, 900, 600).
→ The migrated logical record is (−200, 60, 600, 400), truncating toward zero. It is later clamped on restore per §7.

**SR-15. Invalid file means recovery mode.**
Any one of these makes the file invalid:
- a record with AppId 6;
- two records sharing WindowId 4;
- a record whose WindowId is ≥ the stored counter.

→ Defaults are loaded read-only with a notice, nothing is restored, and the original file is left untouched. Every save is refused.

---

## 15. Open questions

1. **Persist stacking and MRU order.** The original saved records in slot order and kept only one global *active* flag. Should the port persist z-order per Space and a per-Space last-focused window, so hidden Spaces restore their own focus?
2. **Save on focus changes.** Focus changes and un-minimize-by-focus did not schedule a save. Should they? Recommended: yes, debounced.
3. **Per-application deduplication.** Relaunch suppression checked "any live process of this AppId in this Space", not the specific WindowId. With two saved Editor windows and one surviving process, the second record is not relaunched. Should matching be per WindowId?
4. **Truncation drift.** Logical/physical conversion truncates, so a window can drift up to 1 px per save/restore cycle at 125% or 150% (SR-2). Should the port round, or store the physical rectangle with its scale?
5. **Restore inset.** Restore applies a 16 px inset to the maximum size, but un-maximize applies none. Is that difference intended?
6. **Wine windows.** Windows (Wine) program windows have no AppId and are never restored. Should the port restore them, for example by saving the `.exe` path and arguments?
7. **Disabled apps.** Restore looks up AppIds in the catalogue without checking the *enabled* flag (inferred). Should disabled apps be restored?
8. **Retained references never expire.** They are bounded only by the 64-record capacity. Should there be an expiry, or a UI to discard stale saved windows?
9. **WindowId conflict.** If a saved WindowId is taken by a live window at restore, the new window gets a fresh id and the old retained reference also survives, which could duplicate the window on the next boot (inferred). Should the reference be consumed by AppId and SpaceId in that case?
10. **Save error retry.** After a non-busy save error there is no automatic retry until the next change. Should the port retry with backoff?
11. **Warm reattachment on Linux.** Is compositor-restart survival a requirement for the Smithay port? If not, which parts of §10 are dropped?
12. **Importing original configuration.** Must the port import the original configuration file (Spaces, pins, order, records), or start fresh?
13. **Multiple outputs.** Geometry was single-display. How should records map onto multiple outputs or output hot-plug?
14. **Sequential restore.** Restore launches were strictly sequential. Can the port launch in parallel while preserving the focus and stacking outcome in §8?
