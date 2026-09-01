use cut_runtime::RuntimeError;
use std::env;
use std::path::{Path, PathBuf};

const DEFAULT_ACTOR: &str = "ui";

/// Native application configuration resolved before the window is created.
///
/// The application owns configuration and sidecar discovery, while the
/// project contents remain owned by `cutd`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppConfig {
    project_directory: PathBuf,
    cutd_binary: PathBuf,
    actor: String,
    window_title: String,
}

impl AppConfig {
    /// Resolve the packaged-app defaults and explicit `CUT_*` overrides.
    pub fn from_env() -> Result<Self, RuntimeError> {
        let project_directory = env::var_os("CUT_PROJECT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(default_project_directory);
        let cutd_binary = discover_cutd_binary()?;
        Ok(Self {
            project_directory,
            cutd_binary,
            actor: env::var("CUT_APP_ACTOR").unwrap_or_else(|_| DEFAULT_ACTOR.into()),
            window_title: env::var("CUT_APP_WINDOW_TITLE")
                .unwrap_or_else(|_| "Cut · Native editor".into()),
        })
    }

    /// Build a deterministic configuration for tests and smoke commands.
    pub fn for_project_directory(
        project_directory: impl Into<PathBuf>,
        cutd_binary: impl Into<PathBuf>,
    ) -> Self {
        Self {
            project_directory: project_directory.into(),
            cutd_binary: cutd_binary.into(),
            actor: DEFAULT_ACTOR.into(),
            window_title: "Cut · Native editor".into(),
        }
    }

    /// Override the project directory without changing sidecar discovery.
    pub fn with_project_directory(mut self, project_directory: impl Into<PathBuf>) -> Self {
        self.project_directory = project_directory.into();
        self
    }

    /// Override the `cutd` executable used by the request client.
    pub fn with_cutd_binary(mut self, cutd_binary: impl Into<PathBuf>) -> Self {
        self.cutd_binary = cutd_binary.into();
        self
    }

    pub fn project_directory(&self) -> &Path {
        &self.project_directory
    }

    pub fn cutd_binary(&self) -> &Path {
        &self.cutd_binary
    }

    pub fn actor(&self) -> &str {
        &self.actor
    }

    pub fn window_title(&self) -> &str {
        &self.window_title
    }
}

/// Find the packaged `cutd` sidecar, then fall back to the user's PATH.
pub fn discover_cutd_binary() -> Result<PathBuf, RuntimeError> {
    if let Some(value) = env::var_os("CUTD_BIN").or_else(|| env::var_os("CUT_RUNTIME_BIN")) {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
        return Err(RuntimeError::new(
            "RUNTIME_UNAVAILABLE",
            format!("configured cutd binary does not exist: {}", path.display()),
        ));
    }

    let mut candidates = Vec::new();
    if let Ok(executable) = env::current_exe() {
        if let Some(parent) = executable.parent() {
            candidates.push(parent.join(binary_name("cutd")));
        }
    }
    let root = repository_root();
    candidates.push(root.join("target/debug").join(binary_name("cutd")));
    candidates.push(root.join("target/release").join(binary_name("cutd")));

    Ok(candidates
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(binary_name("cutd"))))
}

fn binary_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn default_project_directory() -> PathBuf {
    if cfg!(debug_assertions) {
        return repository_root().join(".cut").join("native-project");
    }

    if let Some(value) = env::var_os("CUT_APP_DATA_DIR") {
        return PathBuf::from(value).join("project");
    }

    let root = if cfg!(target_os = "macos") {
        home_directory()
            .map(|home| home.join("Library").join("Application Support").join("Cut"))
            .unwrap_or_else(|| PathBuf::from(".cut"))
    } else if cfg!(windows) {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".cut"))
            .join("Cut")
    } else {
        env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home_directory().map(|home| home.join(".local").join("share")))
            .unwrap_or_else(|| PathBuf::from(".cut"))
            .join("cut")
    };
    root.join("project")
}

fn home_directory() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::{binary_name, AppConfig};
    use std::path::PathBuf;

    #[test]
    fn smoke_configuration_keeps_runtime_outside_the_view_model() {
        let config = AppConfig::for_project_directory("/tmp/cut-project", "/tmp/cutd");
        assert_eq!(
            config.project_directory(),
            PathBuf::from("/tmp/cut-project")
        );
        assert_eq!(config.cutd_binary(), PathBuf::from("/tmp/cutd"));
        assert_eq!(config.actor(), "ui");
    }

    #[test]
    fn sidecar_name_is_platform_safe() {
        assert!(binary_name("cutd").starts_with("cutd"));
    }
}
