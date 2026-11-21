//! The compositor's side of the shell: starts `amity-shell`, keeps it running,
//! and exchanges messages with it over a private socket (see `amity-ipc`).
//!
//! The shell is the only client that receives a privileged Wayland connection
//! (for layer-shell surfaces such as the taskbar) and the shell channel. Both
//! are connected sockets handed to the child process, so no other client can
//! obtain them.

use std::{
    collections::VecDeque,
    ffi::OsString,
    io::{self, Read, Write},
    os::unix::{
        io::{AsRawFd, BorrowedFd},
        net::UnixStream,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

use amity_ipc::{
    CompositorMessage, LineBuffer, ShellMessage, SwitcherState, WindowInfo, SHELL_FD_ENV,
    SHELL_PROTOCOL_VERSION,
};
use smithay::reexports::{
    calloop::{
        generic::Generic,
        timer::{TimeoutAction, Timer},
        Interest, Mode, PostAction,
    },
    rustix::io::{fcntl_getfd, fcntl_setfd, FdFlags},
};
use tracing::{debug, error, info, warn};

use crate::state::{AmityState, Backend, ClientState};

/// Unsent bytes after which the shell is considered hung and is restarted.
const OUTBOX_LIMIT: usize = 4 << 20;
/// A shell that exits this often within [`RESTART_WINDOW`] is not restarted again.
const RESTART_LIMIT: usize = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(60);
const RESTART_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Default)]
pub struct ShellLink {
    child: Option<Child>,
    stream: Option<UnixStream>,
    inbox: LineBuffer,
    outbox: VecDeque<u8>,
    /// What the shell was last told, to send changes only.
    sent_windows: Option<(Vec<WindowInfo>, Option<u64>)>,
    sent_switcher: Option<Option<SwitcherState>>,
    exits: Vec<Instant>,
    /// Incremented for every started shell; stale event sources compare against it.
    generation: u64,
}

impl ShellLink {
    /// True while a shell process is connected.
    pub fn is_running(&self) -> bool {
        self.stream.is_some()
    }
}

/// The shell program: `AMITY_SHELL` when set (empty disables the shell),
/// otherwise `amity-shell` next to this executable, otherwise from `PATH`.
fn shell_program() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os("AMITY_SHELL") {
        return (!configured.is_empty()).then(|| PathBuf::from(configured));
    }
    let sibling = std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name("amity-shell"))
        .filter(|path| path.exists());
    Some(sibling.unwrap_or_else(|| PathBuf::from("amity-shell")))
}

/// Keeps a file descriptor open across `exec` in the child process.
fn inherit(fd: BorrowedFd<'_>) -> io::Result<()> {
    let flags = fcntl_getfd(fd)?;
    fcntl_setfd(fd, flags.difference(FdFlags::CLOEXEC))?;
    Ok(())
}

impl<BackendData: Backend + 'static> AmityState<BackendData> {
    /// Environment for processes that should open windows on this compositor.
    pub fn client_environment(&self) -> Vec<(OsString, Option<OsString>)> {
        let mut environment = vec![
            (
                OsString::from("WAYLAND_DISPLAY"),
                self.socket_name.clone().map(OsString::from),
            ),
            (
                OsString::from("XDG_SESSION_TYPE"),
                Some(OsString::from("wayland")),
            ),
            (
                OsString::from("XDG_CURRENT_DESKTOP"),
                Some(OsString::from("AmityOS")),
            ),
        ];
        #[cfg(feature = "xwayland")]
        environment.push((
            OsString::from("DISPLAY"),
            self.xdisplay.map(|display| OsString::from(format!(":{display}"))),
        ));
        #[cfg(not(feature = "xwayland"))]
        environment.push((OsString::from("DISPLAY"), None));
        environment
    }

    /// Starts the shell process. Does nothing when it is already running or disabled.
    pub fn start_shell(&mut self) {
        if self.shell.child.is_some() {
            return;
        }
        let Some(program) = shell_program() else {
            info!("AMITY_SHELL is empty; running without a shell");
            return;
        };
        match self.spawn_shell(&program) {
            Ok(()) => info!(program = %program.display(), "started the shell"),
            Err(err) => {
                error!(program = %program.display(), %err, "failed to start the shell");
                self.shell_stopped();
            }
        }
    }

    fn spawn_shell(&mut self, program: &Path) -> io::Result<()> {
        let (channel, child_channel) = UnixStream::pair()?;
        let (wayland, child_wayland) = UnixStream::pair()?;
        channel.set_nonblocking(true)?;

        let mut command = Command::new(program);
        command.stdin(Stdio::null());
        for (name, value) in self.client_environment() {
            match value {
                Some(value) => command.env(name, value),
                None => command.env_remove(name),
            };
        }
        command.env(SHELL_FD_ENV, child_channel.as_raw_fd().to_string());
        command.env("WAYLAND_SOCKET", child_wayland.as_raw_fd().to_string());
        let (channel_fd, wayland_fd) = (child_channel.as_raw_fd(), child_wayland.as_raw_fd());
        // Safety: the closure only calls fcntl, which is async-signal-safe, on
        // descriptors that stay open until after the spawn.
        unsafe {
            command.pre_exec(move || {
                inherit(BorrowedFd::borrow_raw(channel_fd))?;
                inherit(BorrowedFd::borrow_raw(wayland_fd))
            });
        }
        let child = command.spawn()?;
        // The child has its copies; closing ours lets us see it exit.
        drop(child_channel);
        drop(child_wayland);

        self.display_handle.insert_client(
            wayland,
            Arc::new(ClientState {
                privileged: true,
                ..ClientState::default()
            }),
        )?;

        self.shell.generation += 1;
        let generation = self.shell.generation;
        let reader = channel.try_clone()?;
        self.handle
            .insert_source(
                Generic::new(reader, Interest::READ, Mode::Level),
                move |_, stream, data| {
                    if data.shell.generation != generation {
                        return Ok(PostAction::Remove);
                    }
                    let mut stream: &UnixStream = stream;
                    let mut chunk = [0u8; 4096];
                    loop {
                        match stream.read(&mut chunk) {
                            Ok(0) => {
                                data.shell_stopped();
                                return Ok(PostAction::Remove);
                            }
                            Ok(read) => data.shell.inbox.push(&chunk[..read]),
                            Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                            Err(err) => {
                                warn!(%err, "shell channel failed");
                                data.shell_stopped();
                                return Ok(PostAction::Remove);
                            }
                        }
                    }
                    if !data.shell_dispatch() {
                        data.shell_stopped();
                        return Ok(PostAction::Remove);
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|err| io::Error::other(err.to_string()))?;

        self.shell.child = Some(child);
        self.shell.stream = Some(channel);
        self.shell.inbox = LineBuffer::new();
        self.shell.outbox.clear();
        self.shell.sent_windows = None;
        self.shell.sent_switcher = None;
        self.shell_send(&CompositorMessage::Hello {
            protocol: SHELL_PROTOCOL_VERSION,
        });
        Ok(())
    }

    /// Handles complete messages from the shell. Returns false on a protocol error.
    fn shell_dispatch(&mut self) -> bool {
        loop {
            let line = match self.shell.inbox.next_line() {
                Ok(Some(line)) => line,
                Ok(None) => return true,
                Err(err) => {
                    warn!(%err, "shell channel protocol error");
                    return false;
                }
            };
            match amity_ipc::decode::<ShellMessage>(&line) {
                Ok(message) => self.shell_message(message),
                Err(err) => warn!(%err, "ignoring a malformed shell message"),
            }
        }
    }

    fn shell_message(&mut self, message: ShellMessage) {
        debug!(?message, "from shell");
        match message {
            ShellMessage::Hello { protocol } => {
                if protocol != SHELL_PROTOCOL_VERSION {
                    warn!(
                        shell = protocol,
                        compositor = SHELL_PROTOCOL_VERSION,
                        "shell speaks a different channel protocol version"
                    );
                }
            }
            ShellMessage::Activate { id } => {
                if let Some(window) = self.window_by_id(id) {
                    self.focus_window(&window);
                }
            }
            ShellMessage::Minimize { id } => {
                if let Some(window) = self.window_by_id(id) {
                    self.minimize_window(&window);
                }
            }
            ShellMessage::Close { id } => {
                if let Some(window) = self.window_by_id(id) {
                    self.close_window(&window);
                }
            }
            ShellMessage::Unknown => {}
        }
    }

    /// Queues a message for the shell; dropped when no shell is connected.
    pub fn shell_send(&mut self, message: &CompositorMessage) {
        if self.shell.stream.is_none() {
            return;
        }
        self.shell.outbox.extend(amity_ipc::encode(message));
        self.shell_flush();
    }

    fn shell_flush(&mut self) {
        let Some(mut stream) = self.shell.stream.as_ref() else {
            return;
        };
        while !self.shell.outbox.is_empty() {
            let (front, _) = self.shell.outbox.as_slices();
            match stream.write(front) {
                Ok(0) => break,
                Ok(written) => {
                    self.shell.outbox.drain(..written);
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => {
                    warn!(%err, "writing to the shell failed");
                    self.shell.outbox.clear();
                    break;
                }
            }
        }
        if self.shell.outbox.len() > OUTBOX_LIMIT {
            warn!("the shell is not reading its channel; restarting it");
            if let Some(child) = self.shell.child.as_mut() {
                let _ = child.kill();
            }
            self.shell.outbox.clear();
        }
    }

    /// Sends the shell whatever changed since the last call. Runs once per
    /// event-loop iteration.
    pub fn shell_sync(&mut self) {
        if self.shell.stream.is_none() {
            return;
        }
        let windows = self.window_list();
        if self.shell.sent_windows.as_ref() != Some(&windows) {
            self.shell_send(&CompositorMessage::Windows {
                windows: windows.0.clone(),
                focused: windows.1,
            });
            self.shell.sent_windows = Some(windows);
        }
        let switcher = self.wm.switcher().cloned();
        if self.shell.sent_switcher.as_ref() != Some(&switcher) {
            self.shell_send(&CompositorMessage::Switcher {
                state: switcher.clone(),
            });
            self.shell.sent_switcher = Some(switcher);
        }
        self.shell_flush();
    }

    /// The shell went away: clean up and restart it unless it keeps failing.
    fn shell_stopped(&mut self) {
        self.shell.stream = None;
        self.shell.outbox.clear();
        // Invalidate the event source of the stopped shell, if it is still registered.
        self.shell.generation += 1;
        if let Some(mut child) = self.shell.child.take() {
            // The channel closes a moment before the process can be reaped; wait
            // briefly so that the log has the real exit status.
            let mut status = child.try_wait().ok().flatten();
            for _ in 0..10 {
                if status.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
                status = child.try_wait().ok().flatten();
            }
            match status {
                Some(status) => warn!(%status, "the shell exited"),
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    warn!("the shell closed its channel without exiting and was stopped");
                }
            }
        }
        if !self.running.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let now = Instant::now();
        self.shell
            .exits
            .retain(|exit| now.duration_since(*exit) < RESTART_WINDOW);
        self.shell.exits.push(now);
        if self.shell.exits.len() > RESTART_LIMIT {
            error!("the shell keeps exiting; not restarting it again");
            return;
        }
        let scheduled = self
            .handle
            .insert_source(Timer::from_duration(RESTART_DELAY), |_, _, data| {
                data.start_shell();
                TimeoutAction::Drop
            });
        if let Err(err) = scheduled {
            error!(%err, "could not schedule a shell restart");
        }
    }

    /// Stops the shell when the compositor exits.
    pub fn stop_shell(&mut self) {
        self.shell.stream = None;
        self.shell.generation += 1;
        if let Some(mut child) = self.shell.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
