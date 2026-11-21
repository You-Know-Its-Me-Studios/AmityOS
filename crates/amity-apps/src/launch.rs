//! Starting applications.
//!
//! In a normal session every application runs as its own transient systemd
//! user service in `app.slice` (`app-AmityOS-<id>-<n>.service`), following the
//! systemd convention for desktop environments. That gives each application
//! its own control group (for the resource limits Spaces apply later), puts
//! its output in the journal, and makes it independent of the shell process.
//! Without a user service manager (for example a nested development session)
//! the program is started directly.

use std::{
    fmt, io,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::desktop_entry::{expand_exec, DesktopEntry, ExecError};

/// Variables an application needs to find the displays and the session.
const FORWARDED_VARIABLES: &[&str] = &[
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_TYPE",
    "MOZ_ENABLE_WAYLAND",
];

#[derive(Debug)]
pub enum LaunchError {
    /// The entry's `Exec` value is not a valid command line.
    Exec(ExecError),
    /// The program could not be started.
    Spawn(io::Error),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LaunchError::Exec(err) => write!(f, "invalid command: {err}"),
            LaunchError::Spawn(err) => write!(f, "could not start: {err}"),
        }
    }
}

impl std::error::Error for LaunchError {}

/// How applications are started in this session.
#[derive(Debug, Clone)]
pub struct Launcher {
    /// Start applications as transient systemd user services.
    pub use_systemd: bool,
    /// Command that runs a program in a terminal window, for `Terminal=true` entries.
    pub terminal: Vec<String>,
    /// Display and session variables handed to every application.
    pub environment: Vec<(String, String)>,
    /// Default working directory.
    pub home: Option<PathBuf>,
}

/// A fully resolved command, ready to spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    pub program: String,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
    /// The systemd unit the application will run in, if any.
    pub unit: Option<String>,
}

/// Makes an application ID safe to use inside a unit name. `-` separates the
/// parts of the name, so it is replaced as well.
fn unit_component(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(96)
        .collect();
    if cleaned.is_empty() {
        "app".into()
    } else {
        cleaned
    }
}

/// `app-AmityOS-<id>-<unique>.service`
pub fn unit_name(id: &str, unique: u64) -> String {
    format!("app-AmityOS-{}-{unique:x}.service", unit_component(id))
}

fn unique_number() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0);
    // Distinct within this process even on a coarse clock, and across restarts.
    (nanos << 8) ^ (u64::from(std::process::id()) << 40) ^ COUNTER.fetch_add(1, Ordering::Relaxed)
}

impl Launcher {
    /// Chooses how to launch from the current process environment.
    pub fn detect() -> Self {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        let user_manager = runtime
            .as_ref()
            .is_some_and(|dir| dir.join("systemd/private").exists());
        let search_path = std::env::var("PATH").unwrap_or_default();
        let has_systemd_run = search_path
            .split(':')
            .any(|dir| !dir.is_empty() && Path::new(dir).join("systemd-run").is_file());
        Launcher {
            use_systemd: user_manager && has_systemd_run,
            terminal: vec!["foot".into(), "-e".into()],
            environment: FORWARDED_VARIABLES
                .iter()
                .filter_map(|name| Some((name.to_string(), std::env::var(name).ok()?)))
                .collect(),
            home: std::env::var_os("HOME").map(PathBuf::from),
        }
    }

    /// Builds the command that starts `entry`, optionally opening `targets`.
    pub fn command_line(
        &self,
        entry: &DesktopEntry,
        targets: &[String],
        unique: u64,
    ) -> Result<CommandLine, LaunchError> {
        let mut application = expand_exec(entry, targets).map_err(LaunchError::Exec)?;
        if entry.terminal {
            let mut wrapped = self.terminal.clone();
            wrapped.append(&mut application);
            application = wrapped;
        }
        // A `Path=` that does not exist would make the application fail to start.
        let working_directory = entry
            .path
            .clone()
            .filter(|path| path.is_dir())
            .or_else(|| self.home.clone());

        if !self.use_systemd {
            let mut application = application.into_iter();
            return Ok(CommandLine {
                program: application.next().unwrap_or_default(),
                arguments: application.collect(),
                working_directory,
                unit: None,
            });
        }

        let unit = unit_name(&entry.id, unique);
        let mut arguments = vec![
            "--user".to_string(),
            "--collect".into(),
            "--quiet".into(),
            format!("--unit={unit}"),
            "--slice=app.slice".into(),
            format!("--description={}", entry.name),
            // The application counts as running while any of its processes
            // does, so launchers that start the real program and exit work.
            "--property=ExitType=cgroup".into(),
        ];
        if let Some(directory) = &working_directory {
            arguments.push(format!("--working-directory={}", directory.display()));
        }
        for (name, value) in &self.environment {
            arguments.push(format!("--setenv={name}={value}"));
        }
        arguments.push("--".into());
        arguments.extend(application);
        Ok(CommandLine {
            program: "systemd-run".into(),
            arguments,
            working_directory: None,
            unit: Some(unit),
        })
    }

    /// Starts `entry`. Returns once the program (or its service) was started;
    /// it does not wait for the application.
    pub fn launch(&self, entry: &DesktopEntry, targets: &[String]) -> Result<CommandLine, LaunchError> {
        let line = self.command_line(entry, targets, unique_number())?;
        let mut command = Command::new(&line.program);
        command
            .args(&line.arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Not part of the shell's process group: a signal to the shell
            // must not reach applications.
            .process_group(0);
        // The shell's private sockets never reach applications.
        command.env_remove("WAYLAND_SOCKET").env_remove("AMITY_SHELL_FD");
        for (name, value) in &self.environment {
            command.env(name, value);
        }
        if let Some(directory) = line.working_directory.as_ref().filter(|dir| dir.is_dir()) {
            command.current_dir(directory);
        }
        let mut child = command.spawn().map_err(LaunchError::Spawn)?;
        // Reap the child when it ends: at once for systemd-run, at exit for a
        // directly started application. A failing systemd-run (no user service
        // manager, unit rejected) is reported in the journal; the launcher
        // cannot wait for it without freezing.
        let unit = line.unit.clone();
        std::thread::Builder::new()
            .name("app-wait".into())
            .stack_size(64 * 1024)
            .spawn(move || match (child.wait(), unit) {
                (Ok(status), Some(unit)) if !status.success() => {
                    eprintln!("amity-apps: systemd-run could not start {unit}: {status}");
                }
                _ => {}
            })
            .map_err(LaunchError::Spawn)?;
        Ok(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str, exec: &str, terminal: bool) -> DesktopEntry {
        DesktopEntry {
            id: id.into(),
            name: name.into(),
            generic_name: None,
            comment: None,
            exec: exec.into(),
            icon: None,
            terminal,
            path: None,
            startup_wm_class: None,
            categories: vec![],
            keywords: vec![],
            source: PathBuf::from("/usr/share/applications/x.desktop"),
        }
    }

    fn launcher(use_systemd: bool) -> Launcher {
        Launcher {
            use_systemd,
            terminal: vec!["foot".into(), "-e".into()],
            environment: vec![
                ("WAYLAND_DISPLAY".into(), "wayland-1".into()),
                ("DISPLAY".into(), ":0".into()),
            ],
            home: Some(PathBuf::from("/home/amity")),
        }
    }

    #[test]
    fn unit_names_are_valid_and_identify_the_application() {
        assert_eq!(
            unit_name("org.blender.Blender", 0x2a),
            "app-AmityOS-org.blender.Blender-2a.service"
        );
        // Characters with a meaning in unit names are replaced.
        assert_eq!(
            unit_name("google-chrome", 1),
            "app-AmityOS-google_chrome-1.service"
        );
        assert_eq!(unit_name("a b/c@d\\e", 1), "app-AmityOS-a_b_c_d_e-1.service");
        assert_eq!(unit_name("", 1), "app-AmityOS-app-1.service");
        assert!(unit_name(&"x".repeat(500), u64::MAX).len() < 200);
    }

    #[test]
    fn unique_numbers_differ() {
        let numbers: std::collections::HashSet<u64> = (0..1000).map(|_| unique_number()).collect();
        assert_eq!(numbers.len(), 1000);
    }

    #[test]
    fn direct_launch_runs_the_program_itself() {
        let line = launcher(false)
            .command_line(
                &entry("code", "Visual Studio Code", "/usr/share/code/code %F", false),
                &[],
                7,
            )
            .unwrap();
        assert_eq!(
            line,
            CommandLine {
                program: "/usr/share/code/code".into(),
                arguments: vec![],
                working_directory: Some(PathBuf::from("/home/amity")),
                unit: None,
            }
        );
    }

    #[test]
    fn systemd_launch_wraps_the_program_in_a_user_service() {
        let mut blender = entry("org.blender.Blender", "Blender", "blender %f", false);
        let project = std::env::temp_dir();
        blender.path = Some(project.clone());
        let line = launcher(true)
            .command_line(&blender, &["scene one.blend".to_string()], 0x10)
            .unwrap();
        assert_eq!(line.program, "systemd-run");
        assert_eq!(
            line.unit.as_deref(),
            Some("app-AmityOS-org.blender.Blender-10.service")
        );
        assert_eq!(line.working_directory, None);
        assert_eq!(
            line.arguments,
            [
                "--user",
                "--collect",
                "--quiet",
                "--unit=app-AmityOS-org.blender.Blender-10.service",
                "--slice=app.slice",
                "--description=Blender",
                "--property=ExitType=cgroup",
                &format!("--working-directory={}", project.display()),
                "--setenv=WAYLAND_DISPLAY=wayland-1",
                "--setenv=DISPLAY=:0",
                "--",
                "blender",
                "scene one.blend",
            ]
        );
    }

    #[test]
    fn terminal_applications_run_inside_the_terminal() {
        let htop = entry("htop", "Htop", "htop --tree", true);
        let line = launcher(false).command_line(&htop, &[], 1).unwrap();
        assert_eq!(line.program, "foot");
        assert_eq!(line.arguments, ["-e", "htop", "--tree"]);
        let line = launcher(true).command_line(&htop, &[], 1).unwrap();
        assert_eq!(
            &line.arguments[line.arguments.len() - 5..],
            ["--", "foot", "-e", "htop", "--tree"]
        );
    }

    #[test]
    fn an_invalid_command_is_reported() {
        let broken = entry("broken", "Broken", "tool \"unterminated", false);
        assert!(matches!(
            launcher(true).command_line(&broken, &[], 1),
            Err(LaunchError::Exec(ExecError::UnterminatedQuote))
        ));
    }

    #[test]
    fn direct_launch_starts_and_reaps_a_real_process() {
        let marker = std::env::temp_dir().join(format!("amity-apps-launch-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let touch = entry(
            "touch",
            "Touch",
            &format!("touch \"{}\"", marker.display()),
            false,
        );
        let mut launcher = launcher(false);
        launcher.home = None;
        launcher.launch(&touch, &[]).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(marker.exists(), "the launched program did not run");
        let _ = std::fs::remove_file(&marker);

        let missing = entry("missing", "Missing", "/nonexistent/amity-test-program", false);
        assert!(matches!(
            launcher.launch(&missing, &[]),
            Err(LaunchError::Spawn(_))
        ));
    }
}
