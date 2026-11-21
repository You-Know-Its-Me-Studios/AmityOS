//! The shell's end of the private channel to the compositor (see `amity-ipc`).
//!
//! The compositor starts the shell with a connected socket whose descriptor
//! number is in `AMITY_SHELL_FD`. Started by hand, without that variable, the
//! shell runs unconnected: it shows its surfaces but has no window list.

use std::{
    io::{Read, Write},
    os::{
        fd::{FromRawFd, OwnedFd},
        unix::net::UnixStream,
    },
    sync::{Mutex, OnceLock},
    time::Duration,
};

use amity_ipc::{CompositorMessage, LineBuffer, ShellMessage, SHELL_FD_ENV, SHELL_PROTOCOL_VERSION};
use iced::futures::{channel::mpsc, SinkExt, Stream, StreamExt};

/// What arrives from the compositor.
#[derive(Debug, Clone)]
pub enum Event {
    Message(CompositorMessage),
    /// The compositor closed the channel or broke the protocol.
    Closed,
}

static WRITER: OnceLock<Mutex<UnixStream>> = OnceLock::new();
static READER: Mutex<Option<UnixStream>> = Mutex::new(None);

/// Takes ownership of the descriptor the compositor passed.
fn take_inherited() -> Option<UnixStream> {
    let value = std::env::var(SHELL_FD_ENV).ok()?;
    // Applications started by the shell must not see the variable.
    std::env::remove_var(SHELL_FD_ENV);
    let descriptor: i32 = value.parse().ok().filter(|descriptor| *descriptor > 2)?;
    // Safety: the compositor opened this descriptor for the shell and nothing
    // else in this process uses it.
    let inherited = unsafe { OwnedFd::from_raw_fd(descriptor) };
    // The duplicate is close-on-exec, unlike the inherited descriptor, so
    // applications the shell launches cannot inherit the channel.
    let private = inherited.try_clone().ok()?;
    Some(UnixStream::from(private))
}

/// Connects to the compositor if it provided a channel. Returns whether it did.
/// Must run before any thread is started.
pub fn init() -> bool {
    let Some(stream) = take_inherited() else {
        return false;
    };
    let Ok(reader) = stream.try_clone() else {
        return false;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    *READER.lock().unwrap() = Some(reader);
    let _ = WRITER.set(Mutex::new(stream));
    send(&ShellMessage::Hello {
        protocol: SHELL_PROTOCOL_VERSION,
    });
    true
}

/// Sends a message to the compositor. Does nothing when unconnected.
pub fn send(message: &ShellMessage) {
    let Some(writer) = WRITER.get() else {
        return;
    };
    let mut stream = writer.lock().unwrap();
    if let Err(err) = stream.write_all(&amity_ipc::encode(message)) {
        eprintln!("amity-shell: could not send to the compositor: {err}");
    }
}

fn read_loop(mut reader: UnixStream, sender: mpsc::UnboundedSender<Event>) {
    let mut buffer = LineBuffer::new();
    let mut chunk = [0u8; 4096];
    'connection: loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => buffer.push(&chunk[..read]),
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
        loop {
            match buffer.next_line() {
                Ok(Some(line)) => match amity_ipc::decode::<CompositorMessage>(&line) {
                    Ok(message) => {
                        if sender.unbounded_send(Event::Message(message)).is_err() {
                            return;
                        }
                    }
                    Err(err) => eprintln!("amity-shell: ignoring a malformed compositor message: {err}"),
                },
                Ok(None) => break,
                Err(err) => {
                    eprintln!("amity-shell: {err}");
                    break 'connection;
                }
            }
        }
    }
    let _ = sender.unbounded_send(Event::Closed);
}

/// The stream of compositor events, for an iced subscription. Yields nothing
/// when the shell is unconnected.
pub fn events() -> impl Stream<Item = Event> {
    iced::stream::channel(64, |mut output: mpsc::Sender<Event>| async move {
        let reader = READER.lock().unwrap().take();
        if let Some(reader) = reader {
            let (sender, mut receiver) = mpsc::unbounded();
            let spawned = std::thread::Builder::new()
                .name("compositor-channel".into())
                .spawn(move || read_loop(reader, sender));
            if spawned.is_ok() {
                while let Some(event) = receiver.next().await {
                    if output.send(event).await.is_err() {
                        break;
                    }
                }
            }
        }
        // A subscription stream is expected to stay alive.
        std::future::pending::<()>().await;
    })
}
