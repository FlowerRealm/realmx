use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;

use crate::GitToolingError;
use crate::operations::repo_subdir;
use crate::operations::resolve_repository_root;
use crate::operations::run_git_for_status;
use crate::operations::run_git_for_stdout;
use crate::operations::run_git_for_stdout_all;

const SHADOW_REPO_LAYOUT_VERSION: &str = "v1";

#[derive(Debug, Clone)]
pub enum SnapshotWorkspace {
    RealGit(RealGitWorkspace),
    ShadowGit(ShadowGitWorkspace),
}

#[derive(Debug, Clone)]
pub struct RealGitWorkspace {
    repo_root: PathBuf,
    repo_prefix: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ShadowGitWorkspace {
    workspace_root: PathBuf,
    shadow_git_dir: PathBuf,
    repo_prefix: Option<PathBuf>,
}

impl SnapshotWorkspace {
    pub fn resolve(
        repo_path: &Path,
        workspace_root: Option<&Path>,
        shadow_git_dir: Option<&Path>,
    ) -> Result<Self, GitToolingError> {
        if let Ok(repo_root) = resolve_repository_root(repo_path) {
            let repo_prefix = repo_subdir(repo_root.as_path(), repo_path);
            return Ok(Self::RealGit(RealGitWorkspace {
                repo_root,
                repo_prefix,
            }));
        }

        let Some(workspace_root) = workspace_root else {
            return Err(GitToolingError::NotAGitRepository {
                path: repo_path.to_path_buf(),
            });
        };
        let Some(shadow_git_dir) = shadow_git_dir else {
            return Err(GitToolingError::ShadowGitDirRequired {
                workspace_root: workspace_root.to_path_buf(),
            });
        };

        let shadow = ShadowGitWorkspace::new(
            workspace_root.to_path_buf(),
            shadow_git_dir.to_path_buf(),
            /*repo_prefix*/ None,
        );
        shadow.ensure_initialized()?;
        Ok(Self::ShadowGit(shadow))
    }

    pub fn repo_root(&self) -> &Path {
        match self {
            SnapshotWorkspace::RealGit(workspace) => workspace.repo_root(),
            SnapshotWorkspace::ShadowGit(workspace) => workspace.repo_root(),
        }
    }

    pub fn repo_prefix(&self) -> Option<&Path> {
        match self {
            SnapshotWorkspace::RealGit(workspace) => workspace.repo_prefix(),
            SnapshotWorkspace::ShadowGit(workspace) => workspace.repo_prefix(),
        }
    }

    pub fn run_git_for_status<I, S>(&self, args: I) -> Result<(), GitToolingError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_git_for_status_with_env(args, &[])
    }

    pub fn run_git_for_status_with_env<I, S>(
        &self,
        args: I,
        env: &[(OsString, OsString)],
    ) -> Result<(), GitToolingError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        match self {
            SnapshotWorkspace::RealGit(workspace) => {
                workspace.run_git_for_status_with_env(args, env)
            }
            SnapshotWorkspace::ShadowGit(workspace) => {
                workspace.run_git_for_status_with_env(args, env)
            }
        }
    }

    pub fn run_git_for_stdout<I, S>(&self, args: I) -> Result<String, GitToolingError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_git_for_stdout_with_env(args, &[])
    }

    pub fn run_git_for_stdout_with_env<I, S>(
        &self,
        args: I,
        env: &[(OsString, OsString)],
    ) -> Result<String, GitToolingError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        match self {
            SnapshotWorkspace::RealGit(workspace) => {
                workspace.run_git_for_stdout_with_env(args, env)
            }
            SnapshotWorkspace::ShadowGit(workspace) => {
                workspace.run_git_for_stdout_with_env(args, env)
            }
        }
    }

    pub fn run_git_for_stdout_all<I, S>(&self, args: I) -> Result<String, GitToolingError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_git_for_stdout_all_with_env(args, &[])
    }

    pub fn run_git_for_stdout_all_with_env<I, S>(
        &self,
        args: I,
        env: &[(OsString, OsString)],
    ) -> Result<String, GitToolingError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        match self {
            SnapshotWorkspace::RealGit(workspace) => {
                workspace.run_git_for_stdout_all_with_env(args, env)
            }
            SnapshotWorkspace::ShadowGit(workspace) => {
                workspace.run_git_for_stdout_all_with_env(args, env)
            }
        }
    }
}

impl RealGitWorkspace {
    fn repo_root(&self) -> &Path {
        self.repo_root.as_path()
    }

    fn repo_prefix(&self) -> Option<&Path> {
        self.repo_prefix.as_deref()
    }

    fn run_git_for_status_with_env<I, S>(
        &self,
        args: I,
        env: &[(OsString, OsString)],
    ) -> Result<(), GitToolingError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let env = (!env.is_empty()).then_some(env);
        run_git_for_status(self.repo_root.as_path(), args, env)
    }

    fn run_git_for_stdout_with_env<I, S>(
        &self,
        args: I,
        env: &[(OsString, OsString)],
    ) -> Result<String, GitToolingError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let env = (!env.is_empty()).then_some(env);
        run_git_for_stdout(self.repo_root.as_path(), args, env)
    }

    fn run_git_for_stdout_all_with_env<I, S>(
        &self,
        args: I,
        env: &[(OsString, OsString)],
    ) -> Result<String, GitToolingError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let env = (!env.is_empty()).then_some(env);
        run_git_for_stdout_all(self.repo_root.as_path(), args, env)
    }
}

impl ShadowGitWorkspace {
    fn new(workspace_root: PathBuf, shadow_git_dir: PathBuf, repo_prefix: Option<PathBuf>) -> Self {
        Self {
            workspace_root,
            shadow_git_dir,
            repo_prefix,
        }
    }

    pub fn shadow_git_dir_for(codex_home: &Path, workspace_root: &Path) -> PathBuf {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        workspace_root.to_string_lossy().hash(&mut hasher);
        let digest = format!("{:016x}", hasher.finish());
        codex_home
            .join("workspace-snapshots")
            .join(SHADOW_REPO_LAYOUT_VERSION)
            .join(digest)
    }

    fn repo_root(&self) -> &Path {
        self.workspace_root.as_path()
    }

    fn repo_prefix(&self) -> Option<&Path> {
        self.repo_prefix.as_deref()
    }

    fn env(&self) -> Vec<(OsString, OsString)> {
        vec![
            (
                OsString::from("GIT_DIR"),
                self.shadow_git_dir.as_os_str().to_os_string(),
            ),
            (
                OsString::from("GIT_WORK_TREE"),
                self.workspace_root.as_os_str().to_os_string(),
            ),
        ]
    }

    fn ensure_initialized(&self) -> Result<(), GitToolingError> {
        let head = self.shadow_git_dir.join("HEAD");
        if head.is_file() {
            return Ok(());
        }

        fs::create_dir_all(&self.shadow_git_dir)?;
        let env = self.env();
        run_git_for_status(
            self.workspace_root.as_path(),
            [OsString::from("init"), OsString::from("--bare")],
            Some(env.as_slice()),
        )?;
        run_git_for_status(
            self.workspace_root.as_path(),
            [
                OsString::from("symbolic-ref"),
                OsString::from("HEAD"),
                OsString::from("refs/heads/main"),
            ],
            Some(env.as_slice()),
        )?;
        Ok(())
    }

    fn run_git_for_status_with_env<I, S>(
        &self,
        args: I,
        env: &[(OsString, OsString)],
    ) -> Result<(), GitToolingError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut merged_env = self.env();
        merged_env.extend_from_slice(env);
        run_git_for_status(
            self.workspace_root.as_path(),
            args,
            Some(merged_env.as_slice()),
        )
    }

    fn run_git_for_stdout_with_env<I, S>(
        &self,
        args: I,
        env: &[(OsString, OsString)],
    ) -> Result<String, GitToolingError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut merged_env = self.env();
        merged_env.extend_from_slice(env);
        run_git_for_stdout(
            self.workspace_root.as_path(),
            args,
            Some(merged_env.as_slice()),
        )
    }

    fn run_git_for_stdout_all_with_env<I, S>(
        &self,
        args: I,
        env: &[(OsString, OsString)],
    ) -> Result<String, GitToolingError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut merged_env = self.env();
        merged_env.extend_from_slice(env);
        run_git_for_stdout_all(
            self.workspace_root.as_path(),
            args,
            Some(merged_env.as_slice()),
        )
    }
}
