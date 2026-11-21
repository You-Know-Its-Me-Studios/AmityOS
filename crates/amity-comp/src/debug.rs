//! Development-only introspection for tests and debugging.
//!
//! When `AMITY_COMP_DEBUG_SOCKET` is set, the compositor answers
//! `GET /state` on a Unix socket in the user's runtime directory with a JSON
//! description of its windows, outputs and focus:
//!
//! ```text
//! curl --unix-socket "$XDG_RUNTIME_DIR/amity-comp-debug.sock" http://localhost/state
//! ```
//!
//! The variable is set only in the development image. The socket is readable
//! by its owner only and offers no actions. A request is answered on the
//! compositor's own thread, which it can hold up for a few tens of
//! milliseconds: another reason this is not for release images. When the
//! variable names a path, put it in a directory only the user can enter.

use std::{
    io::{self, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    time::Duration,
};

use amity_ipc::{SwitcherState, WindowInfo};
use serde::Serialize;
use smithay::{
    desktop::space::SpaceElement,
    reexports::calloop::{generic::Generic, Interest, Mode, PostAction},
};
use tracing::{info, warn};

use crate::{
    focus::KeyboardFocusTarget,
    layout::Tiling,
    state::{AmityState, Backend},
};

pub const DEBUG_SOCKET_ENV: &str = "AMITY_COMP_DEBUG_SOCKET";
const DEBUG_SOCKET_NAME: &str = "amity-comp-debug.sock";

#[derive(Serialize)]
struct DebugWindow {
    #[serde(flatten)]
    info: WindowInfo,
    tiling: &'static str,
    /// Outer rectangle (title bar included) in global coordinates; absent while minimized.
    geometry: Option<[i32; 4]>,
    server_side_decoration: bool,
    x11: bool,
}

#[derive(Serialize)]
struct DebugOutput {
    name: String,
    geometry: [i32; 4],
    work_area: [i32; 4],
    scale: f64,
    /// Id of the fullscreen window that covers this output right now, hiding
    /// the taskbar and the other windows.
    covered_by: Option<u64>,
}

#[derive(Serialize)]
struct DebugState {
    /// Managed windows, most recently used first.
    windows: Vec<DebugWindow>,
    active_window: Option<u64>,
    keyboard_focus: String,
    outputs: Vec<DebugOutput>,
    switcher: Option<SwitcherState>,
    shell_running: bool,
    wayland_display: Option<String>,
    x11_display: Option<u32>,
}

fn socket_path() -> Option<PathBuf> {
    let configured = std::env::var_os(DEBUG_SOCKET_ENV)?;
    let configured = PathBuf::from(configured);
    if configured.is_absolute() {
        return Some(configured);
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(PathBuf::from(runtime).join(DEBUG_SOCKET_NAME))
}

impl<BackendData: Backend + 'static> AmityState<BackendData> {
    /// Starts the debug endpoint when the environment asks for it.
    pub fn init_debug_socket(&mut self) {
        let Some(path) = socket_path() else {
            return;
        };
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path).and_then(|listener| {
            listener.set_nonblocking(true)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            Ok(listener)
        }) {
            Ok(listener) => listener,
            Err(err) => {
                warn!(path = %path.display(), %err, "could not open the debug socket");
                return;
            }
        };
        let inserted = self.handle.insert_source(
            Generic::new(listener, Interest::READ, Mode::Level),
            |_, listener, data| {
                loop {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if let Err(err) = data.debug_respond(stream) {
                                warn!(%err, "debug request failed");
                            }
                        }
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                        Err(err) => {
                            warn!(%err, "debug socket failed");
                            break;
                        }
                    }
                }
                Ok(PostAction::Continue)
            },
        );
        match inserted {
            Ok(_) => info!(path = %path.display(), "debug socket enabled"),
            Err(err) => warn!(%err, "could not register the debug socket"),
        }
    }

    /// Answers one request. This blocks the compositor for the duration of a
    /// local read and write, which is acceptable for a development tool.
    fn debug_respond(&self, mut stream: UnixStream) -> io::Result<()> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_millis(50)))?;
        stream.set_write_timeout(Some(Duration::from_millis(50)))?;
        let mut request = [0u8; 1024];
        let read = stream.read(&mut request)?;
        let request = String::from_utf8_lossy(&request[..read]);
        let mut words = request.split_whitespace();
        let (method, target) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
        let (status, body) = match (method, target) {
            ("GET", "/state") => (
                "200 OK",
                serde_json::to_string(&self.debug_state()).unwrap_or_else(|_| "{}".into()),
            ),
            _ => ("404 Not Found", "{\"error\":\"unknown request\"}".to_string()),
        };
        write!(
            stream,
            "HTTP/1.0 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        stream.flush()
    }

    fn debug_state(&self) -> DebugState {
        let rectangle = |r: smithay::utils::Rectangle<i32, smithay::utils::Logical>| {
            [r.loc.x, r.loc.y, r.size.w, r.size.h]
        };
        let (infos, active_window) = self.window_list();
        let windows = infos
            .into_iter()
            .filter_map(|info| {
                let window = self.window_by_id(info.id)?;
                Some(DebugWindow {
                    tiling: match window.tiling() {
                        Tiling::Floating => "floating",
                        Tiling::Maximized => "maximized",
                        Tiling::Left => "left",
                        Tiling::Right => "right",
                    },
                    geometry: self.space.element_location(&window).map(|location| {
                        let size = window.geometry().size;
                        [location.x, location.y, size.w, size.h]
                    }),
                    server_side_decoration: window.header_height() > 0,
                    #[cfg(feature = "xwayland")]
                    x11: window.is_x11(),
                    #[cfg(not(feature = "xwayland"))]
                    x11: false,
                    info,
                })
            })
            .collect();
        let keyboard_focus = match self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
        {
            Some(KeyboardFocusTarget::Window(window)) => self
                .wm
                .windows()
                .find(|candidate| candidate.0 == window)
                .and_then(|candidate| candidate.id())
                .map(|id| format!("window:{id}"))
                .unwrap_or_else(|| "window:unmanaged".into()),
            Some(KeyboardFocusTarget::LayerSurface(layer)) => format!("layer:{}", layer.namespace()),
            Some(KeyboardFocusTarget::Popup(_)) => "popup".into(),
            None => "none".into(),
        };
        let outputs = self
            .space
            .outputs()
            .map(|output| DebugOutput {
                name: output.name(),
                geometry: rectangle(self.space.output_geometry(output).unwrap_or_default()),
                work_area: rectangle(self.work_area(output)),
                scale: output.current_scale().fractional_scale(),
                covered_by: output
                    .user_data()
                    .get::<crate::shell::FullscreenSurface>()
                    .and_then(|slot| slot.covering())
                    .and_then(|window| window.id()),
            })
            .collect();
        DebugState {
            windows,
            active_window,
            keyboard_focus,
            outputs,
            switcher: self.wm.switcher().cloned(),
            shell_running: self.shell.is_running(),
            wayland_display: self.socket_name.clone(),
            #[cfg(feature = "xwayland")]
            x11_display: self.xdisplay,
            #[cfg(not(feature = "xwayland"))]
            x11_display: None,
        }
    }
}
