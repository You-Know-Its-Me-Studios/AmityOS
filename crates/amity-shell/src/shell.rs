//! Shell state and behaviour.

use std::path::PathBuf;

use amity_apps::{DesktopEntry, Launcher};
use amity_ipc::{CompositorMessage, ShellMessage, SwitcherState, WindowInfo};
use amity_ui::{kit, tokens};
use chrono::{DateTime, Local};
use iced::{
    event,
    futures::{channel::mpsc, SinkExt, Stream, StreamExt},
    keyboard::{self, key::Named, Key},
    widget::image,
    window, Color, Element, Subscription, Task, Theme,
};
use iced_layershell::{
    reexport::{Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption},
    to_layer_message,
};

use crate::{channel, view};

/// Height of the taskbar in logical pixels.
pub const TASKBAR_HEIGHT: u32 = 62;
/// Gap between the taskbar and the left and right screen edges.
pub const TASKBAR_SIDE_MARGIN: i32 = 16;
/// Gap between the taskbar and the bottom screen edge.
pub const TASKBAR_BOTTOM_MARGIN: i32 = 14;
/// Launcher results shown at once (shell-behaviour.md §3.4).
pub const LAUNCHER_ROWS: usize = 8;
/// Longest launcher query, in characters (§3.3).
pub const QUERY_LIMIT: usize = 80;
/// Widget id of the launcher's search field.
pub const QUERY_ID: &str = "launcher-query";

const DEFAULT_WALLPAPER: &str = "/usr/share/amityos/wallpapers/amity-default.png";

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    /// Something arrived from the compositor.
    Channel(channel::Event),
    /// The minute changed.
    Tick,
    SurfaceOpened(window::Id),
    SurfaceClosed(window::Id),
    /// A taskbar entry was pressed.
    Task(u64),
    ToggleLauncher,
    CloseLauncher,
    Query(String),
    /// Enter in the search field.
    Submit,
    /// A launcher result was chosen, by its position in the result list.
    Activate(usize),
    /// A key press that no widget used, on the given surface.
    Key(window::Id, Key),
}

pub struct Shell {
    /// Whether the compositor gave us a channel.
    connected: bool,
    /// Windows in most-recently-used order, as the compositor reports them.
    pub windows: Vec<WindowInfo>,
    pub focused: Option<u64>,
    pub switcher: Option<SwitcherState>,

    taskbar: window::Id,
    launcher: Option<window::Id>,
    switcher_surface: Option<window::Id>,

    /// Installed applications, reloaded each time the launcher opens.
    pub catalogue: Vec<DesktopEntry>,
    pub query: String,
    /// Indices into `catalogue` matching `query`, best first.
    pub results: Vec<usize>,
    pub selected: usize,
    /// Why the last launch failed, shown in the launcher.
    pub launch_error: Option<String>,
    app_launcher: Launcher,

    pub now: DateTime<Local>,
    pub wallpaper: Option<image::Handle>,
}

/// What a press on a taskbar entry does (§2.6, one window per entry): the
/// focused window is minimized, any other is focused, which also restores it.
pub fn task_action(window: &WindowInfo, focused: Option<u64>) -> ShellMessage {
    if focused == Some(window.id) && !window.minimized {
        ShellMessage::Minimize { id: window.id }
    } else {
        ShellMessage::Activate { id: window.id }
    }
}

/// Taskbar order: by window id, which is creation order. Unlike the use
/// order, it does not change when focus moves, so entries stay put.
pub fn task_order(windows: &[WindowInfo]) -> Vec<&WindowInfo> {
    let mut ordered: Vec<&WindowInfo> = windows.iter().collect();
    ordered.sort_by_key(|window| window.id);
    ordered
}

/// Moves the launcher selection without wrapping (§3.3).
pub fn move_selection(selected: usize, count: usize, down: bool) -> usize {
    if count == 0 {
        0
    } else if down {
        (selected + 1).min(count - 1)
    } else {
        selected.saturating_sub(1).min(count - 1)
    }
}

/// First result shown when `selected` is highlighted: results scroll a page
/// at a time (§3.4).
pub fn page_start(selected: usize, rows: usize) -> usize {
    if rows == 0 {
        0
    } else {
        selected / rows * rows
    }
}

/// The name to show for a window's application: the installed application
/// with that desktop file ID or window class, otherwise a readable form of
/// the identifier.
pub fn application_name(app_id: &str, catalogue: &[DesktopEntry]) -> String {
    let wanted = app_id.to_lowercase();
    if !wanted.is_empty() {
        let known = catalogue.iter().find(|entry| {
            entry.id.to_lowercase() == wanted
                || entry
                    .startup_wm_class
                    .as_ref()
                    .is_some_and(|class| class.to_lowercase() == wanted)
        });
        if let Some(entry) = known {
            return entry.name.clone();
        }
    }
    // "org.example.TextEditor" -> "TextEditor", "foot" -> "Foot".
    let last = app_id.rsplit('.').next().unwrap_or(app_id).trim();
    let mut chars = last.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Application".to_owned(),
    }
}

/// Up to two initials for the tile that stands in for an application icon.
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let letters: String = match words.as_slice() {
        [] => "?".to_owned(),
        [single] => single.chars().take(2).collect(),
        [first, .., last] => first.chars().take(1).chain(last.chars().take(1)).collect(),
    };
    letters.to_uppercase()
}

/// Shortens text to `limit` characters, ending in an ellipsis when cut.
pub fn ellipsize(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        let mut cut: String = text.chars().take(limit.saturating_sub(1)).collect();
        cut.truncate(cut.trim_end().len());
        cut.push('…');
        cut
    }
}

fn wallpaper() -> Option<image::Handle> {
    let path = std::env::var_os("AMITY_WALLPAPER")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_WALLPAPER));
    path.is_file().then(|| image::Handle::from_path(path))
}

/// A tick at every minute boundary, for the clock.
fn minute_ticks() -> impl Stream<Item = Message> {
    iced::stream::channel(1, |mut output: mpsc::Sender<Message>| async move {
        let (sender, mut receiver) = mpsc::unbounded();
        let spawned = std::thread::Builder::new()
            .name("clock".into())
            .spawn(move || loop {
                let into_minute = u64::from(chrono::Timelike::second(&Local::now()));
                std::thread::sleep(std::time::Duration::from_secs(60 - into_minute.min(59)));
                if sender.unbounded_send(()).is_err() {
                    break;
                }
            });
        if spawned.is_ok() {
            while receiver.next().await.is_some() {
                if output.send(Message::Tick).await.is_err() {
                    break;
                }
            }
        }
        std::future::pending::<()>().await;
    })
}

fn surface_keys(event: iced::Event, status: event::Status, id: window::Id) -> Option<Message> {
    match event {
        iced::Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => match key {
            // Enter is handled by the search field when it has focus.
            Key::Named(Named::Enter) if status == event::Status::Captured => None,
            Key::Named(Named::Escape | Named::ArrowDown | Named::ArrowUp | Named::Enter) => {
                Some(Message::Key(id, key))
            }
            _ => None,
        },
        _ => None,
    }
}

impl Shell {
    pub fn boot(connected: bool) -> (Self, Task<Message>) {
        let taskbar = window::Id::unique();
        let shell = Shell {
            connected,
            windows: Vec::new(),
            focused: None,
            switcher: None,
            taskbar,
            launcher: None,
            switcher_surface: None,
            catalogue: amity_apps::installed_applications(),
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            launch_error: None,
            app_launcher: Launcher::detect(),
            now: Local::now(),
            wallpaper: wallpaper(),
        };
        let open_taskbar = Task::done(Message::NewLayerShell {
            settings: NewLayerShellSettings {
                size: Some((0, TASKBAR_HEIGHT)),
                layer: Layer::Top,
                anchor: Anchor::Bottom | Anchor::Left | Anchor::Right,
                exclusive_zone: Some(TASKBAR_HEIGHT as i32),
                margin: Some((0, TASKBAR_SIDE_MARGIN, TASKBAR_BOTTOM_MARGIN, TASKBAR_SIDE_MARGIN)),
                keyboard_interactivity: KeyboardInteractivity::None,
                output_option: OutputOption::Active,
                events_transparent: false,
                namespace: Some("amity-taskbar".to_owned()),
            },
            id: taskbar,
        });
        (shell, open_taskbar)
    }

    pub fn namespace() -> String {
        "amity-desktop".to_owned()
    }

    pub fn style(&self, _theme: &Theme) -> iced::theme::Style {
        iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: kit::color(tokens::text::BODY),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(channel::events).map(Message::Channel),
            Subscription::run(minute_ticks),
            window::open_events().map(Message::SurfaceOpened),
            window::close_events().map(Message::SurfaceClosed),
            event::listen_with(surface_keys),
        ])
    }

    pub fn view(&self, id: window::Id) -> Element<'_, Message> {
        if id == self.taskbar {
            view::taskbar(self)
        } else if Some(id) == self.launcher {
            view::launcher(self)
        } else if Some(id) == self.switcher_surface {
            view::switcher(self)
        } else {
            view::background(self)
        }
    }

    /// The application a result row stands for.
    pub fn result(&self, row: usize) -> Option<&DesktopEntry> {
        self.results.get(row).and_then(|&index| self.catalogue.get(index))
    }

    fn refresh_results(&mut self) {
        let matches = amity_apps::search(&self.catalogue, &self.query);
        self.results = matches
            .into_iter()
            .filter_map(|entry| {
                self.catalogue
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate, entry))
            })
            .collect();
        self.selected = 0;
    }

    fn open_launcher(&mut self) -> Task<Message> {
        if self.launcher.is_some() {
            return Task::none();
        }
        // Rescan on every open, so newly installed applications appear.
        self.catalogue = amity_apps::installed_applications();
        self.query.clear();
        self.launch_error = None;
        self.refresh_results();
        let id = window::Id::unique();
        self.launcher = Some(id);
        Task::done(Message::NewLayerShell {
            settings: NewLayerShellSettings {
                size: Some((0, 0)),
                layer: Layer::Overlay,
                anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
                exclusive_zone: Some(-1),
                margin: None,
                keyboard_interactivity: KeyboardInteractivity::Exclusive,
                output_option: OutputOption::Active,
                events_transparent: false,
                namespace: Some("amity-launcher".to_owned()),
            },
            id,
        })
    }

    fn close_launcher(&mut self) -> Task<Message> {
        match self.launcher.take() {
            Some(id) => Task::done(Message::RemoveWindow(id)),
            None => Task::none(),
        }
    }

    fn set_switcher(&mut self, state: Option<SwitcherState>) -> Task<Message> {
        self.switcher = state;
        match (&self.switcher, self.switcher_surface) {
            (Some(_), None) => {
                let id = window::Id::unique();
                self.switcher_surface = Some(id);
                Task::done(Message::NewLayerShell {
                    settings: NewLayerShellSettings {
                        size: Some((0, 0)),
                        layer: Layer::Overlay,
                        anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
                        exclusive_zone: Some(-1),
                        margin: None,
                        // The compositor keeps the keyboard while Alt is held.
                        keyboard_interactivity: KeyboardInteractivity::None,
                        output_option: OutputOption::Active,
                        events_transparent: true,
                        namespace: Some("amity-switcher".to_owned()),
                    },
                    id,
                })
            }
            (None, Some(id)) => {
                self.switcher_surface = None;
                Task::done(Message::RemoveWindow(id))
            }
            _ => Task::none(),
        }
    }

    fn activate(&mut self, row: usize) -> Task<Message> {
        let Some(entry) = self.result(row).cloned() else {
            return Task::none();
        };
        match self.app_launcher.launch(&entry, &[]) {
            Ok(_) => self.close_launcher(),
            Err(err) => {
                eprintln!("amity-shell: launching {} failed: {err}", entry.id);
                self.launch_error = Some(format!("{} could not be started: {err}", entry.name));
                Task::none()
            }
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Channel(channel::Event::Message(message)) => match message {
                CompositorMessage::Hello { protocol } => {
                    if protocol != amity_ipc::SHELL_PROTOCOL_VERSION {
                        eprintln!("amity-shell: the compositor speaks channel protocol {protocol}");
                    }
                    Task::none()
                }
                CompositorMessage::Windows { windows, focused } => {
                    self.windows = windows;
                    self.focused = focused;
                    Task::none()
                }
                CompositorMessage::Switcher { state } => self.set_switcher(state),
                CompositorMessage::ToggleLauncher => self.update(Message::ToggleLauncher),
                CompositorMessage::Dismiss => self.close_launcher(),
                CompositorMessage::Unknown => Task::none(),
            },
            Message::Channel(channel::Event::Closed) => {
                // The compositor is gone or is replacing the shell.
                if self.connected {
                    std::process::exit(0);
                }
                Task::none()
            }
            Message::Tick => {
                self.now = Local::now();
                Task::none()
            }
            Message::SurfaceOpened(id) => {
                if Some(id) == self.launcher {
                    iced::widget::operation::focus(QUERY_ID)
                } else {
                    Task::none()
                }
            }
            Message::SurfaceClosed(id) => {
                if Some(id) == self.launcher {
                    self.launcher = None;
                }
                if Some(id) == self.switcher_surface {
                    self.switcher_surface = None;
                }
                Task::none()
            }
            Message::Task(id) => {
                if let Some(window) = self.windows.iter().find(|window| window.id == id) {
                    channel::send(&task_action(window, self.focused));
                }
                Task::none()
            }
            Message::ToggleLauncher => {
                if self.launcher.is_some() {
                    self.close_launcher()
                } else {
                    self.open_launcher()
                }
            }
            Message::CloseLauncher => self.close_launcher(),
            Message::Query(query) => {
                self.query = query.chars().take(QUERY_LIMIT).collect();
                self.launch_error = None;
                self.refresh_results();
                Task::none()
            }
            Message::Submit => self.activate(self.selected),
            Message::Activate(row) => self.activate(row),
            Message::Key(id, key) => {
                if Some(id) != self.launcher {
                    return Task::none();
                }
                match key {
                    Key::Named(Named::Escape) => self.close_launcher(),
                    Key::Named(Named::ArrowDown) => {
                        self.selected = move_selection(self.selected, self.results.len(), true);
                        Task::none()
                    }
                    Key::Named(Named::ArrowUp) => {
                        self.selected = move_selection(self.selected, self.results.len(), false);
                        Task::none()
                    }
                    Key::Named(Named::Enter) => self.activate(self.selected),
                    _ => Task::none(),
                }
            }
            // Layer-shell requests are carried out by the runtime.
            _ => Task::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: u64, minimized: bool) -> WindowInfo {
        WindowInfo {
            id,
            title: format!("Window {id}"),
            app_id: "foot".into(),
            minimized,
            maximized: false,
            fullscreen: false,
            output: None,
        }
    }

    fn entry(id: &str, name: &str, class: Option<&str>) -> DesktopEntry {
        DesktopEntry {
            id: id.into(),
            name: name.into(),
            generic_name: None,
            comment: None,
            exec: "x".into(),
            icon: None,
            terminal: false,
            path: None,
            startup_wm_class: class.map(str::to_owned),
            categories: vec![],
            keywords: vec![],
            source: PathBuf::new(),
        }
    }

    // shell-behaviour.md §2.6, entries with one window
    #[test]
    fn a_taskbar_press_minimizes_the_focused_window_and_focuses_any_other() {
        assert_eq!(
            task_action(&window(1, false), Some(1)),
            ShellMessage::Minimize { id: 1 }
        );
        assert_eq!(
            task_action(&window(1, false), Some(2)),
            ShellMessage::Activate { id: 1 }
        );
        assert_eq!(
            task_action(&window(1, false), None),
            ShellMessage::Activate { id: 1 }
        );
        // A minimized window is restored by focusing it.
        assert_eq!(
            task_action(&window(1, true), None),
            ShellMessage::Activate { id: 1 }
        );
        assert_eq!(
            task_action(&window(1, true), Some(1)),
            ShellMessage::Activate { id: 1 }
        );
    }

    #[test]
    fn taskbar_entries_keep_creation_order() {
        let windows = [window(3, false), window(1, true), window(2, false)];
        let ids: Vec<u64> = task_order(&windows).iter().map(|window| window.id).collect();
        assert_eq!(ids, [1, 2, 3]);
    }

    // §3.3: Up and Down clamp, no wrap
    #[test]
    fn launcher_selection_is_clamped() {
        assert_eq!(move_selection(0, 3, true), 1);
        assert_eq!(move_selection(2, 3, true), 2);
        assert_eq!(move_selection(0, 3, false), 0);
        assert_eq!(move_selection(2, 3, false), 1);
        assert_eq!(move_selection(5, 0, true), 0);
        assert_eq!(move_selection(9, 3, false), 2);
    }

    // §3.4: page-wise scrolling
    #[test]
    fn launcher_results_scroll_by_page() {
        assert_eq!(page_start(0, 8), 0);
        assert_eq!(page_start(7, 8), 0);
        assert_eq!(page_start(8, 8), 8);
        assert_eq!(page_start(17, 8), 16);
        assert_eq!(page_start(3, 0), 0);
    }

    #[test]
    fn application_names_come_from_installed_applications() {
        let catalogue = [
            entry("code", "Visual Studio Code", Some("Code")),
            entry("google-chrome", "Google Chrome", Some("Google-chrome")),
            entry("org.blender.Blender", "Blender", None),
        ];
        assert_eq!(application_name("Code", &catalogue), "Visual Studio Code");
        assert_eq!(application_name("code", &catalogue), "Visual Studio Code");
        assert_eq!(application_name("google-chrome", &catalogue), "Google Chrome");
        assert_eq!(application_name("org.blender.Blender", &catalogue), "Blender");
        // Unknown applications get a readable form of their identifier.
        assert_eq!(application_name("foot", &catalogue), "Foot");
        assert_eq!(
            application_name("org.example.TextEditor", &catalogue),
            "TextEditor"
        );
        assert_eq!(application_name("", &catalogue), "Application");
    }

    #[test]
    fn initials_and_ellipsis() {
        assert_eq!(initials("Visual Studio Code"), "VC");
        assert_eq!(initials("Blender"), "BL");
        assert_eq!(initials("foot"), "FO");
        assert_eq!(initials("amity-web"), "AW");
        assert_eq!(initials("  "), "?");
        assert_eq!(ellipsize("short", 10), "short");
        assert_eq!(ellipsize("exactly ten", 11), "exactly ten");
        assert_eq!(ellipsize("a rather long window title", 12), "a rather lo…");
        assert_eq!(ellipsize("cut after space here", 11), "cut after…");
    }
}
