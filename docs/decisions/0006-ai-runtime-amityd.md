# 0006: amityd, the Amity AI runtime

Status: accepted (default from the owner's brief).

## Decision

`crates/amityd` is a Rust system daemon (a systemd service).

- **Providers:** a `ModelProvider` trait (chat, completion, tool calls, embeddings, model
  management) so that future Amity models plug in without changing callers. There are
  two backends:
  - **Local:** llama.cpp through a maintained Rust binding, running CPU, CUDA, ROCm or
    Vulkan builds. ONNX Runtime is the alternative if the binding is unmaintained when
    pinned.
  - **Remote:** an API stub, off by default.

  The default model is a small open model. It is labelled as a placeholder everywhere,
  including in Settings (screenshot 07 shows "[MODEL NAME]"). No Amity model capability
  is claimed or simulated.
- **APIs:** a versioned D-Bus API and a local Unix socket API.
- **Context and privacy:** context reaches the model only through explicit per-Space
  grants. The grantable sources are files in the Space, terminal output, the focused
  window title, task results and diagnostics. Settings controls them as in screenshots
  07 and 34 ("Allowed / Ask", "Remember project context", "Use clipboard and screen").
  Nothing leaves the machine unless the user enables a remote provider.
- **Actions:** commands, commits and installs show the exact action and wait for approval
  (design principle "Ask before acting", screenshot 04). The agent states (Ready,
  Working, Waiting on you, Stopped) appear in the taskbar tray.
- **GPU memory:** the daemon keeps a VRAM budget and yields to user applications. It
  unloads or offloads models before a game engine or renderer fails to allocate.
- **Consumers:** the shell assistant surface, "Explain this" (over journald, systemd unit
  state and process state), a VS Code extension (built first), and stubs for the
  Unity, Unreal and Blender plugins.

## Licences

| Component | Licence | Linked? |
|---|---|---|
| llama.cpp | MIT | yes (static or dynamic) |
| Rust binding (chosen when pinned) | must be MIT/Apache-2.0 | yes |
| ONNX Runtime (alternative) | MIT | yes |
| Placeholder model weights | per model; must allow commercial redistribution or be downloaded at first run | data |
