use crate::config::Config;
use crate::config_loader::ConfigLayerStackOrdering;
use crate::config_loader::merge_toml_values;
use crate::config_loader::project_root_markers_from_config;
use crate::git_info::get_git_repo_root;
use codex_app_server_protocol::ConfigLayerSource;
use codex_config::LEGACY_PROJECT_CONFIG_DIR_NAME;
use codex_config::PROJECT_CONFIG_DIR_NAME;
use codex_git::CreateGhostCommitOptions;
use codex_git::RestoreGhostCommitOptions;
use codex_git::ShadowGitWorkspace;
use std::path::Path;
use std::path::PathBuf;

const WORKSPACE_SNAPSHOT_FALLBACK_MARKERS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    PROJECT_CONFIG_DIR_NAME,
    LEGACY_PROJECT_CONFIG_DIR_NAME,
    ".agents",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
];

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceSnapshotPaths {
    pub(crate) workspace_root: PathBuf,
    pub(crate) shadow_git_dir: Option<PathBuf>,
}

pub(crate) fn resolve_workspace_snapshot_paths(
    config: &Config,
    cwd: &Path,
) -> WorkspaceSnapshotPaths {
    if let Some(repo_root) = get_git_repo_root(cwd) {
        return WorkspaceSnapshotPaths {
            workspace_root: repo_root,
            shadow_git_dir: None,
        };
    }

    let workspace_root = discover_workspace_root(cwd, config);
    let shadow_git_dir = Some(ShadowGitWorkspace::shadow_git_dir_for(
        config.codex_home.as_path(),
        &workspace_root,
    ));
    WorkspaceSnapshotPaths {
        workspace_root,
        shadow_git_dir,
    }
}

pub(crate) fn apply_workspace_snapshot_to_create_options<'a>(
    options: CreateGhostCommitOptions<'a>,
    paths: &WorkspaceSnapshotPaths,
) -> CreateGhostCommitOptions<'a> {
    let options = options.workspace_root(paths.workspace_root.clone());
    if let Some(shadow_git_dir) = &paths.shadow_git_dir {
        options.shadow_git_dir(shadow_git_dir.clone())
    } else {
        options
    }
}

pub(crate) fn apply_workspace_snapshot_to_restore_options<'a>(
    options: RestoreGhostCommitOptions<'a>,
    paths: &WorkspaceSnapshotPaths,
) -> RestoreGhostCommitOptions<'a> {
    let options = options.workspace_root(paths.workspace_root.clone());
    if let Some(shadow_git_dir) = &paths.shadow_git_dir {
        options.shadow_git_dir(shadow_git_dir.clone())
    } else {
        options
    }
}

fn discover_workspace_root(cwd: &Path, config: &Config) -> PathBuf {
    let markers = workspace_root_markers(config);

    for ancestor in cwd.ancestors() {
        if markers.iter().any(|marker| ancestor.join(marker).exists()) {
            return ancestor.to_path_buf();
        }
    }

    cwd.to_path_buf()
}

fn workspace_root_markers(config: &Config) -> Vec<String> {
    let mut merged = toml::Value::Table(toml::map::Map::new());
    for layer in config.config_layer_stack.get_layers(
        ConfigLayerStackOrdering::LowestPrecedenceFirst,
        /*include_disabled*/ false,
    ) {
        if matches!(layer.name, ConfigLayerSource::Project { .. }) {
            continue;
        }
        merge_toml_values(&mut merged, &layer.config);
    }

    match project_root_markers_from_config(&merged) {
        Ok(Some(markers)) if !markers.is_empty() => markers,
        Ok(Some(_)) | Ok(None) | Err(_) => WORKSPACE_SNAPSHOT_FALLBACK_MARKERS
            .iter()
            .map(ToString::to_string)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigBuilder;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::process::Command;
    use toml::Value as TomlValue;

    fn git(path: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(path)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed with {status}");
    }

    fn init_git_repo(path: &Path) {
        git(path, &["init", "--initial-branch=main"]);
        git(path, &["config", "user.name", "Codex Tests"]);
        git(path, &["config", "user.email", "codex-tests@example.com"]);
        fs::write(path.join("README.txt"), "repo\n").expect("write README");
        git(path, &["add", "README.txt"]);
        git(path, &["commit", "-m", "init"]);
    }

    #[test]
    fn resolve_workspace_snapshot_paths_uses_git_root_when_available() {
        let temp = tempfile::tempdir().expect("tempdir");
        init_git_repo(temp.path());
        fs::create_dir_all(temp.path().join("nested")).expect("create nested");
        let config = futures::executor::block_on(
            ConfigBuilder::default()
                .codex_home(temp.path().join("home"))
                .build(),
        )
        .expect("build config");

        let paths = resolve_workspace_snapshot_paths(&config, &temp.path().join("nested"));

        assert_eq!(paths.workspace_root, temp.path());
        assert_eq!(paths.shadow_git_dir, None);
    }

    #[test]
    fn resolve_workspace_snapshot_paths_uses_fallback_marker_for_non_git_project() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let nested = project.join("src/app");
        fs::create_dir_all(&nested).expect("create nested");
        fs::write(project.join("package.json"), "{}\n").expect("write package.json");
        let codex_home = temp.path().join("home");
        fs::create_dir_all(&codex_home).expect("create home");
        let config = futures::executor::block_on(
            ConfigBuilder::default()
                .codex_home(codex_home.clone())
                .build(),
        )
        .expect("build config");

        let paths = resolve_workspace_snapshot_paths(&config, &nested);

        assert_eq!(paths.workspace_root, project);
        assert_eq!(
            paths.shadow_git_dir,
            Some(ShadowGitWorkspace::shadow_git_dir_for(
                codex_home.as_path(),
                temp.path().join("project").as_path()
            ))
        );
    }

    #[test]
    fn resolve_workspace_snapshot_paths_respects_custom_project_root_markers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let nested = project.join("pkg");
        fs::create_dir_all(&nested).expect("create nested");
        fs::write(project.join(".workspace-root"), "x").expect("write marker");
        let config = futures::executor::block_on(
            ConfigBuilder::default()
                .codex_home(temp.path().join("home"))
                .cli_overrides(vec![(
                    "project_root_markers".to_string(),
                    TomlValue::Array(vec![TomlValue::String(".workspace-root".to_string())]),
                )])
                .build(),
        )
        .expect("build config");

        let paths = resolve_workspace_snapshot_paths(&config, &nested);

        assert_eq!(paths.workspace_root, project);
    }
}
