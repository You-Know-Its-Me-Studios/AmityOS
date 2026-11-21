//! Applications: the catalogue of installed applications and how they start.
//!
//! Implemented so far (Phase 3): reading desktop entries, searching them and
//! launching applications as systemd user services. The app manager daemon
//! (per-Space app profiles and grants, autostart, its D-Bus API) is Phase 5;
//! see docs/specs/apps-lifecycle.md and decision 0007.

pub mod desktop_entry;
pub mod launch;

pub use desktop_entry::{application_dirs, load_catalogue, search, DesktopEntry, Environment};
pub use launch::{LaunchError, Launcher};

/// The applications installed for the current user, sorted by name.
pub fn installed_applications() -> Vec<DesktopEntry> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let data_home = std::env::var("XDG_DATA_HOME").ok();
    let data_dirs = std::env::var("XDG_DATA_DIRS").ok();
    let dirs = application_dirs(home.as_deref(), data_home.as_deref(), data_dirs.as_deref());
    load_catalogue(&dirs, &Environment::from_process())
}
