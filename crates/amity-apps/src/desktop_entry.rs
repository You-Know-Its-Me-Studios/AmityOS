//! Reading freedesktop.org desktop entries (`*.desktop` files), which is how
//! installed applications describe themselves: native packages, Flatpak apps
//! and vendor installers all provide them.
//!
//! Implements the parts of the Desktop Entry Specification 1.5 that matter for
//! listing and starting applications. Localized keys (`Name[de]`) are read but
//! not used yet.

use std::{
    collections::HashSet,
    fmt, fs,
    path::{Path, PathBuf},
};

/// One launchable application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    /// Desktop file ID: the path below `applications/` without `.desktop`,
    /// with `/` replaced by `-` (for example `org.mozilla.firefox`).
    pub id: String,
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    /// The `Exec` value, unexpanded.
    pub exec: String,
    pub icon: Option<String>,
    /// The program needs a terminal window.
    pub terminal: bool,
    /// Working directory to start the program in.
    pub path: Option<PathBuf>,
    /// The window class or `app_id` the program's windows carry, when it
    /// differs from the desktop file ID.
    pub startup_wm_class: Option<String>,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
    /// The file this entry was read from.
    pub source: PathBuf,
}

/// Why an `Exec` value could not be turned into a command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    Empty,
    UnterminatedQuote,
    /// A field code this implementation does not know, such as `%x`.
    UnknownFieldCode(char),
    /// A `%` at the end of the value.
    DanglingPercent,
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecError::Empty => write!(f, "the command is empty"),
            ExecError::UnterminatedQuote => write!(f, "a quoted argument is not closed"),
            ExecError::UnknownFieldCode(code) => write!(f, "unknown field code %{code}"),
            ExecError::DanglingPercent => write!(f, "the command ends with a lone %"),
        }
    }
}

impl std::error::Error for ExecError {}

/// Undoes the escapes of a `string` value: `\s`, `\n`, `\t`, `\r` and `\\`.
fn unescape(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => result.push(' '),
            Some('n') => result.push('\n'),
            Some('t') => result.push('\t'),
            Some('r') => result.push('\r'),
            Some('\\') => result.push('\\'),
            Some(other) => {
                result.push('\\');
                result.push(other);
            }
            None => result.push('\\'),
        }
    }
    result
}

/// Splits a `;`-separated list value; `\;` is a literal semicolon.
fn split_list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(';') => current.push(';'),
                Some(other) => {
                    current.push('\\');
                    current.push(other);
                }
                None => current.push('\\'),
            },
            ';' => items.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    items.push(current);
    items
        .into_iter()
        .map(|item| unescape(item.trim()))
        .filter(|item| !item.is_empty())
        .collect()
}

fn is_true(value: Option<&str>) -> bool {
    value == Some("true")
}

/// Looks a program up the way a shell would.
fn program_exists(program: &str, search_path: &str) -> bool {
    fn executable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }
    if program.contains('/') {
        return executable(Path::new(program));
    }
    search_path
        .split(':')
        .filter(|dir| !dir.is_empty())
        .any(|dir| executable(&Path::new(dir).join(program)))
}

/// What decides whether an entry is shown.
#[derive(Debug, Clone)]
pub struct Environment {
    /// Desktop names from `XDG_CURRENT_DESKTOP`, for `OnlyShowIn`/`NotShowIn`.
    pub desktops: Vec<String>,
    /// `PATH`, for `TryExec`.
    pub search_path: String,
}

impl Environment {
    pub fn from_process() -> Self {
        Environment {
            desktops: std::env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .split(':')
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect(),
            search_path: std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
        }
    }
}

/// The outcome of reading one desktop file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// An application to list.
    Application(Box<DesktopEntry>),
    /// A valid entry that must not be listed (hidden, another desktop's, not
    /// an application, or its program is missing). It still masks entries with
    /// the same ID in lower-priority directories.
    Hidden,
    /// Not a usable desktop entry.
    Invalid,
}

/// Parses the text of a desktop file.
pub fn parse(id: &str, source: &Path, contents: &str, environment: &Environment) -> Parsed {
    let mut in_entry_group = false;
    let mut seen_entry_group = false;
    let mut keys: Vec<(&str, &str)> = Vec::new();
    for line in contents.lines() {
        let line = line.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(group) = trimmed.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            in_entry_group = group == "Desktop Entry";
            seen_entry_group |= in_entry_group;
            continue;
        }
        if !in_entry_group {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            keys.push((key.trim(), value.trim_start()));
        }
    }
    if !seen_entry_group {
        return Parsed::Invalid;
    }
    // The first occurrence of a key wins; localized keys are a different key.
    let get = |name: &str| keys.iter().find(|(key, _)| *key == name).map(|(_, value)| *value);

    if get("Type") != Some("Application") {
        return if get("Type").is_some() {
            Parsed::Hidden
        } else {
            Parsed::Invalid
        };
    }
    let (Some(name), Some(exec)) = (get("Name"), get("Exec")) else {
        return Parsed::Invalid;
    };
    if is_true(get("Hidden")) || is_true(get("NoDisplay")) {
        return Parsed::Hidden;
    }
    let on_this_desktop = |list: &str| {
        split_list(list)
            .iter()
            .any(|name| environment.desktops.iter().any(|desktop| desktop == name))
    };
    if get("OnlyShowIn").is_some_and(|list| !on_this_desktop(list))
        || get("NotShowIn").is_some_and(on_this_desktop)
    {
        return Parsed::Hidden;
    }
    if let Some(try_exec) = get("TryExec") {
        if !program_exists(&unescape(try_exec), &environment.search_path) {
            return Parsed::Hidden;
        }
    }

    let text = |name: &str| get(name).map(unescape).filter(|value| !value.is_empty());
    Parsed::Application(Box::new(DesktopEntry {
        id: id.to_owned(),
        name: unescape(name),
        generic_name: text("GenericName"),
        comment: text("Comment"),
        exec: unescape(exec),
        icon: text("Icon"),
        terminal: is_true(get("Terminal")),
        path: text("Path").map(PathBuf::from),
        startup_wm_class: text("StartupWMClass"),
        categories: get("Categories").map(split_list).unwrap_or_default(),
        keywords: get("Keywords").map(split_list).unwrap_or_default(),
        source: source.to_owned(),
    }))
}

/// Turns an `Exec` value into a program and its arguments.
///
/// `targets` are the files or URLs to open; none when the application is just
/// started. Field codes are expanded as the specification describes: `%f` and
/// `%u` take the first target, `%F` and `%U` all of them, `%i` becomes
/// `--icon <Icon>`, `%c` the name, `%k` the desktop file, `%%` a percent sign.
/// Deprecated codes are dropped.
pub fn expand_exec(entry: &DesktopEntry, targets: &[String]) -> Result<Vec<String>, ExecError> {
    let mut arguments: Vec<String> = Vec::new();
    let mut current = String::new();
    // True once the current argument has started, even if it is still empty (`""`).
    let mut started = false;
    let mut chars = entry.exec.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => {
                if started {
                    arguments.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        None => return Err(ExecError::UnterminatedQuote),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            None => return Err(ExecError::UnterminatedQuote),
                            // Inside quotes a backslash escapes these four characters.
                            Some(escaped @ ('"' | '`' | '$' | '\\')) => current.push(escaped),
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                        },
                        Some('%') => match chars.next() {
                            Some('%') => current.push('%'),
                            // Field codes are not expanded inside quotes.
                            Some(other) => {
                                current.push('%');
                                current.push(other);
                            }
                            None => return Err(ExecError::DanglingPercent),
                        },
                        Some(other) => current.push(other),
                    }
                }
            }
            '%' => match chars.next() {
                None => return Err(ExecError::DanglingPercent),
                Some('%') => {
                    started = true;
                    current.push('%');
                }
                Some('f' | 'u') => {
                    if let Some(target) = targets.first() {
                        started = true;
                        current.push_str(target);
                    }
                }
                Some('F' | 'U') => {
                    // A list code stands alone as its own argument(s).
                    if started {
                        arguments.push(std::mem::take(&mut current));
                        started = false;
                    }
                    arguments.extend(targets.iter().cloned());
                }
                Some('i') => {
                    if let Some(icon) = &entry.icon {
                        if started {
                            arguments.push(std::mem::take(&mut current));
                            started = false;
                        }
                        arguments.push("--icon".into());
                        arguments.push(icon.clone());
                    }
                }
                Some('c') => {
                    started = true;
                    current.push_str(&entry.name);
                }
                Some('k') => {
                    started = true;
                    current.push_str(&entry.source.to_string_lossy());
                }
                Some('d' | 'D' | 'n' | 'N' | 'v' | 'm') => {}
                Some(other) => return Err(ExecError::UnknownFieldCode(other)),
            },
            other => {
                started = true;
                current.push(other);
            }
        }
    }
    if started {
        arguments.push(current);
    }
    if arguments.first().is_none_or(|program| program.is_empty()) {
        return Err(ExecError::Empty);
    }
    Ok(arguments)
}

/// Directories that hold desktop entries, highest priority first, from
/// `XDG_DATA_HOME` and `XDG_DATA_DIRS` plus the Flatpak export directories.
pub fn application_dirs(
    home: Option<&Path>,
    data_home: Option<&str>,
    data_dirs: Option<&str>,
) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    match data_home.filter(|value| !value.is_empty()) {
        Some(data_home) => roots.push(PathBuf::from(data_home)),
        None => roots.extend(home.map(|home| home.join(".local/share"))),
    }
    roots.extend(home.map(|home| home.join(".local/share/flatpak/exports/share")));
    roots.extend(
        data_dirs
            .filter(|value| !value.is_empty())
            .unwrap_or("/usr/local/share:/usr/share")
            .split(':')
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from),
    );
    roots.push(PathBuf::from("/var/lib/flatpak/exports/share"));

    let mut seen = HashSet::new();
    roots
        .into_iter()
        .map(|root| root.join("applications"))
        .filter(|dir| seen.insert(dir.clone()))
        .collect()
}

fn scan(dir: &Path, prefix: &str, depth: u32, found: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        // Follows symlinks: Flatpak exports are links into the app's files.
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            if depth < 3 {
                scan(&path, &format!("{prefix}{file_name}-"), depth + 1, found);
            }
        } else if let Some(stem) = file_name.strip_suffix(".desktop") {
            found.push((format!("{prefix}{stem}"), path));
        }
    }
}

/// Reads every application from `dirs` (highest priority first), sorted by
/// name. An entry in an earlier directory replaces one with the same ID later.
pub fn load_catalogue(dirs: &[PathBuf], environment: &Environment) -> Vec<DesktopEntry> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut entries = Vec::new();
    for dir in dirs {
        let mut found = Vec::new();
        scan(dir, "", 0, &mut found);
        found.sort();
        for (id, path) in found {
            if seen.contains(&id) {
                continue;
            }
            // Desktop files are small; anything huge is not one.
            if fs::metadata(&path).is_ok_and(|meta| meta.len() > 1 << 20) {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            match parse(&id, &path, &contents, environment) {
                Parsed::Application(entry) => {
                    seen.insert(id);
                    entries.push(*entry);
                }
                Parsed::Hidden => {
                    seen.insert(id);
                }
                Parsed::Invalid => {}
            }
        }
    }
    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    entries
}

/// Applications matching a query, best matches first: names that start with
/// the query, then names that contain it, then matches in the description,
/// keywords or command. An empty query matches everything.
pub fn search<'a>(entries: &'a [DesktopEntry], query: &str) -> Vec<&'a DesktopEntry> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return entries.iter().collect();
    }
    let rank = |entry: &DesktopEntry| -> Option<u8> {
        let name = entry.name.to_lowercase();
        if name.starts_with(&query) {
            Some(0)
        } else if name.split_whitespace().any(|word| word.starts_with(&query)) {
            Some(1)
        } else if name.contains(&query) {
            Some(2)
        } else if entry
            .generic_name
            .iter()
            .chain(entry.keywords.iter())
            .any(|text| text.to_lowercase().contains(&query))
            || entry.id.to_lowercase().contains(&query)
        {
            Some(3)
        } else {
            None
        }
    };
    let mut matches: Vec<(u8, &DesktopEntry)> = entries
        .iter()
        .filter_map(|entry| rank(entry).map(|rank| (rank, entry)))
        .collect();
    // Stable: entries keep their name order within a rank.
    matches.sort_by_key(|(rank, _)| *rank);
    matches.into_iter().map(|(_, entry)| entry).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment() -> Environment {
        Environment {
            desktops: vec!["AmityOS".into()],
            search_path: "/usr/bin:/bin".into(),
        }
    }

    fn application(contents: &str) -> DesktopEntry {
        match parse(
            "test",
            Path::new("/usr/share/applications/test.desktop"),
            contents,
            &environment(),
        ) {
            Parsed::Application(entry) => *entry,
            other => panic!("expected an application, got {other:?}"),
        }
    }

    fn outcome(contents: &str) -> Parsed {
        parse("test", Path::new("/tmp/test.desktop"), contents, &environment())
    }

    fn entry_with_exec(exec: &str) -> DesktopEntry {
        application(&format!(
            "[Desktop Entry]\nType=Application\nName=Test App\nIcon=test-icon\nExec={exec}\n"
        ))
    }

    fn args(exec: &str, targets: &[&str]) -> Result<Vec<String>, ExecError> {
        let targets: Vec<String> = targets.iter().map(|s| s.to_string()).collect();
        expand_exec(&entry_with_exec(exec), &targets)
    }

    const CODE: &str = "\
[Desktop Entry]
Name=Visual Studio Code
Name[de]=Visual Studio Code (de)
Comment=Code Editing. Redefined.
GenericName=Text Editor
Exec=/usr/share/code/code %F
Icon=vscode
Type=Application
StartupNotify=false
StartupWMClass=Code
Categories=TextEditor;Development;IDE;
MimeType=application/x-code-workspace;
Actions=new-empty-window;
Keywords=vscode;

[Desktop Action new-empty-window]
Name=New Empty Window
Exec=/usr/share/code/code --new-window %F
Icon=vscode
";

    #[test]
    fn parses_a_real_world_entry() {
        let entry = application(CODE);
        assert_eq!(entry.id, "test");
        assert_eq!(entry.name, "Visual Studio Code");
        assert_eq!(entry.generic_name.as_deref(), Some("Text Editor"));
        assert_eq!(entry.comment.as_deref(), Some("Code Editing. Redefined."));
        // The action group's Exec must not leak into the entry.
        assert_eq!(entry.exec, "/usr/share/code/code %F");
        assert_eq!(entry.icon.as_deref(), Some("vscode"));
        assert_eq!(entry.startup_wm_class.as_deref(), Some("Code"));
        assert_eq!(entry.categories, ["TextEditor", "Development", "IDE"]);
        assert_eq!(entry.keywords, ["vscode"]);
        assert!(!entry.terminal);
        assert_eq!(entry.path, None);
    }

    #[test]
    fn tolerates_comments_spacing_and_windows_line_endings() {
        let entry = application(
            "# a comment\r\n\r\n[Desktop Entry]\r\nType = Application\r\nName =  Spaced  \r\nExec=tool --flag\r\nTerminal=true\r\nPath=/srv/work\r\n",
        );
        assert_eq!(entry.name, "Spaced  ");
        assert_eq!(entry.exec, "tool --flag");
        assert!(entry.terminal);
        assert_eq!(entry.path.as_deref(), Some(Path::new("/srv/work")));
    }

    #[test]
    fn string_escapes_are_undone() {
        let entry = application("[Desktop Entry]\nType=Application\nName=Tab\\tand\\sspace\\\\\nExec=x\n");
        assert_eq!(entry.name, "Tab\tand space\\");
        assert_eq!(split_list("a\\;b;c;;"), ["a;b", "c"]);
    }

    #[test]
    fn entries_that_must_not_be_listed() {
        let base = "[Desktop Entry]\nType=Application\nName=X\nExec=x\n";
        assert!(matches!(outcome(base), Parsed::Application(_)));
        assert_eq!(outcome(&format!("{base}NoDisplay=true\n")), Parsed::Hidden);
        assert_eq!(outcome(&format!("{base}Hidden=true\n")), Parsed::Hidden);
        assert_eq!(outcome(&format!("{base}OnlyShowIn=GNOME;KDE;\n")), Parsed::Hidden);
        assert!(matches!(
            outcome(&format!("{base}OnlyShowIn=GNOME;AmityOS;\n")),
            Parsed::Application(_)
        ));
        assert_eq!(outcome(&format!("{base}NotShowIn=AmityOS;\n")), Parsed::Hidden);
        assert!(matches!(
            outcome(&format!("{base}NotShowIn=GNOME;\n")),
            Parsed::Application(_)
        ));
        assert_eq!(
            outcome(&format!("{base}TryExec=/nonexistent/amity-test-program\n")),
            Parsed::Hidden
        );
        assert_eq!(
            outcome("[Desktop Entry]\nType=Link\nName=X\nURL=https://example.org\n"),
            Parsed::Hidden
        );
    }

    #[test]
    fn files_that_are_not_desktop_entries() {
        assert_eq!(outcome(""), Parsed::Invalid);
        assert_eq!(outcome("Name=X\nExec=x\nType=Application\n"), Parsed::Invalid);
        assert_eq!(
            outcome("[Desktop Entry]\nType=Application\nName=X\n"),
            Parsed::Invalid
        );
        assert_eq!(
            outcome("[Desktop Entry]\nType=Application\nExec=x\n"),
            Parsed::Invalid
        );
        assert_eq!(
            outcome("[Other]\nType=Application\nName=X\nExec=x\n"),
            Parsed::Invalid
        );
    }

    #[test]
    fn exec_without_targets_drops_file_codes() {
        assert_eq!(
            args("/usr/share/code/code %F", &[]).unwrap(),
            ["/usr/share/code/code"]
        );
        assert_eq!(args("firefox %u", &[]).unwrap(), ["firefox"]);
        assert_eq!(
            args("tool --open %f --verbose", &[]).unwrap(),
            ["tool", "--open", "--verbose"]
        );
        assert_eq!(
            args(
                "/usr/bin/flatpak run --branch=stable --command=blender org.blender.Blender @@ %f @@",
                &[]
            )
            .unwrap(),
            [
                "/usr/bin/flatpak",
                "run",
                "--branch=stable",
                "--command=blender",
                "org.blender.Blender",
                "@@",
                "@@"
            ]
        );
    }

    #[test]
    fn exec_with_targets() {
        assert_eq!(args("edit %f", &["a.txt", "b.txt"]).unwrap(), ["edit", "a.txt"]);
        assert_eq!(
            args("edit %F", &["a b.txt", "c.txt"]).unwrap(),
            ["edit", "a b.txt", "c.txt"]
        );
        assert_eq!(
            args("browse --url=%u", &["https://example.org/?q=1"]).unwrap(),
            ["browse", "--url=https://example.org/?q=1"]
        );
        assert_eq!(args("view %U", &["x", "y"]).unwrap(), ["view", "x", "y"]);
    }

    #[test]
    fn exec_special_codes() {
        assert_eq!(args("app %i", &[]).unwrap(), ["app", "--icon", "test-icon"]);
        assert_eq!(args("app --title=%c", &[]).unwrap(), ["app", "--title=Test App"]);
        assert_eq!(
            args("app %k", &[]).unwrap(),
            ["app", "/usr/share/applications/test.desktop"]
        );
        assert_eq!(
            args("app 100%% %d%D%n%N%v%m done", &[]).unwrap(),
            ["app", "100%", "done"]
        );
        assert_eq!(args("app %x", &[]), Err(ExecError::UnknownFieldCode('x')));
        assert_eq!(args("app %", &[]), Err(ExecError::DanglingPercent));
    }

    #[test]
    fn exec_quoting() {
        assert_eq!(
            args(r#"sh -c "echo \"hi\" \$HOME \\ and `x`" tail"#, &[]),
            // `x` is kept as written: only \` is an escape, a bare backtick is literal.
            Ok(vec![
                "sh".into(),
                "-c".into(),
                "echo \"hi\" $HOME \\ and `x`".into(),
                "tail".into()
            ])
        );
        assert_eq!(args(r#"app "" last"#, &[]).unwrap(), ["app", "", "last"]);
        assert_eq!(
            args(r#"app "two words" "100%%""#, &[]).unwrap(),
            ["app", "two words", "100%"]
        );
        // A field code inside quotes is not expanded.
        assert_eq!(args(r#"app "%f""#, &["x"]).unwrap(), ["app", "%f"]);
        assert_eq!(args(r#"app "open"#, &[]), Err(ExecError::UnterminatedQuote));
        assert_eq!(args("   ", &[]), Err(ExecError::Empty));
        assert_eq!(args("  spaced    out  ", &[]).unwrap(), ["spaced", "out"]);
    }

    #[test]
    fn application_directories_follow_the_base_directory_rules() {
        let home = Path::new("/home/amity");
        assert_eq!(
            application_dirs(Some(home), None, None),
            [
                PathBuf::from("/home/amity/.local/share/applications"),
                PathBuf::from("/home/amity/.local/share/flatpak/exports/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
            ]
        );
        // Explicit values replace the defaults; duplicates are listed once.
        assert_eq!(
            application_dirs(
                Some(home),
                Some("/data/home"),
                Some("/var/lib/flatpak/exports/share:/usr/share")
            ),
            [
                PathBuf::from("/data/home/applications"),
                PathBuf::from("/home/amity/.local/share/flatpak/exports/share/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
                PathBuf::from("/usr/share/applications"),
            ]
        );
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("amity-apps-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn file(name: &str, extra: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec=run-{name}\n{extra}")
    }

    #[test]
    fn catalogue_respects_priority_masking_and_order() {
        let user = TempDir::new("user");
        let system = TempDir::new("system");
        user.write("editor.desktop", &file("My Editor", ""));
        user.write("hidden.desktop", &file("Hidden", "Hidden=true\n"));
        system.write("editor.desktop", &file("System Editor", ""));
        system.write("hidden.desktop", &file("Should Stay Hidden", ""));
        system.write("zebra.desktop", &file("zebra", ""));
        system.write("Alpha.desktop", &file("Alpha", ""));
        system.write("vendor/tool.desktop", &file("Vendor Tool", ""));
        system.write("notes.txt", "not a desktop file");
        system.write("broken.desktop", "garbage");

        let dirs = [
            user.0.clone(),
            system.0.clone(),
            PathBuf::from("/nonexistent/amity"),
        ];
        let catalogue = load_catalogue(&dirs, &environment());
        let listed: Vec<(&str, &str)> = catalogue
            .iter()
            .map(|entry| (entry.id.as_str(), entry.name.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("Alpha", "Alpha"),
                ("editor", "My Editor"),
                ("vendor-tool", "Vendor Tool"),
                ("zebra", "zebra"),
            ]
        );
        assert_eq!(catalogue[1].source, user.0.join("editor.desktop"));
    }

    #[test]
    fn search_ranks_name_matches_first() {
        let named = |id: &str, name: &str, generic: Option<&str>, keywords: &[&str]| DesktopEntry {
            id: id.into(),
            name: name.into(),
            generic_name: generic.map(str::to_owned),
            comment: None,
            exec: "x".into(),
            icon: None,
            terminal: false,
            path: None,
            startup_wm_class: None,
            categories: vec![],
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            source: PathBuf::new(),
        };
        let entries = [
            named(
                "org.blender.Blender",
                "Blender",
                Some("3D modeler"),
                &["3d", "render"],
            ),
            named("code", "Visual Studio Code", Some("Text Editor"), &["vscode"]),
            named("google-chrome", "Google Chrome", Some("Web Browser"), &[]),
            named("firefox", "Firefox", Some("Web Browser"), &["web", "internet"]),
            named("org.gnome.TextEditor", "Text Editor", None, &[]),
        ];
        let names = |query: &str| -> Vec<&str> {
            search(&entries, query)
                .into_iter()
                .map(|entry| entry.name.as_str())
                .collect()
        };
        assert_eq!(
            names(""),
            [
                "Blender",
                "Visual Studio Code",
                "Google Chrome",
                "Firefox",
                "Text Editor"
            ]
        );
        assert_eq!(names("  "), names(""));
        assert_eq!(names("BLEN"), ["Blender"]);
        // Word start in the name outranks a match in the generic name.
        assert_eq!(names("code"), ["Visual Studio Code"]);
        assert_eq!(names("text"), ["Text Editor", "Visual Studio Code"]);
        assert_eq!(names("chrome"), ["Google Chrome"]);
        assert_eq!(names("web"), ["Google Chrome", "Firefox"]);
        assert_eq!(names("vscode"), ["Visual Studio Code"]);
        assert_eq!(names("3d"), ["Blender"]);
        assert_eq!(names("nothing matches this"), Vec::<&str>::new());
    }
}
