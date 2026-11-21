# Roadmap

AmityOS is built on the Linux kernel ([decision 0001](decisions/0001-linux-base.md)).
Each phase ends with a commit or PR, an acceptance note in [acceptance](acceptance/), and
a summary of what works, what was tested and what was not. A phase does not start on a
broken one.

| Phase | Deliverable | Acceptance |
|---|---|---|
| 0 Plan | [Migration plan](migration/plan.md) and decision records | Done: [phase-0](acceptance/phase-0.md) |
| 1 Restructure | Earlier prototype code removed from the tree and from history; new Cargo workspace; behaviour [specs](specs/) | `cargo build` and `cargo test` pass on Linux (WSL); a fresh clone contains none of the removed code. [phase-1](acceptance/phase-1.md) |
| 2 Bootable image | mkosi builds a branded image and installable ISO; `AmityOS.ps1` boots it in QEMU from Windows with storage on F: | Boots to a graphical session; networking works; Chrome and VS Code install and launch; real-hardware boot steps documented |
| 3 Compositor and shell MVP | `amity-comp` nested and on DRM/KMS: window management, XWayland, taskbar, launcher, MRU switcher, multi-monitor | VS Code, Chrome and Blender run on it |
| 4 Redesign | Design inventory, design system in `amity-ui`, every design screenshot implemented | A saved side-by-side comparison for each screenshot; gaps and conflicts listed; WCAG AA, keyboard navigation, 100–200 % scaling, reduced motion |
| 5 Spaces, apps, tasks | `amity-spaces`, `amity-apps`, `amity-tasks` | Create a Space, set a quota, share a folder, launch an app in a Space, switch Spaces without killing apps, restore a session after reboot |
| 6 Amity AI runtime | `amityd`, provider trait, local model (GPU where available), assistant surface, "Explain this", VS Code extension | Each behaviour observed; the placeholder model is labelled as a placeholder |
| 7 Compatibility matrix | [acceptance/compatibility.md](acceptance/compatibility.md) | VS Code, Chrome, Firefox, Unity Hub + Editor, Unreal Editor, Blender, Godot, Android Studio + emulator, Docker, Git, Node, Python, Rust, vkcube, a CUDA sample, a HIP/ROCm sample. Anything not run for lack of hardware is recorded as "not run" |
| 8 Product layer | Installer, signed updates (A/B or OSTree-style), first-run setup, branding, AI privacy settings, crash and diagnostics opt-in | Per feature |

## "Explain this"

An Amity AI feature (Phase 6): explain a failure from real system evidence, namely
journald entries, systemd unit state, process state and task results. It shows the
evidence and the permissions involved, and never offers a fabricated explanation.

## Out of scope

Training the Amity models. The runtime is built so they can be added later.
