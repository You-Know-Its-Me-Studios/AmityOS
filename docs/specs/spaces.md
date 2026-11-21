# Spaces: behavioural specification

This document describes how "Spaces" behaved in the original AmityOS implementation: the no_std Rust storage service, the compositor and the Settings app that ran on the custom AmityOS kernel. It was written by reading that code before the code was deleted, and it is the reference for re-implementing Spaces on the Linux base. The planned Linux design is a btrfs subvolume per Space with qgroups, bind mounts plus Landlock, and a systemd user slice per Space. Where this document and the old design docs disagree, it follows the code and lists the disagreement in section 11. Anything not stated directly by the code is marked "(inferred)". Section 12 holds porting guidance. That guidance is not normative and is not derived from the code.

Sources read: `userspace/src/managed_storage.rs`, `userspace/src/storage_service.rs`, `userspace/src/apps/space_environment.rs`, `filesystem/src/network_policy.rs`, `userspace/src/desktop_clipboard.rs`, `userspace/src/bin/settings.rs`, plus the supporting files `filesystem/src/storage.rs`, `filesystem/src/names.rs`, `filesystem/src/portable.rs`, `filesystem/src/fat.rs` (allocation budget only), `userspace/src/fs.rs`, `userspace/src/config.rs`, `userspace/src/project.rs`, `userspace/src/task_service.rs`, `userspace/src/network_service.rs`, `userspace/src/local_service.rs`, `userspace/src/bin/desktop.rs` (clipboard, settings apply, legacy binding), `userspace/src/bin/storage_probe.rs` (acceptance expectations), `kernel/src/process/mod.rs` (Space stamping), `abi/src/service.rs` and `abi/src/application.rs`. Design docs: `docs/space-storage-contract.md`, `docs/managed-storage.md`, `docs/space-storage-ui.md`.

## 0. Terminology and error codes

| Term | Meaning |
|---|---|
| Space | The durable storage and authority domain. It has a numeric id, a name, a quota and a private storage tree. Every process belongs to exactly one Space. |
| Desktop | The user-facing name for a Space in the shell and Settings. A Desktop is a configuration entry that points at a Space id. |
| Workspace | The internal name of a Desktop configuration record in `/CONFIG/DESKTOP.CFG`. |
| Retained storage | A Space that exists in the storage catalog but has no Desktop entry. |
| Share (grant) | A directory from one Space (the source) made visible in another Space (the target) at `/SHARED/<alias>`. |
| Owner | The Space that a stored object is charged to. Owner 0 means "no Space": a system or shared object. |
| Manager / admin | A process whose kernel context has the STORAGE_MANAGER or the DESKTOP flag. |
| Packaged application | A process with a non-zero AppId and a permission set, started through the application manager. Unpackaged and system processes have AppId 0. |

Error codes that callers see. The names are internal; the strings are what `fs::error` displays to users.

| Code | Kind | User-visible text |
|---|---|---|
| -1 | Invalid | Invalid path or argument |
| -2 | NotFound | File or folder not found |
| -3 | Access | Permission denied |
| -4 | (out of memory) | Not enough memory |
| -5 | EBUSY (UI/compositor only) | Resource is busy |
| -6 | NoSpace | Disk is full |
| -8 | Exists | Name already exists |
| -9 | NotEmpty | Folder is not empty |
| -10 | TooLarge | Buffer is too small |
| -11 | Busy (storage service maps Busy to EAGAIN) | Try again |
| -13 | Timeout | Filesystem request timed out; result unknown |
| -14 | Quota | Desktop storage limit reached; free space or increase its limit. Previous saved file retained |
| -15 | OutcomeUnknown | Save outcome unknown after storage interruption. Keep this buffer and reopen/check the saved file before retrying |
| -16 | ReadOnly | Volume is read-only; inspect recovery status before saving |
| -17 | Corrupt | Storage metadata needs recovery; writes are disabled |
| -18 | (cross-volume) | Cross-volume move is unsupported; copy then verify before deleting |
| -19 | Shared | Folder is shared; remove its shares before moving it to another Desktop |
| -46 | Conflict (task service) | (environment UI text; see 6.1) |

## 1. Concepts and identities

### 1.1 Space record

The catalog stores these fields for each Space:

| Field | Type | Meaning |
|---|---|---|
| id | u64 | Durable, volume-scoped identity. Never 0. |
| name | UTF-8, 1–127 bytes | Display label. It is not an identity and does not have to be unique. |
| quota | u64 bytes | Hard limit. 0 means no per-Space limit. |
| legacy | u64 | Legacy Desktop ordinal (0–31), or u64::MAX for a normal (non-legacy) Space. |

The catalog also stores `next_space` and `next_file` counters, the share grants (section 3), and a FileId/owner record for every object on the volume. The catalog is persisted in `/AMITY/META.BIN` and is committed in the same journal transaction as the file data it describes. Every successful mutation reconciles and rewrites the catalog before commit. A failed mutation rolls back both the files and the in-memory catalog.

### 1.2 Identity rules

- Space ids come from `next_space`, which starts at 1 and only increases. A failed creation is rolled back, so it does not consume an id. Ids are never reused.
- Id 0 is reserved for system processes and services. Space 0 cannot hold a clipboard, open local sockets or offer handles. It is treated as "no Space".
- Id 1 is "Personal". It is created automatically when the catalog is first initialised, with legacy ordinal 0 and quota 0.
- **Spaces can never be deleted.** The protocol has operations to list, show volumes, bind legacy, create, update (name and quota), show usage, share, unshare, list shares and show identity. There is no delete operation (see section 5).
- Each Space has a private physical root at `/SPACES/S` followed by the id as 16 zero-padded uppercase hex digits, for example `/SPACES/S0000000000000002`. Two subdirectories, `PROJECT` and `STATE`, are created with it. The physical root path is always 25 bytes long.
- Each object has a FileIdentity of (volume UUID, FileId, owner Space). FileIds come from a monotonic counter and are never reused. They are preserved across overwrite, append, truncate, rename and move. Copy creates new FileIds. Delete followed by recreate produces a new FileId.

### 1.3 Name rules

| Rule | Storage service (authoritative) | Settings UI (extra checks) |
|---|---|---|
| Length | 1–127 UTF-8 bytes. The wire field is 128 bytes and must be NUL-terminated, with zeros after the terminator. | Same limit, checked after trimming. |
| Characters | Rejects any character below U+0020. | Rejects every character for which `char::is_control` is true, which adds U+007F and U+0080–U+009F. |
| Whitespace | Stored exactly as given. | Create trims leading and trailing whitespace before sending. Save validates the trimmed value but sends the untrimmed value (see section 10). |
| Uniqueness | Not required. | Not required. |
| Error | -1 | "Enter a name of 1–127 UTF-8 bytes without control characters." (create) / "Use a name up to 127 UTF-8 bytes and a valid guest folder" (save) |

### 1.4 Legacy and default Spaces

- Legacy Spaces exist so that Desktop configurations written before Spaces existed (formats C01–C04, which had no ids) keep their files. BIND_LEGACY(ordinal, name) is the idempotent mapping from a configuration slot index (an ordinal, 0–31) to a Space:
  - If a Space with that legacy ordinal already exists, the service renames it to `name` and returns its id.
  - Otherwise the service creates a new Space with that legacy ordinal and quota 0.
  - An ordinal of 32 or more returns -1. When the catalog already holds 32 Spaces, the call fails with -1 even if the ordinal is already bound (see section 10).
- When the compositor starts, it binds every Desktop entry whose `space_id` is 0 by its slot index, then saves the configuration with the resolved ids. If that save fails, the next boot repeats the same lookup, so no duplicate Spaces are created. An entry with id 0 never carries authority.
- On a fresh install, the default configuration has three Desktops: "Personal" (folder `/HOME`), "Project" (folder `/HOME/PROJECT`, Project toggle on) and "Focus" (folder `/HOME`). All three have id 0, so the compositor binds them as legacy ordinals 0, 1 and 2, giving Spaces 1, 2 and 3. **On a fresh install all three default Desktops are therefore legacy Spaces and share one `/HOME`.** Only Desktops created later in Settings are non-legacy (inferred from `Config::defaults` together with the compositor's binding loop).
- A legacy Space differs from a normal Space in one way: `/HOME` resolves to the shared physical `/HOME` (owner 0) instead of the Space's private PROJECT directory. Legacy Spaces still get private `/PROJECT` and `/STATE` directories. In usage reports, flag bit 1 (LEGACY) is set.
- No two Spaces can share a legacy ordinal. Loading a catalog that contains such a duplicate is treated as corruption.
- Recovery fallback: if the catalog cannot be read, or is missing on a volume that cannot be initialised (read-only media or invalid FAT mirrors), the service exposes a synthetic read-only catalog. That catalog contains one Space, id 1 "Personal (recovery)", legacy 0, and the volume is read-only. If the compositor's own configuration cannot be read, it builds temporary Desktops from the storage catalog, each with folder `/PROJECT`, and shows "Desktop settings need recovery. Existing storage is available; original preferences remain unchanged."

### 1.5 Limits

| Limit | Value | Error when exceeded |
|---|---|---|
| Spaces in the catalog (legacy, attached and retained together; lifetime because nothing is deleted) | 32 | -1 on create or bind |
| Desktop entries in the configuration | 32 | Settings: "The Desktop limit has been reached."; config: -6 |
| Share grants (whole volume) | 128 | -10 |
| Share alias length | 63 UTF-8 bytes (64-byte NUL-terminated field) | -1 |
| Source path for a share | 1023 bytes | -1 |
| Catalog objects (whole volume, including system files) | 8192 | -10 |
| Catalog size | 4 MiB | -10 |
| Single file | 16 MiB | -10 |
| Directory handles | 16 per process, 256 in total | -6 |
| Pending handle-transfer grants | 8 per sender, 64 in total | -6 |
| Clipboard | 1 MiB total across all Spaces, at most 32 Space entries | -10 |
| Network policy entries | 32 | -6 |
| Project-task environment | 1536 bytes, 24 lines | -7 |
| Running project tasks | 2 per Space, 4 in total | -6 |
| Protected system headroom | 8 MiB (8,388,608 bytes) | -6 |
| Allocation unit on production images | 512 bytes (FAT32, one 512-byte sector per cluster) | n/a |

### 1.6 Per-Space namespace

Every path a process uses is a virtual path. The storage service translates it, on every request, using the caller's kernel-assigned Space. A caller never names physical `/SPACES/...` or `/AMITY/...` paths directly. The top-level names are case-insensitive (Unicode upper-case comparison).

| Virtual path | Physical target | Read | Write | Purpose |
|---|---|---|---|---|
| `/` | Synthetic | Listing only | Never | Lists, in byte-sorted order: CONFIG, HOME, PROJECT, SHARED, STATE, SYSTEM. `/APPS` is reachable but not listed. |
| `/PROJECT` | `<root>/PROJECT` | Yes | Yes, inside it (the root itself cannot be written, renamed or removed) | The Space's private user files ("Project"). Default folder for new Desktops. |
| `/HOME` | Normal Space: the same directory as `/PROJECT`. Legacy Space: shared physical `/HOME` (owner 0). | Yes | Yes, inside it | Familiar home folder. Private for normal Spaces, a shared compatibility pool for legacy Spaces. |
| `/STATE` | `<root>/STATE` | Yes | Yes, inside it | Private application and service state for this Space: environment, task definitions, app state. Packaged apps are redirected (section 4.3). |
| `/SHARED` | Synthetic | Lists the aliases of grants that target this Space | Never | Container for incoming shares. |
| `/SHARED/<alias>` | The grant's source directory, located by its stable FileId | Yes | Only if the grant is writable, and only inside it | Incoming share (section 3). |
| `/SYSTEM` | `/SYSTEM` | Yes | Never, for anyone, managers included | Shipped system files. |
| `/CONFIG` | `/CONFIG` | Yes, except `/CONFIG/DESKTOP.CFG` | Managers only | System preferences. `DESKTOP.CFG` can be read only by the desktop, storage managers and the app manager. Other processes receive a per-Space projection of it instead (section 6.2). |
| `/APPS/A<16 hex AppId>` | `/AMITY/APPS/A<id>` (global, owner 0) | A packaged app may read its own subtree only. The app manager may read everything. | App manager only | Installed package payloads. Shared by all Spaces and not charged to any Space. |

Any other first component, or a missing grant alias, returns -3. A path that tries to climb above the root (for example `/PROJECT/../../HOME`) returns -1. `..` is resolved on the text of the path before translation, so `/SHARED/x/../../STATE/f` means the caller's own `/STATE/f`, never the source Space's. After translation, the service resolves the physical result, including VFAT short-name aliases such as `SECRET~1.TXT`, and checks that it is still inside the mount base. If it is not, the request returns -3.

### 1.7 Binding processes to Spaces

- The kernel stamps the Space onto each process when it is created. The Space never changes afterwards. Switching, renaming or reordering Desktops does not move existing processes.
- An ordinarily spawned child inherits its parent's Space and receives no management role flags.
- Only the compositor can spawn a native image into a chosen Space. Only the image named `settings` may additionally receive STORAGE_MANAGER, and only from the compositor. The compositor itself runs in Space 1 with STORAGE_MANAGER and DESKTOP.
- The application manager launches packaged apps. A privileged requester (the desktop, a storage manager or the app-manager service) may choose any non-zero Space. Any other requester can launch only into its own Space, and only with permissions it already holds. The storage-manager launch flag is honoured only for the boot-archive Settings app (AppId 4).
- Apps that the shell launches from a Desktop run in that Desktop's Space. "Open project" in Settings switches to the selected Desktop and starts Files there at `/PROJECT`.
- A request may name an explicit Space other than the caller's own (a "Space override"). The storage service honours this only from managers, the app manager or the task service. From anyone else it returns -3. A Space that does not exist returns -2.

## 2. Quota model

### 2.1 What is charged

The usage of Space S is:

> used(S) = 512 + the sum, over every object owned by S, of (allocated(o) + metadata(o))

- **allocated(o)** is the number of bytes in the object's allocation chain: whole clusters, including slack. Directories count too: a new directory takes one cluster.
- **metadata(o)** is (physical path length in UTF-8 bytes + 63), rounded up to the next multiple of 512. The physical path is the full `/SPACES/S…/…` path. Paths of up to 449 bytes are charged 512, and paths of 450–961 bytes are charged 1024.
- **512 per Space** is a fixed base charge.
- The `overhead` field reported for a Space is 512 plus the sum of all metadata(o). The catalog's real physical size is system overhead and is not charged to anyone.
- A brand-new Space owns three directories (its root, `PROJECT` and `STATE`; path lengths 25, 33 and 31 bytes). With 512-byte clusters its used value is 512 + 3 × (512 + 512) = **3,584 bytes**, and its overhead is **2,048 bytes**.

### 2.2 Ownership

- The owner of an object is the Space whose private root contains its physical path. Everything outside every private root has owner 0: legacy `/HOME`, `/SYSTEM`, `/CONFIG`, `/AMITY/APPS` and the catalog. Owner 0 has no quota and is limited only by volume capacity.
- An object is charged once, to its owner, no matter how many Spaces can see it. Data written by Space B into a writable share from Space A is owned by A and charged to A.
- Moving an object into another owner's tree changes its owner and moves its charge. The move must still fit the new owner's quota (see 2.3). Exception: a directory that is the root of a share cannot change owner (section 3.6).
- Renaming recomputes the metadata charge of the renamed object and all of its descendants. A rename that makes a path longer can therefore fail with -14.

### 2.3 Enforcement

Requests are handled one at a time, so two callers can never both spend the same remaining budget. For each mutation:

1. **Budget.** The service finds the owner of the destination. That is the target path, or the second path for rename, copy and replace. It then sets a per-request allocation budget. The quota part is `quota − used(owner)`, saturating at 0, or unlimited if the quota is 0 or the owner is 0. The volume part is `free − 8 MiB`, saturating at 0.
2. **Peak allocation.** Every cluster allocated during the request counts against that budget. Clusters freed during the same request are not credited back. Saves always write the complete new content into a new chain before releasing the old one, so **an overwrite needs free quota at least as large as the new allocation**, even when the final usage would not change.
3. **Order of checks.** When a single allocation would exceed both limits, the quota is checked first (-14), then the volume (-6).
4. **Final check.** After the request, the service rescans the catalog. If **any** Space now has used > quota, with quota non-zero, the entire request is refused with -14 and rolled back. This catches metadata growth that the budget in step 2 does not cover. It also means that any over-quota Space would block every mutation on the volume.
5. **Failure.** A refused request changes nothing: file contents, FileIds, usage and quotas stay as they were. "Previous saved file retained" in the -14 message refers to this.
6. **Mount-time check.** When the volume is mounted, the catalog is verified against a full scan. If any Space is found over its quota, or anything else mismatches, the whole catalog becomes read-only.

Writes that land in owner-0 locations (legacy `/HOME`, `/CONFIG`) are bounded only by the volume rule. Space creation and legacy binding do not set a budget before creating their three directories, so (inferred) those three clusters are not held back by the 8 MiB headroom. The final quota check still applies to them.

### 2.4 Usage reporting

The per-Space record (`SpaceInfo`) contains:

| Field | Value |
|---|---|
| id, volume | Space id, volume UUID |
| quota | Byte-exact quota (0 means unlimited) |
| used | used(S) as defined in 2.1 |
| available | The smaller of (free − 8 MiB) and (quota − used), each saturating at 0. When the quota is 0, only the first term applies. |
| overhead | Metadata charge (2.1) |
| flags | 1 = LEGACY, 2 = READ_ONLY (volume-wide), 4 = RECOVERED (the journal was replayed at mount; volume-wide) |
| name | Name, NUL-padded to 128 bytes |

- The volume record contains capacity, available = free − 8 MiB, overhead = 8 MiB + 64 MiB journal = 75,497,472 bytes, the flags, and the name "Amity system volume". Only one volume exists.
- A `statfs`-like query on an open file or directory reports the availability of that object's owner: the Space's `available`, or the volume's `available` for owner 0. It also reports the raw free bytes, the allocation unit (512) and a maximum name length of 765 bytes.
- Who may read usage: managers may read any Space. Any process may read its own Space. Anything else returns -3. Listing all Spaces requires a manager or the app manager.
- Settings shows "Used X · Available Y" and "Metadata X · Journal/headroom Y", formatted as `<int>.<2 digits> MiB` with truncation, not rounding.

### 2.5 Changing a quota

- UPDATE(id, name, quota) sets both the name and the quota. If the new quota is non-zero and below the current used(S), it returns -14 and nothing changes. A quota exactly equal to used is accepted. A quota of 0 removes the limit.
- A quota is a limit, not a reservation. No capacity is reserved for it. Settings says "A maximum limit does not reserve disk capacity."
- Settings works in whole MiB. The field must hold a non-negative integer, which is multiplied by 1,048,576 with overflow checking. Otherwise the result is -1, shown as "Quota must be a whole number of MiB; zero means no Space limit." at creation or "Quota unchanged: Invalid path or argument" on Apply. The field shows `quota / 1 MiB`, truncated.
- Renaming a Desktop ("Save Desktop") re-reads the byte-exact quota and sends it back unchanged, so renaming never rounds a quota. Only the "Apply quota" button changes it. That button sends the Space's stored name, not the edited name field.
- Messages: "Quota saved durably" on success, "Quota unchanged: <error>" on failure.

### 2.6 Quota at creation

CREATE(name, volume UUID, quota) sets the quota before the final check runs. A non-zero quota below 3,584 bytes (with 512-byte clusters) therefore makes the creation fail with -14, and no Space is created. The UI only offers whole MiB, so it cannot hit this case. If the volume UUID does not match, the result is -2.

## 3. Sharing

### 3.1 Grant record

Each grant records the source Space, the target Space, the root (the FileId of the source directory), whether it is writable, and an alias. A grant names a directory by its stable FileId, not by its path.

### 3.2 Creating a share

SHARE(target, source, source_path, alias, writable) is open to managers only; anyone else gets -3. The service checks, in this order:

0. The request must be well formed (-1): the writable flag is 0 or 1, and the path field (1,024 bytes) and alias field (64 bytes) each hold NUL-terminated text with no character below U+0020. This step is what enforces the 63-byte alias limit.
1. The target and source Spaces must exist (-2).
2. The alias must be a valid filesystem name (-1): not empty, not `.` or `..`, no leading space, no trailing space or dot, at most 255 UTF-16 code units, no characters below U+0020, no U+007F or U+FFFF, and none of `" * / : < > ? \ |`.
3. The volume must hold fewer than 128 grants (-10).
4. No existing grant for the same target may use the same alias, compared case-insensitively with Unicode upper-casing (-8).
5. `source_path` is read in the **source** Space's namespace and must begin with `/HOME` or `/PROJECT` (-3 otherwise). This rules out `/STATE`, `/SYSTEM`, `/CONFIG`, `/APPS` and `/SHARED`, so a received share cannot be shared on. For a normal source, `/HOME/x` and `/PROJECT/x` are the same directory. For a legacy source, `/HOME/x` is the shared global HOME.
6. The path must resolve to an existing **directory** (-2 if it is missing or is a file). The root of the mount, such as `/PROJECT` itself, is allowed.

The service does not reject target equal to source, and it does not reject a second grant of the same directory under a different alias or of a directory nested inside one already shared. Shares are always directories, never single files. There is no operation to change an existing grant; to change it, unshare and share again.

Settings UI. The "Explicit directory share" section creates a share **into the selected Desktop**, which is the target. "From Desktop:" cycles through configured Desktops only, the selected one included. The source directory defaults to `/PROJECT`, the alias to `Shared`, and "Allow writes" starts off. Messages: "Directory share saved" or "Share not added: <error>". The list shows each incoming share as "<alias> · Read-only" or "<alias> · Read/write".

### 3.3 How a share appears in the target

- `/SHARED` lists the aliases of every grant whose target is the caller's Space.
- `/SHARED/<alias>/...` resolves to the grant's root directory plus the rest of the path. The root is found by FileId each time, so renaming it within its owner is followed automatically.
- The list of shares for a target (managers only) returns source Space, writable, alias and path. The path is shown as `/PROJECT/...` when the root is inside the source's project directory. Otherwise the raw physical path is shown, for example `/HOME/...` for a legacy source (see section 10).

### 3.4 Access through a share

- Read-only grant: reading, listing, stat and copying out are allowed. Writing, creating, mkdir, rename (at either end), remove, and copying or moving in all return -3. Spelling a name with its short alias gives no way around this.
- Writable grant: mutations are allowed inside the root. The root itself (`/SHARED/<alias>`) can never be written, renamed or removed (-3).
- The grant's writable flag is checked again on every operation, and so are the rights of any open handle.
- A copy out of a share creates new objects with new FileIds, owned by the destination Space.

### 3.5 Charging

All data under a shared root belongs to the source Space (or to owner 0 if the source is legacy HOME) and is charged there. A write by the target counts against the **source's** quota. The target's quota is not involved.

### 3.6 Keeping a shared root stable

- Delete: removing a directory that is the root of any grant returns -11 (Busy). Parent directories cannot be removed either, because only empty directories can be removed.
- Replace: a rename-replace whose destination is a shared root returns -11.
- Rename within the same owner: allowed, and the grant follows it.
- Move to another owner: refused with -19, "Folder is shared; remove its shares before moving it to another Desktop". This check covers shared roots anywhere under the moved path.

### 3.7 Revocation (UNSHARE)

UNSHARE(target, alias) is open to managers only. The alias is matched case-insensitively. A missing grant returns -2. Source files are never touched. Once the removal has been committed durably, every handle in the target Space whose path starts with `/SHARED/<alias>` is invalidated: directory handles, open file handles and pending handle-transfer offers. Those handles then return -3, and so do new path lookups. Settings shows "Directory share removed; source files retained" on success and "Share retained: <error>" on failure.

### 3.8 Shares when a Desktop is removed

Removing a Desktop removes neither the Space nor any grant. Grants from it and into it persist and keep working for any process that can still reach them. While a Space has no Desktop entry, Settings cannot show or remove the shares that target it (see section 10).

## 4. Isolation and authority

### 4.1 Roles

All role flags come from the kernel's process context. A request cannot supply them.

| Role | How a process gets it | Storage authority |
|---|---|---|
| Ordinary process | Default | Its own Space's namespace only. No Space override. Can read its own usage. |
| Packaged application | Started by the app manager with an AppId and permissions | As an ordinary process, but needs FILE_READ for reads and FILE_WRITE for writes, has its `/STATE` redirected, and may read its own `/APPS` subtree (section 4.3). |
| Manager: desktop (compositor) or storage manager (Settings) | Kernel-assigned | May override the Space, list Spaces and volumes, create, update, bind legacy, share, unshare, list shares, read any usage, write `/CONFIG`, read `DESKTOP.CFG`, and set network policy. Like everyone else, cannot write `/SYSTEM`. |
| App manager | Kernel service identity | May override the Space, list Spaces and volumes, read and write `/AMITY/APPS` through `/APPS`, and read `DESKTOP.CFG`. A request without an explicit Space defaults to Space 1. |
| Task service | Kernel service identity | May override the Space. Acts for desktops, storage managers and the Terminal (AppId 3, own Space only). |
| Network service | Kernel service identity | May read and set network policy for any Space. Acts as the broker for handle transfers. |

The compositor additionally refuses two requests unless the sender is a storage manager: applying Settings (changing the Desktop list) and opening a project in another Space. A window title confers no authority.

### 4.2 How a path is resolved

For each request the service:

1. Authenticates the caller's context and determines the effective Space (its own Space, or a permitted override).
2. If the path goes through a directory handle, checks that the handle belongs to the same process and was opened in the same Space (otherwise -3), and that it carries the read right, or the write right for a mutation (otherwise -3). It then rebuilds the handle's current location from its FileId and appends the request path. The request path is confined to the handle's subtree: a request path that repeats the handle's own base path has that prefix removed first, and `..` cannot climb above its start.
3. For packaged apps: handles `/APPS` separately, checks permission bits, and redirects `/STATE` (section 4.3).
4. Translates the first path component as described in section 1.6. Writes are refused (-3) to read-only mounts and to the mount root itself.
5. Resolves the physical path, following short names, and refuses (-3) a result that falls outside the mount base.
6. Refuses (-3) `DESKTOP.CFG` to non-managers.
7. For operations on an open file: rebuilds the file's virtual path from its FileId and repeats steps 3–6. If the path now names a different object, for example after delete and recreate, the result is -3.

### 4.3 App-private storage

- `/STATE/...` for a packaged app with AppId N is stored at `/STATE/Applications/A<N as 16 uppercase hex>/...` inside the Space. If the app already uses that full prefix, it is not added a second time. The Browser (AppId 5) is the one exception: its state stays at `/STATE/Browser/...`. App state is therefore per (Space, app) and is charged to the Space.
- `/APPS/A<N>/...` holds the read-only package payload. It is global across Spaces. An app may read only its own `A<N>` subtree. `/APPS` itself, another app's subtree, and any write all return -3. An unpackaged process (AppId 0) gets -3 for every `/APPS` path. The `/APPS` check happens before the FILE_READ check, so (inferred) an app can read its own payload without FILE_READ.
- Unpackaged processes (AppId 0) have no permission checks and see the raw `/STATE` of their Space, including `Environment.BIN` and `Tasks/`.

### 4.4 Directory handles as capabilities

- OPEN_DIRECTORY(path, rights) takes rights 1 (read), 2 (write) or 3 (read and write). Rights of 0, or with any other bit set, return -1. A handle without the read right (rights 2) is accepted but fails every use with -3, because every use requires the read right. The path must be a directory. Asking for write first checks that a child path could be written (it test-resolves `<path>/.amity-write-check`). Opening `/` or `/SHARED` always succeeds and skips that check.
- A handle belongs to the process that opened it and to the Space it was opened in. It is limited to 16 per process and 256 in total. It follows its directory through renames within the same owner. It fails with -3 if the directory is deleted, even when a new directory is created at the same path, and it fails if a revoked share made it unreachable. Handles are cleaned up when their process exits.

### 4.5 Passing handles between processes

A process in a non-zero Space can offer a directory or file handle through a broker, which must be the network service in Space 0. The receiving process must be in **the same Space and have the same AppId** as the sender, otherwise -3. Authority is re-checked when the handle is claimed. The limits are 8 pending offers per sender and 64 in total. Handles therefore never cross Space or app boundaries.

### 4.6 Other per-Space partitions

- Local (Unix-like) sockets: socket names are scoped to (Space, AppId). Space 0 is refused.
- DNS cache entries are stored per Space.
- Project tasks are stored per Space, with at most 2 running per Space.
- The clipboard (section 7) and network policy (section 6.4) are per Space.

## 5. Removing a Desktop and retained storage

- **Removing a Desktop only removes its configuration entry.** The Space record, its files, quota, usage and all shares (from it and to it) remain.
- Removal is refused with -5 if any process belongs to that Space. Settings asks the task service which processes run in the Space; the compositor checks again for live windows, pending launches and processes in the Space. Settings itself always runs in some Space, so it cannot remove its own Desktop. The message is "Open Settings from another Desktop, then close this Desktop's applications before removing it. Files are retained." The last remaining Desktop cannot be removed: the Remove button is disabled and the configuration returns -1.
- Confirmation dialog: title "Remove desktop?", text "Remove this configuration from the list.\nPrivate files and shares stay in Retained storage.", buttons Cancel and Remove. Other failures show "Cannot remove Desktop safely: <error>" or "Cannot inspect Desktop ownership: <error>".
- On removal, the compositor drops the clipboard contents of the removed Spaces and drops their saved windows. If the active Desktop was removed, Desktop 0 becomes active.
- **Retained storage** lists every Space in the catalog that has no Desktop entry, by its stored name. If there are none, it shows "All stored Spaces have a Desktop entry." "Add back" creates a Desktop entry with the stored name, the same Space id and folder `/PROJECT`. It never moves, recreates or claims files. On failure it shows "Stored files retained: <error>".
- Create is ordered so that nothing is lost. Settings first creates the Space, then adds the Desktop entry. If adding the entry fails: "Storage retained; Desktop not added: <error>. Use Add back below." If the create result is unknown (-13 or -15): "Creation outcome is unknown. Refresh Retained storage and check before creating another Desktop."
- Because no delete exists, retained storage keeps counting toward the 32-Space limit and toward disk usage indefinitely. The only way to free that space is to delete the files inside the retained Space, after adding it back.

## 6. Per-Space environment, settings and network policy

### 6.1 Project-task environment

- The environment is stored at `/STATE/Environment.BIN` in the Space, as text: lines of `KEY=VALUE` separated by `\n`. It is owned by the task service. A missing file means an empty environment. An invalid stored file is reported but never repaired. The task service fails reads of it, so Settings shows "Environment unavailable: <error>", or "Stored environment is invalid; existing data was preserved." if invalid content reaches the UI. Because compare-and-set first reads and validates the current value, an invalid file also blocks every Settings save.
- Validation:
  - Total size at most 1,536 bytes, and no NUL (-7).
  - At most 24 lines, counting empty lines (-7).
  - Each non-empty line contains `=` (-1).
  - The key matches `[A-Za-z_][A-Za-z0-9_]*` and is 1–64 bytes long (-1).
  - The value contains no control characters (-1).
  - No key appears twice (-1).
  - The reserved keys `PWD`, `AMITY_SPACE_ID`, `TMPDIR` and `BUILD_DIR` are rejected (-1).
  - `HOME` is allowed and overrides the default.
- The variables apply **only to project tasks started by the task service**, not to ordinary app launches (inferred: the environment is built only in the task service). When a task starts, the environment is assembled as:
  1. the Space's variables;
  2. the task definition's own variables, which override them;
  3. `PWD=<task cwd>`, `AMITY_SPACE_ID=<decimal id>`, `TMPDIR=/STATE/Temp`, `BUILD_DIR=/STATE/Build`;
  4. `HOME=/PROJECT`, unless HOME was set.

  The assembled result, NUL-separated, must fit in 2,048 bytes (-7).
- Writes are compare-and-set. The client sends the SHA-256 of the content it read; if that no longer matches, the result is -46. Valid writes are synced durably. Running processes keep the values they started with.
- Who may read or write it: desktops and storage managers for any Space, and the Terminal (AppId 3) for its own Space. Everyone else gets -3.
- UI messages:
  - "Refresh to read this Desktop's project-task variables."
  - "Loaded. Set or remove a variable to save it."
  - "Refresh must succeed before changing stored variables."
  - "Use an ASCII variable name and a value without control characters; OS-owned names are reserved."
  - "There is no stored variable with this name."
  - "Environment exceeds the supported 1536-byte/24-variable limit. Previous values retained."
  - "Environment saved durably. Running processes keep their existing values."
  - "Changed in another window. Draft retained; Refresh and review before saving." (-46)
  - "Save outcome is unknown. Draft retained; Refresh and check before retrying." (-13 or -15)
  - "Environment not changed: <error>"
  - "Environment unavailable: <error>"
  - "Values apply to new project tasks in this Desktop."

### 6.2 Per-Desktop presentation settings

These are owned by the compositor and stored for all Desktops together in `/CONFIG/DESKTOP.CFG`, not inside the Space:

- name (also mirrored into the catalog);
- "Relevant folder" (at most 1,023 bytes; must be an existing directory as seen from that Space; otherwise "Choose an existing guest folder");
- pinned apps and taskbar order;
- "Fixed layout" and "Project" toggles;
- wallpaper (empty means inherit the global one) and Fill/Fit;
- "Restore saved windows at startup" (on by default);
- accent (Global, Violet, Aqua, Amber);
- scale (Global, 100%, 125%, 150%);
- effects (Global, Full, Reduced).

A non-manager process that reads the Desktop configuration receives a projection containing only its own Space: its entry, its saved windows, and the global values resolved for that Space. It never sees other Desktops' names or folders. A projection request for a Space with no Desktop entry returns -3.

Per-(app, Space) grants also exist: enabled, start-at-login, permission set and accepted licence. They are kept by the app manager ("Installed once. Enabled and configured separately for each Desktop.") and are outside the sources of this spec.

### 6.3 Network policy

- Storage: `/AMITY/NETWORK.BIN`, a versioned file bound to the volume. Each entry is a (Space id, denied) pair. Entries are sorted by id, ids are unique and non-zero, there are at most 32 entries, and the file is protected by a CRC.
- **A missing file means every Space is allowed** (the fresh-install state). A Space with no entry is allowed. A malformed file, or one bound to another volume, is an error and is never treated as allowed. An entry for a Space that does not exist is also an error.
- Space 0 can never get a policy. The network service refuses it with -3, the storage service refuses it with -2 (there is no Space 0), and the policy store itself refuses it with -1. Adding a 33rd new entry returns -6.
- Who may use it: reading is allowed for managers, the network service and a process asking about its own Space; others get -3. Setting is allowed for managers and the network service; others get -3. Settings changes it through the network service, which writes the file, clears its cached decision, and on deny immediately aborts that Space's connections.
- How the network service enforces it:
  - Opening a socket, DNS resolution and connect all need a fresh decision. Other socket operations use a cached decision for 20 ticks.
  - If the storage service is busy, established traffic may use the last confirmed decision for up to 100 ticks. New operations fail.
  - Any other error fails closed with EACCES.
  - Every 20 ticks the service re-checks policy for each Space that has sockets. A Space that is denied or unknown has its sockets aborted, its queued frames dropped, its pending DNS queries failed with EACCES and its DNS cache cleared.
  - A tick is 10 ms (inferred: Settings shows ticks / 100 as seconds).
- A packaged app without the NETWORK permission gets EACCES for everything except status, close and cancel-DNS.
- Status reports `denied = 1` whenever the policy is anything other than a known "allowed", including when it is unknown.
- UI text:
  - "Desktop network access"
  - "Allow this Desktop to use the network"
  - "Blocking closes this Desktop's active connections."
  - "Desktop network access blocked and saved." / "Desktop network access allowed and saved."
  - "Policy not saved: <error>"
  - "Policy unavailable: <error>" followed by "Check storage recovery, then Refresh. Access stays blocked."
  - "Desktop network policy unavailable; access blocked."
  - "Open Settings in that Desktop to test its access." (the connection test runs only for Settings' own Space)
  - "Adapter connect/disconnect affects all guest Desktops."

## 7. Clipboard partitioning

- The clipboard lives only in compositor memory, as one byte buffer per Space id. A compositor restart clears all clipboards. Clipboards are never written to disk.
- SET(space, bytes):
  - Space 0 returns -3.
  - The content must be UTF-8 (-1, checked at the compositor boundary).
  - The total of all **other** Spaces' contents plus the new content must be at most 1,048,576 bytes (-10).
  - A message payload larger than 1 MiB returns -10.
  - Empty content deletes the Space's entry.
  - A Space without an entry when 32 entries already exist returns -10.
  - The replacement buffer is allocated before the old content is discarded, so a failure always leaves the previous contents in place.
- GET(space) returns that Space's bytes, or empty. Space 0 returns -3. A caller buffer smaller than the content returns -10.
- The caller's Space comes from the kernel context of the sending process. All apps in one Space share one clipboard. Packaged apps need the CLIPBOARD permission; unpackaged ones do not. The compositor's own text fields (shell dialogs) use the clipboard of the **active** Desktop's Space.
- When the Desktop list changes, the compositor drops the clipboard of every Space that no longer has a Desktop entry. This happens even though the Space's storage is retained.
- Copying data from one Space into another needs an explicit share. The clipboard confers no authority between Spaces.

## 8. Validation rules and user-visible messages to keep

### 8.1 Validation summary

| Input | Rule | Error |
|---|---|---|
| Space name | 1–127 bytes, no character below U+0020. The UI also trims (on create) and rejects all control characters. | -1 |
| Quota (API) | Any u64, 0 = unlimited. Must not be below current usage on update or at the final check on create. | -14 |
| Quota (UI) | Whole MiB, non-negative, no overflow | -1 |
| Legacy ordinal | Less than 32 | -1 |
| Volume id on create | Must equal the one volume's UUID | -2 |
| Share alias | Filename rules (3.2), at most 63 bytes, unique per target ignoring case | -1 / -8 |
| Share source | `/HOME…` or `/PROJECT…` of the source, an existing directory | -3 / -2 |
| Path components | At most 32 components and 1,023 bytes; no `..` above the root | -1 / -10 |
| Directory rights | 1, 2 or 3; rights 2 alone opens but every use fails | -1 / -3 |
| Desktop folder | At most 1,023 bytes, an existing directory in that Space | UI message |
| Environment | See 6.1 | -1 / -7 |

### 8.2 Messages in the Desktops section of Settings

Creation dialog: title "Create Desktop", subtitle "Private project files and application state on a guest volume.", fields "Name", "Guest storage location" and "Storage limit in MiB (0 = no limit)", note "A maximum limit does not reserve disk capacity.".

Creation errors, in the order they are checked:
1. "Recover the Desktop configuration before creating another Desktop." (configuration is read-only, or an entry has an unresolved id)
2. "Enter a name of 1–127 UTF-8 bytes without control characters."
3. "The Desktop limit has been reached."
4. "Not enough memory to add a Desktop."
5. "No available guest volume. Check storage recovery."
6. "This volume is read-only. Recover storage before creating a Desktop."
7. "Quota must be a whole number of MiB; zero means no Space limit."
8. "Desktop not created: <error>"

Creation success: "Desktop ‘<name>’ created. Switch when you are ready."

Storage state lines:
- "Read-only: recover storage before saving"
- "Recovered: journal replay completed"
- "Storage ready"
- "Legacy Home is shared; Project and app state are private."
- "Home and Project use this Desktop's private storage."
- "Storage state unavailable; do not assume a save is durable."

Refresh failures:
- "Guest volumes unavailable: <error>"
- "Desktop storage unavailable: <error>"
- "Directory shares unavailable: <error>"
- "Retained storage unavailable: <error>"

Save and apply:
- "Desktop storage not updated: <error>"
- "Preferences not applied: <error>"

The share, unshare, quota, removal and retained-storage messages are listed in sections 2.5, 3.2, 3.7 and 5.

## 9. Test vectors

Unless a vector says otherwise, it assumes:
- 512-byte allocation units;
- a fresh normal Space A with id 2 and Space B with id 3, both created with quota 0, used = 3,584 each;
- ample free space (at least 8 MiB of headroom beyond what the test needs);
- the `PROJECT` directory never needs a second cluster.

The byte-exact quota results (Q3–Q7, Q10) are specific to the FAT charging formula. Section 12 explains how to re-baseline them.

| # | Setup / input | Expected outcome |
|---|---|---|
| Q1 | CREATE("Work", volume, quota 0) | Succeeds with a new monotonic id. used = 3,584; overhead = 2,048; quota = 0; available = free − 8 MiB; flags = 0; physical directories `/SPACES/S…<id>`, `PROJECT` and `STATE` exist. |
| Q2 | CREATE("Tiny", volume, quota 1,000) | -14. No Space is created and the next id is not consumed. |
| Q3 | CREATE("Edge", volume, quota 3,584) | Succeeds, with available = 0. Any later write that allocates a cluster in it returns -14. |
| Q4 | A: UPDATE(quota 1,048,576). Write new `/PROJECT/a.bin` of 1,044,480 bytes. | Succeeds. used = 3,584 + 1,044,480 + 512 = 1,048,576; available = 0. |
| Q5 | As Q4, but the file is 1,044,481 bytes. | -14. The 2,041 clusters (1,044,992 bytes) fit the allocation budget, but the final check finds 1,049,088 > 1,048,576. The file does not exist afterwards and used = 3,584. |
| Q6 | A at quota 1 MiB holds `/PROJECT/f` of 614,400 bytes, so used = 618,496. Overwrite `f` with 614,400 new bytes. | -14: the peak needs 614,400, but only 430,080 is left. The old content, FileId and used are unchanged. Overwriting with 430,080 bytes succeeds, leaving used = 434,176. |
| Q7 | A with used = 618,496: UPDATE(quota 524,288); then UPDATE(quota 618,496); then UPDATE(quota 0). | First -14 with the quota unchanged; second succeeds (equal is allowed); third succeeds (unlimited). |
| Q8 | Quota 1 MiB, used 3,584. Volume free = 20 MiB, then 8.5 MiB, then 7 MiB. | available = 1,044,992, then 524,288, then 0. |
| Q9 | Legacy Space L with quota 1 MiB writes 5 MiB to `/HOME/big.bin`; then 5 MiB to `/PROJECT/big.bin`. | The first succeeds with owner 0, and L's used is unchanged. The second returns -14. |
| Q10 | A is exactly at its quota and renames a file whose physical path grows from 449 to 450 bytes. | -14, because the metadata charge rises from 512 to 1,024. The rename does not happen. |
| S1 | Manager: SHARE(target B, source A, "/PROJECT/docs", "Docs", read-only), where docs is a directory. | Succeeds. B's listing of `/SHARED` is ["Docs"]. SHARES(B) returns {source 2, writable 0, alias "Docs", path "/PROJECT/docs"}. |
| S2 | B, after S1: read `/SHARED/Docs/x.txt`; read `/SHARED/DOCS/X.TXT`; write, mkdir, rename and remove under `/SHARED/Docs`; copy `/PROJECT/y` to `/SHARED/Docs/y`; copy `/SHARED/Docs/x.txt` to `/PROJECT/x.txt`. | The two reads succeed. Every mutation returns -3. The copy in returns -3. The copy out succeeds, creating a new FileId owned by B (owner_space 3). |
| S3 | B reads `/SHARED/Docs/../../STATE/state.txt`. | Resolves to B's own `/STATE/state.txt`, returning B's file or -2. It never reaches A's state. |
| S4 | SHARE(B, A, "/PROJECT/drop", "Drop", writable). B writes 100 bytes to `/SHARED/Drop/n.txt`. | Succeeds. The identity has owner_space 2. A's used rises by 1,024 (one cluster plus 512 metadata). B's used is unchanged. A can read the file at `/PROJECT/drop/n.txt`. |
| S5 | As S4, but A is at its quota. | B's write returns -14, even though B has room. |
| S6 | SHARE errors: source path "/STATE/x" → -3; "/SYSTEM" → -3; "/SHARED/Docs" → -3; "/PROJECT/file.txt" (a file) → -2; "/PROJECT/missing" → -2; alias "docs" again for B → -8; alias "a:b" → -1; alias "x." → -1; alias of 64 bytes → -1; target id 99 that does not exist → -2; the 129th grant on the volume → -10; request from a non-manager → -3. | As listed. |
| S7 | A removes the empty `/PROJECT/docs` while S1 is active. Then A renames `/PROJECT/docs` to `/PROJECT/papers`. | The remove returns -11. The rename succeeds; B's `/SHARED/Docs` now shows `papers`, and SHARES(B) shows path "/PROJECT/papers". |
| S8 | A moves the shared `/PROJECT/papers` into a writable share it has from Space C. | -19, and nothing moves. |
| S9 | B holds a directory handle on `/SHARED/Docs` and an open file under it. Manager: UNSHARE(B, "DOCS"), then UNSHARE(B, "Docs") again. | The first succeeds. Afterwards the handle and the open file both return -3, a path lookup under `/SHARED/Docs` returns -3, and A's files are intact. The second returns -2. |
| I1 | An ordinary process in B reads `/SPACES/S0000000000000002/PROJECT/secret.txt`; writes `/AMITY`, `/SPACES`, `/AMITY~1` and `/PROJECT/../SPACES`; writes `/PROJECT/../../HOME`; reads and writes `/CONFIG/DESKTOP.CFG`. | All return -3, except `/PROJECT/../../HOME`, which returns -1. |
| I2 | An ordinary process in B sends a request with a Space override set to A; calls UPDATE on A; calls USAGE(A); calls USAGE(B). | -3, -3, -3, then success. |
| I3 | Settings (manager) writes `/SYSTEM/x`, then writes `/CONFIG/foo`. An ordinary process writes `/CONFIG/foo`. | Settings: -3, then success. Ordinary process: -3. |
| I4 | Packaged app with AppId 0x1234 in A. Without FILE_WRITE it writes `/PROJECT/a`. With FILE_WRITE it writes `/STATE/c.db`. It writes `/APPS/A0000000000001234/x` and reads `/APPS/A0000000000005678/m`. An unpackaged process reads `/APPS/A0000000000001234/m`. | -3. Then success, with the file stored at `<A root>/STATE/Applications/A0000000000001234/c.db`. Then -3, -3, and -3 for the unpackaged process. |
| I5 | Process P opens a directory handle; process Q in the same Space uses that handle; P uses it with a different Space override; P opens a 17th handle. | -3, -3, -6. |
| I6 | A directory handle on `/PROJECT/d`. The directory is removed and a new `/PROJECT/d` is created. The old handle is used. | -3. The handle never reaches the replacement directory. |
| N1 | Space names: "" → -1. 127-byte name → succeeds. 128-byte name → -1. "A\tB" → -1. Two Spaces named "Work" → both succeed with distinct ids. "Work\u007F" through the API → succeeds; through Settings → rejected with the name message. "  Work  " through Settings Create → stored as "Work". | As listed. |
| N2 | The catalog holds 32 Spaces: 31 Desktops and 1 retained. Settings Create. | Settings' own check passes (31 < 32), but the service returns -1, shown as "Desktop not created: Invalid path or argument". |
| L1 | Fresh catalog: BIND_LEGACY(0,"Personal"); BIND_LEGACY(0,"Renamed"); BIND_LEGACY(1,"Project"); BIND_LEGACY(32,"X"). | Returns 1; then 1 again with the name changed to "Renamed"; then a new id 2 with legacy ordinal 1; then -1. |
| R1 | Desktop "Work" (id 4) has no processes. Remove it, then refresh Settings, then Add back. | The configuration entry is gone. SPACES still lists id 4 with the same used and the same shares. Its clipboard is cleared. Retained storage lists "Work". Add back produces a Desktop with id 4, folder `/PROJECT`, and every file intact. |
| R2 | Remove the Desktop that Settings itself runs in. Remove the only Desktop. | First: -5, shown as "Open Settings from another Desktop, then close this Desktop's applications before removing it. Files are retained." Second: the Remove button is disabled and the configuration returns -1. |
| E1 | Environment writes: "FOO=bar" → saved. "1FOO=x" → -1. "PWD=/x" → -1. "HOME=/PROJECT/src" → saved (overrides the default HOME). "A=1\nA=2" → -1. "K=a\tb" → -1. 25 lines → -7. 1,537 bytes → -7. A CAS whose stale hash no longer matches → -46. | As listed. A task started afterwards sees FOO=bar, PWD, AMITY_SPACE_ID=<id>, TMPDIR=/STATE/Temp, BUILD_DIR=/STATE/Build, and HOME. |
| C1 | Clipboard: set(1,"one"); set(2,"two"); get(1); get(3); set(1,"changed"); get(2). | "one", then empty, then "two". Space 1 now holds "changed". |
| C2 | Clipboard: set(1, 1,048,573 bytes); set(2,"two"); set(2,"four"); set(0,"bad"); set(1,""); set(2,"four"). | Space 2's set of "four" first returns -10 and keeps "two". set(0) returns -3. After Space 1 is cleared, set(2,"four") succeeds. |
| C3 | Clipboard: set Spaces 1–32 to "x"; set(33,"next"); set(1,""); set(33,"next"). | The first set(33) returns -10. After Space 1 is cleared, set(33) succeeds. |
| P1 | No `NETWORK.BIN` exists. Space 2 opens a TCP socket. Manager sets Space 2 to denied. Space 2 then opens a socket. Space 3 opens a socket. Space 3 queries policy for Space 2. A manager sets policy for space 0 through the network service. | First open succeeds (missing file means allowed). After deny: Space 2's existing sockets are aborted and its new open returns -3. Space 3's open succeeds. Space 3's query about Space 2 returns -3. Setting space 0 returns -3. |
| P2 | `NETWORK.BIN` has a bad CRC. | The policy read returns an error. Every Space is treated as denied, and status shows denied = 1. |

## 10. Open questions and behaviours that looked accidental

1. **There is no deletion.** Spaces cannot be deleted, so "Retained storage" can only grow. The 32-Space cap applies over the whole lifetime of the volume. The Linux port must decide whether to add a real delete, and if so what happens to grants that refer to the deleted Space.
2. **Fresh installs are legacy.** The three default Desktops on a fresh install are legacy Spaces sharing `/HOME`. Keep the legacy shared HOME, or drop it on Linux?
3. Name checks differ between the service and Settings: the service allows U+007F and C1 controls and does not trim. "Save Desktop" validates the trimmed name but sends it untrimmed.
4. BIND_LEGACY checks the 32-Space limit before looking up an existing ordinal, so rebinding an already-bound ordinal fails when the catalog is full.
5. A share whose target equals its source is accepted.
6. Only `/HOME` and `/PROJECT` may be shared, but a shared root can later be renamed, within the same owner, into `/STATE`. The grant follows it, which exposes state. SHARES then reports the raw physical path (`/SPACES/S…/STATE/…`).
7. Deleting a shared root returns -11, displayed as "Try again", which does not tell the user why. Moving it to another owner gets a specific message (-19).
8. Lowering a quota below current usage returns -14, whose text ("…Previous saved file retained") is written for saves, not settings.
9. Settings shows the quota truncated to whole MiB. Pressing "Apply quota" turns a byte-exact quota into whole MiB. Rename keeps the exact value.
10. The final quota check examines every Space, not just the one being written. A single over-quota Space would make every mutation on the volume fail, and at mount it makes the whole catalog read-only.
11. Creating a Space and binding legacy ordinals do not reserve the 8 MiB headroom for their own three directories (inferred).
12. Settings can only pick configured Desktops as share sources, and it lists incoming shares only for configured Desktops. Grants involving a retained Space persist but cannot be seen or removed until the Space is added back.
13. The clipboard is tied to the Desktop list, not the storage catalog. Removing and re-adding a Desktop loses its clipboard.
14. App-manager (and Wi-Fi-service) requests that name no Space are treated as Space 1. Space 1 is therefore special beyond being "Personal".
15. Every process can read `/CONFIG`. Only `DESKTOP.CFG` is protected.
16. The environment's 24-line limit counts empty lines. The task's final environment can reach 48 user and definition variables plus 5 OS-owned ones, while the kernel ABI declares 32 environment entries; whether the kernel enforces that was not checked.
17. Alias collisions are detected with Unicode upper-case folding, but `/SHARED` listings are sorted by raw bytes, so upper-case names sort first.
18. The name stored in the catalog and the Desktop name can drift apart. At boot, BIND_LEGACY copies the configuration name into the catalog only for unresolved entries. "Add back" uses the catalog name.
19. Write authority is decided per request, not per open handle. A handle opened with write rights through a share can still be refused a write if the grant has since become read-only, and the reverse also applies.
20. A legacy Space's shared `/HOME` contains an ordinary folder named `/HOME/PROJECT`, the default folder of the "Project" Desktop. It has nothing to do with the private `/PROJECT`, and the two are easy to confuse.

## 11. Disagreements between code and design docs

| Doc | Doc says | Code does |
|---|---|---|
| space-storage-ui.md | The configuration format is `AMITYC05` with 2,196-byte records. | Writes `AMITYC07` and still reads C01–C06. |
| space-storage-ui.md | "Applications do not yet serialize separate private state files beneath `/STATE`." | `/STATE` is in active use: the task service keeps `Environment.BIN` and `Tasks/Definitions`, packaged apps are redirected to `/STATE/Applications/A<id>`, and the Browser uses `/STATE/Browser`. |
| space-storage-contract.md | "Only the trusted desktop can select [the Space] at spawn." | The compositor is the only one that can spawn a native image into a chosen Space. The application manager, however, launches packaged apps into any Space for privileged requesters (desktop, storage manager, its own service), and the task service launches tasks into a target Space through it. |
| space-storage-contract.md | "Only a deliberately delegated immutable Settings executable receives storage-management rights." | The compositor itself holds STORAGE_MANAGER, and the service's admin check accepts either DESKTOP or STORAGE_MANAGER. Kernel-launched test probes also receive it. |
| space-storage-contract.md | "Closing/revoking a share invalidates its issued directory handles." | Revocation also invalidates open file handles and pending handle-transfer offers. The doc understates it. |
| space-storage-contract.md | "32 configured/retained Spaces." | 32 is the total catalog size, legacy Spaces included. Nothing is ever deleted, so it is a lifetime cap. |
| space-storage-contract.md | "System binaries/preferences are read-only to ordinary applications." | `/SYSTEM` is read-only to everyone through the namespace, managers included. `/CONFIG` is writable by managers and readable by every process except for `DESKTOP.CFG`. |
| space-storage-contract.md | A shared root "cannot move to another owner while its shares exist." | Correct (-19). In addition, deleting or replacing a shared root returns -11, which the doc does not mention. |
| space-storage-contract.md / space-storage-ui.md | Imply that the shared legacy HOME applies to "existing/default legacy configurations". | Correct, but the consequence is not spelled out: every fresh install's three default Desktops are legacy (section 1.4). |

`managed-storage.md` covers only journal and format mechanics. Nothing in it conflicts with the Space behaviour described here.

## 12. Porting notes for the Linux base (non-normative)

- **Space to storage.** One btrfs subvolume per Space, for example `spaces/S<16 hex>`, containing `PROJECT/` and `STATE/`, with a qgroup limit equal to the quota (none when it is 0). Keep ids monotonic and never reuse them. Keep a catalog of id, name, quota, legacy and grants that is updated atomically. The legacy shared HOME, if kept, becomes an unquota'd subvolume outside the Spaces.
- **Quota arithmetic.** The FAT formula (cluster bytes, 512 + per-path metadata, 512 per Space) cannot be reproduced exactly with qgroups. The properties to keep are:
  - the limit is hard and checked before commit;
  - a failed write leaves the old content and usage unchanged;
  - a whole-file replacement needs headroom for the new copy (btrfs copy-on-write plus write-then-rename behaves this way);
  - data is charged to its owner, not to whoever wrote it;
  - a quota cannot be lowered below current usage;
  - available = min(quota − used, free − headroom).

  Vectors Q3–Q7 and Q10 need new byte figures once the accounting has been chosen.
- **Namespace.** Use a mount namespace per Space, or per app instance. Bind `PROJECT` at `/PROJECT` (and at `/HOME` for normal Spaces), `STATE` at `/STATE`, each grant at `/SHARED/<alias>` (read-only bind for read-only grants), and `/SYSTEM` read-only. `/CONFIG` is read-only, with the desktop configuration hidden. Add Landlock rules denying everything else. For packaged apps, bind `STATE/Applications/A<id>` over `/STATE`, and `/APPS/A<id>` read-only.
- **Differences to decide on:**
  - Renaming across subvolumes fails with EXDEV, whereas AmityOS moved objects between owners and transferred their charge.
  - Linux cannot revoke open file descriptors when a bind mount is removed, whereas AmityOS invalidated open handles on unshare.
  - Bind mounts alone do not stop the source directory from being removed or renamed (inferred), so a storage manager must enforce the shared-root protections in 3.6.
- **Processes.** A systemd user slice per Space. Children stay in the Space's slice, and only the desktop or Settings starts units in a chosen Space. Per-Space task limits (2 running) map onto slice or unit limits.
- **Network.** Per-Space deny via the cgroup, for example an nftables cgroupv2 match, or a network namespace per Space. Default allow; fail closed on an unreadable policy; abort established connections when access is denied.
- **Clipboard.** Partition it by the Space of the requesting client, resolved from the client's cgroup or pidfd, never from anything the client claims. Keep the 1 MiB total and 32-entry limits, and keep the content in memory only.
