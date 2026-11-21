//! Message types for the private channels between AmityOS components.
//!
//! # Shell channel (version 1)
//!
//! `amity-comp` starts `amity-shell` and hands it one end of a connected Unix
//! socket pair; the file descriptor number is in the [`SHELL_FD_ENV`]
//! environment variable. There is no listening socket, so no other process can
//! reach the channel. Each message is one line of JSON terminated by `\n`.
//!
//! Both sides send `Hello` first. A message type the receiver does not know
//! decodes as `Unknown` and is ignored, so either side can be newer.

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Version of the shell channel protocol described in this crate.
pub const SHELL_PROTOCOL_VERSION: u32 = 1;

/// Environment variable holding the shell channel's file descriptor number.
pub const SHELL_FD_ENV: &str = "AMITY_SHELL_FD";

/// Longest accepted message line. Longer input is treated as a protocol error.
pub const MAX_LINE_BYTES: usize = 1 << 20;

/// Longest window title or application id in a message, in bytes of UTF-8.
/// Both come from applications, so the sender bounds them ([`bounded_text`]).
pub const MAX_TEXT_BYTES: usize = 127;

/// Most windows listed in one `Windows` message; the most recently used ones
/// are kept. Together with [`MAX_TEXT_BYTES`] this keeps every message well
/// under [`MAX_LINE_BYTES`], whatever applications do.
pub const MAX_WINDOWS: usize = 256;

/// Makes application-supplied text safe to show and bounded in size: control
/// characters become spaces and the text is cut at [`MAX_TEXT_BYTES`] on a
/// character boundary.
pub fn bounded_text(text: &str) -> String {
    let mut bounded = String::with_capacity(text.len().min(MAX_TEXT_BYTES));
    for character in text.chars() {
        let character = if character.is_control() { ' ' } else { character };
        if bounded.len() + character.len_utf8() > MAX_TEXT_BYTES {
            break;
        }
        bounded.push(character);
    }
    bounded
}

/// One managed top-level window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    /// Stable for the life of the window; never reused within a session.
    pub id: u64,
    /// At most [`MAX_TEXT_BYTES`] bytes, without control characters.
    pub title: String,
    /// Application identity: the Wayland `app_id` or the X11 window class.
    /// At most [`MAX_TEXT_BYTES`] bytes, without control characters.
    pub app_id: String,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    /// Name of the output the window is on, when it is on one.
    pub output: Option<String>,
}

/// State of the most-recently-used window switcher (Alt+Tab).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitcherState {
    /// Window ids in most-recently-used order, as captured when it opened.
    pub candidates: Vec<u64>,
    /// Index into `candidates`.
    pub selected: usize,
}

/// Messages from the compositor to the shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompositorMessage {
    Hello {
        protocol: u32,
    },
    /// The complete window list in most-recently-used order. Sent after
    /// `Hello` and whenever anything in it changes.
    Windows {
        windows: Vec<WindowInfo>,
        focused: Option<u64>,
    },
    /// The switcher opened, moved (`Some`) or closed (`None`).
    Switcher {
        state: Option<SwitcherState>,
    },
    /// The launcher shortcut was pressed.
    ToggleLauncher,
    /// Close transient shell surfaces such as the launcher.
    Dismiss,
    #[serde(other)]
    Unknown,
}

/// Messages from the shell to the compositor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShellMessage {
    Hello {
        protocol: u32,
    },
    /// Focus a window: restore it if minimized and raise it.
    Activate {
        id: u64,
    },
    Minimize {
        id: u64,
    },
    /// Ask the application to close a window; it may refuse.
    Close {
        id: u64,
    },
    #[serde(other)]
    Unknown,
}

/// Encodes a message as one line, including the trailing newline.
pub fn encode<T: Serialize>(message: &T) -> Vec<u8> {
    let mut line = serde_json::to_vec(message).expect("channel messages always serialize");
    line.push(b'\n');
    line
}

/// Decodes one line (without its newline).
pub fn decode<T: DeserializeOwned>(line: &[u8]) -> Result<T, serde_json::Error> {
    serde_json::from_slice(line)
}

/// The peer sent a line longer than [`MAX_LINE_BYTES`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineTooLong;

impl std::fmt::Display for LineTooLong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "channel line exceeds {MAX_LINE_BYTES} bytes")
    }
}

impl std::error::Error for LineTooLong {}

/// Reassembles lines from the byte chunks a socket delivers.
#[derive(Debug, Default)]
pub struct LineBuffer {
    pending: Vec<u8>,
}

impl LineBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds received bytes.
    pub fn push(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
    }

    /// Returns the next complete line without its newline, if one has arrived.
    pub fn next_line(&mut self) -> Result<Option<Vec<u8>>, LineTooLong> {
        match self.pending.iter().position(|&byte| byte == b'\n') {
            Some(end) if end > MAX_LINE_BYTES => Err(LineTooLong),
            Some(end) => {
                let mut line: Vec<u8> = self.pending.drain(..=end).collect();
                line.pop();
                Ok(Some(line))
            }
            None if self.pending.len() > MAX_LINE_BYTES => Err(LineTooLong),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: u64) -> WindowInfo {
        WindowInfo {
            id,
            title: format!("Window {id}"),
            app_id: "org.example.App".into(),
            minimized: false,
            maximized: id % 2 == 0,
            fullscreen: false,
            output: Some("Virtual-1".into()),
        }
    }

    #[test]
    fn messages_round_trip_as_single_lines() {
        let messages = [
            CompositorMessage::Hello {
                protocol: SHELL_PROTOCOL_VERSION,
            },
            CompositorMessage::Windows {
                windows: vec![window(1), window(2)],
                focused: Some(2),
            },
            CompositorMessage::Switcher {
                state: Some(SwitcherState {
                    candidates: vec![2, 1],
                    selected: 1,
                }),
            },
            CompositorMessage::Switcher { state: None },
            CompositorMessage::ToggleLauncher,
            CompositorMessage::Dismiss,
        ];
        for message in messages {
            let line = encode(&message);
            assert_eq!(line.last(), Some(&b'\n'));
            assert_eq!(line.iter().filter(|&&byte| byte == b'\n').count(), 1);
            let decoded: CompositorMessage = decode(&line[..line.len() - 1]).unwrap();
            assert_eq!(decoded, message);
        }
        for message in [
            ShellMessage::Hello {
                protocol: SHELL_PROTOCOL_VERSION,
            },
            ShellMessage::Activate { id: 7 },
            ShellMessage::Minimize { id: 7 },
            ShellMessage::Close { id: 7 },
        ] {
            let line = encode(&message);
            let decoded: ShellMessage = decode(&line[..line.len() - 1]).unwrap();
            assert_eq!(decoded, message);
        }
    }

    #[test]
    fn a_title_with_a_newline_stays_on_one_line() {
        let mut info = window(3);
        info.title = "first\nsecond \"quoted\"".into();
        let message = CompositorMessage::Windows {
            windows: vec![info],
            focused: None,
        };
        let line = encode(&message);
        assert_eq!(line.iter().filter(|&&byte| byte == b'\n').count(), 1);
        let decoded: CompositorMessage = decode(&line[..line.len() - 1]).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn application_text_is_bounded_and_printable() {
        assert_eq!(bounded_text("plain title"), "plain title");
        assert_eq!(
            bounded_text("tab\there\nnewline\u{7}bell"),
            "tab here newline bell"
        );
        assert_eq!(bounded_text(&"x".repeat(500)).len(), MAX_TEXT_BYTES);
        // Cut on a character boundary: "é" is two bytes, so 63 of them fit in 127.
        let accented = bounded_text(&"é".repeat(200));
        assert_eq!(accented.chars().count(), 63);
        assert!(accented.len() <= MAX_TEXT_BYTES);
        assert_eq!(bounded_text(""), "");
    }

    #[test]
    fn the_largest_window_list_fits_in_one_line() {
        // Quotes and backslashes double in JSON; they are the worst case after bounding.
        let worst = bounded_text(&"\"".repeat(MAX_TEXT_BYTES));
        let windows: Vec<WindowInfo> = (0..MAX_WINDOWS as u64)
            .map(|id| WindowInfo {
                id: u64::MAX - id,
                title: worst.clone(),
                app_id: worst.clone(),
                minimized: false,
                maximized: false,
                fullscreen: false,
                output: Some(worst.clone()),
            })
            .collect();
        let line = encode(&CompositorMessage::Windows {
            windows,
            focused: Some(u64::MAX),
        });
        assert!(line.len() < MAX_LINE_BYTES / 2, "{} bytes", line.len());
    }

    #[test]
    fn unknown_message_types_are_tolerated() {
        let decoded: CompositorMessage = decode(br#"{"type":"from_the_future","value":1}"#).unwrap();
        assert_eq!(decoded, CompositorMessage::Unknown);
        let decoded: ShellMessage = decode(br#"{"type":"from_the_future"}"#).unwrap();
        assert_eq!(decoded, ShellMessage::Unknown);
        assert!(decode::<ShellMessage>(b"not json").is_err());
    }

    #[test]
    fn line_buffer_reassembles_split_and_joined_chunks() {
        let first = encode(&ShellMessage::Activate { id: 1 });
        let second = encode(&ShellMessage::Close { id: 2 });
        let mut stream = first.clone();
        stream.extend_from_slice(&second);

        let mut buffer = LineBuffer::new();
        let (head, tail) = stream.split_at(first.len() + 3);
        buffer.push(&head[..5]);
        assert_eq!(buffer.next_line().unwrap(), None);
        buffer.push(&head[5..]);
        let line = buffer.next_line().unwrap().unwrap();
        assert_eq!(
            decode::<ShellMessage>(&line).unwrap(),
            ShellMessage::Activate { id: 1 }
        );
        assert_eq!(buffer.next_line().unwrap(), None);
        buffer.push(tail);
        let line = buffer.next_line().unwrap().unwrap();
        assert_eq!(
            decode::<ShellMessage>(&line).unwrap(),
            ShellMessage::Close { id: 2 }
        );
        assert_eq!(buffer.next_line().unwrap(), None);
    }

    #[test]
    fn line_buffer_rejects_oversized_lines() {
        let mut buffer = LineBuffer::new();
        buffer.push(&vec![b'x'; MAX_LINE_BYTES + 1]);
        assert_eq!(buffer.next_line(), Err(LineTooLong));
    }
}
