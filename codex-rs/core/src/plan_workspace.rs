use crate::git_info::get_git_repo_root;
use crate::plan_csv::render_empty_plan_csv;
use crate::plan_csv::render_plan_text;
use codex_state::ThreadPlanItemCreateParams;
use codex_state::canonicalize_thread_plan_csv;
use codex_state::parse_thread_plan_csv;
use std::borrow::Cow;
use std::path::Path;
use std::path::PathBuf;
use tokio::fs;

const PLANS_DIR: &str = "plans";
const THREADS_DIR: &str = "threads";
const ARCHIVED_THREADS_DIR: &str = "archived_threads";
const RUNTIME_DIR: &str = ".runtime";
const REQUIREMENTS_FILE: &str = "requirements.md";
const DESIGN_FILE: &str = "design.md";
const TASKS_CSV_FILE: &str = "tasks.csv";
const TASKS_MD_FILE: &str = "tasks.md";
const ACTIVE_TASKS_CSV_FILE: &str = "active_tasks.csv";
const ACTIVE_TASKS_MD_FILE: &str = "active_tasks.md";

#[derive(Debug, Clone)]
pub struct PlanWorkspace {
    codex_home: PathBuf,
    repo_name: String,
    root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanWorkspaceFile {
    Requirements,
    Design,
    TasksCsv,
    TasksMd,
}

impl PlanWorkspaceFile {
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Requirements => REQUIREMENTS_FILE,
            Self::Design => DESIGN_FILE,
            Self::TasksCsv => TASKS_CSV_FILE,
            Self::TasksMd => TASKS_MD_FILE,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanWorkspaceSnapshot {
    pub requirements: String,
    pub design: String,
    pub draft_tasks_csv: String,
    pub draft_tasks_md: String,
    pub active_tasks_csv: Option<String>,
    pub active_tasks_md: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PlanWorkspacePlan {
    pub raw_csv: String,
    pub rows: Vec<ThreadPlanItemCreateParams>,
    pub plan_text: String,
}

#[derive(Debug, Clone)]
pub struct PlanWorkspaceResolvedPlan {
    pub plan: PlanWorkspacePlan,
    pub source: PlanWorkspacePlanSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanWorkspacePlanSource {
    Draft,
    Active,
}

impl PlanWorkspace {
    pub fn new(codex_home: &Path, cwd: &Path, thread_id: &str) -> Self {
        let repo_name = repo_slug_for_workspace(cwd);
        let root = active_root(codex_home, &repo_name, thread_id);
        Self {
            codex_home: codex_home.to_path_buf(),
            repo_name,
            root,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn file_path(&self, file: PlanWorkspaceFile) -> PathBuf {
        self.root.join(file.file_name())
    }

    fn runtime_file_path(&self, file_name: &str) -> PathBuf {
        self.root.join(RUNTIME_DIR).join(file_name)
    }

    pub fn archived_root(&self) -> PathBuf {
        archived_root(self.codex_home.as_path(), &self.repo_name, self.thread_id())
    }

    pub fn archived_file_path(&self, file: PlanWorkspaceFile) -> PathBuf {
        self.archived_root().join(file.file_name())
    }

    pub async fn archived_exists(&self) -> anyhow::Result<bool> {
        fs::try_exists(self.archived_root())
            .await
            .map_err(Into::into)
    }

    pub async fn move_to_archived(&self) -> anyhow::Result<bool> {
        self.move_root(self.root(), self.archived_root().as_path())
            .await
    }

    pub async fn restore_from_archived(&self) -> anyhow::Result<bool> {
        self.move_root(self.archived_root().as_path(), self.root())
            .await
    }

    pub async fn ensure_scaffold(&self) -> anyhow::Result<()> {
        fs::create_dir_all(&self.root).await?;
        fs::create_dir_all(self.root.join(RUNTIME_DIR)).await?;
        self.ensure_file(PlanWorkspaceFile::Requirements, "")
            .await?;
        self.ensure_file(PlanWorkspaceFile::Design, "").await?;
        self.ensure_file(
            PlanWorkspaceFile::TasksCsv,
            render_empty_plan_csv().as_str(),
        )
        .await?;
        self.ensure_file(PlanWorkspaceFile::TasksMd, "# Plan\n")
            .await?;
        Ok(())
    }

    async fn ensure_file(
        &self,
        file: PlanWorkspaceFile,
        default_content: &str,
    ) -> anyhow::Result<()> {
        let path = self.file_path(file);
        if fs::try_exists(&path).await? {
            return Ok(());
        }
        fs::write(path, default_content).await?;
        Ok(())
    }

    pub async fn read_file(&self, file: PlanWorkspaceFile) -> anyhow::Result<String> {
        self.ensure_scaffold().await?;
        Ok(read_to_string_if_exists(self.file_path(file).as_path())
            .await?
            .unwrap_or_default())
    }

    pub async fn write_file(&self, file: PlanWorkspaceFile, content: &str) -> anyhow::Result<()> {
        self.ensure_scaffold().await?;
        match file {
            PlanWorkspaceFile::TasksCsv => {
                let plan = canonicalize_workspace_tasks_csv(content)?;
                fs::write(
                    self.file_path(PlanWorkspaceFile::TasksCsv),
                    plan.raw_csv.as_str(),
                )
                .await?;
                fs::write(
                    self.file_path(PlanWorkspaceFile::TasksMd),
                    plan.plan_text.as_str(),
                )
                .await?;
            }
            PlanWorkspaceFile::TasksMd => {
                anyhow::bail!("tasks.md is derived from tasks.csv and cannot be edited directly");
            }
            PlanWorkspaceFile::Requirements | PlanWorkspaceFile::Design => {
                fs::write(self.file_path(file), content).await?;
            }
        }
        Ok(())
    }

    pub async fn snapshot(&self) -> anyhow::Result<PlanWorkspaceSnapshot> {
        self.ensure_scaffold().await?;
        Ok(PlanWorkspaceSnapshot {
            requirements: self.read_file(PlanWorkspaceFile::Requirements).await?,
            design: self.read_file(PlanWorkspaceFile::Design).await?,
            draft_tasks_csv: self.read_file(PlanWorkspaceFile::TasksCsv).await?,
            draft_tasks_md: self.read_file(PlanWorkspaceFile::TasksMd).await?,
            active_tasks_csv: read_to_string_if_exists(
                self.runtime_file_path(ACTIVE_TASKS_CSV_FILE).as_path(),
            )
            .await?,
            active_tasks_md: read_to_string_if_exists(
                self.runtime_file_path(ACTIVE_TASKS_MD_FILE).as_path(),
            )
            .await?,
        })
    }

    pub async fn resolve_plan_for_restore(
        &self,
    ) -> anyhow::Result<Option<PlanWorkspaceResolvedPlan>> {
        if fs::try_exists(self.root()).await? {
            let draft_csv =
                read_to_string_if_exists(self.file_path(PlanWorkspaceFile::TasksCsv).as_path())
                    .await?
                    .unwrap_or_default();
            if let Some(plan) = parse_workspace_plan(draft_csv.as_str())? {
                return Ok(Some(PlanWorkspaceResolvedPlan {
                    plan,
                    source: PlanWorkspacePlanSource::Draft,
                }));
            }
        }

        if fs::try_exists(self.archived_root()).await? {
            let archived_draft_csv = read_to_string_if_exists(
                self.archived_file_path(PlanWorkspaceFile::TasksCsv)
                    .as_path(),
            )
            .await?
            .unwrap_or_default();
            if let Some(plan) = parse_workspace_plan(archived_draft_csv.as_str())? {
                return Ok(Some(PlanWorkspaceResolvedPlan {
                    plan,
                    source: PlanWorkspacePlanSource::Draft,
                }));
            }
        }

        if !fs::try_exists(self.root()).await? && !fs::try_exists(self.archived_root()).await? {
            self.ensure_scaffold().await?;
        }

        let active_csv = if fs::try_exists(self.root()).await? {
            read_to_string_if_exists(self.runtime_file_path(ACTIVE_TASKS_CSV_FILE).as_path())
                .await?
        } else if fs::try_exists(self.archived_root()).await? {
            read_to_string_if_exists(
                self.archived_root()
                    .join(RUNTIME_DIR)
                    .join(ACTIVE_TASKS_CSV_FILE)
                    .as_path(),
            )
            .await?
        } else {
            read_to_string_if_exists(self.runtime_file_path(ACTIVE_TASKS_CSV_FILE).as_path())
                .await?
        };
        let Some(active_csv) = active_csv else {
            return Ok(None);
        };
        let Some(plan) = parse_workspace_plan(active_csv.as_str())? else {
            return Ok(None);
        };
        Ok(Some(PlanWorkspaceResolvedPlan {
            plan,
            source: PlanWorkspacePlanSource::Active,
        }))
    }

    pub async fn finalize_plan_for_acceptance(&self) -> anyhow::Result<PlanWorkspacePlan> {
        self.ensure_scaffold().await?;
        let draft_csv = self.read_file(PlanWorkspaceFile::TasksCsv).await?;
        match parse_workspace_plan(draft_csv.as_str())? {
            Some(plan) => Ok(plan),
            None => anyhow::bail!(
                "tasks.csv must include at least one non-header row before finalizing the plan"
            ),
        }
    }

    pub async fn persist_active_plan(
        &self,
        raw_csv: &str,
        update_public_draft: bool,
    ) -> anyhow::Result<PlanWorkspacePlan> {
        self.ensure_scaffold().await?;
        let plan = canonicalize_workspace_tasks_csv(raw_csv)?;
        fs::write(
            self.runtime_file_path(ACTIVE_TASKS_CSV_FILE),
            plan.raw_csv.as_str(),
        )
        .await?;
        fs::write(
            self.runtime_file_path(ACTIVE_TASKS_MD_FILE),
            plan.plan_text.as_str(),
        )
        .await?;
        if update_public_draft {
            fs::write(
                self.file_path(PlanWorkspaceFile::TasksCsv),
                plan.raw_csv.as_str(),
            )
            .await?;
            fs::write(
                self.file_path(PlanWorkspaceFile::TasksMd),
                plan.plan_text.as_str(),
            )
            .await?;
        }
        Ok(plan)
    }

    pub async fn draft_matches_active(&self) -> anyhow::Result<bool> {
        self.ensure_scaffold().await?;
        let draft_csv = self.read_file(PlanWorkspaceFile::TasksCsv).await?;
        let active_csv =
            read_to_string_if_exists(self.runtime_file_path(ACTIVE_TASKS_CSV_FILE).as_path())
                .await?;
        let Some(active_csv) = active_csv else {
            return Ok(true);
        };
        let draft = normalize_workspace_csv(draft_csv.as_str())?;
        let active = normalize_workspace_csv(active_csv.as_str())?;
        Ok(draft == active)
    }

    pub async fn rendered_plan_document(&self) -> anyhow::Result<String> {
        let snapshot = self.snapshot().await?;
        let tasks_md = if snapshot.draft_tasks_md.trim().is_empty() {
            snapshot
                .active_tasks_md
                .as_deref()
                .filter(|content| !content.trim().is_empty())
                .map(Cow::Borrowed)
                .unwrap_or_else(|| Cow::Owned(snapshot.draft_tasks_md.clone()))
        } else {
            Cow::Owned(snapshot.draft_tasks_md.clone())
        };

        let mut sections = Vec::new();
        if !snapshot.requirements.trim().is_empty() {
            sections.push(format!(
                "# Requirements\n\n{}",
                snapshot.requirements.trim_end()
            ));
        }
        if !snapshot.design.trim().is_empty() {
            sections.push(format!("# Design\n\n{}", snapshot.design.trim_end()));
        }
        if !tasks_md.trim().is_empty() {
            sections.push(tasks_md.trim_end().to_string());
        }
        Ok(sections.join("\n\n"))
    }

    fn thread_id(&self) -> &str {
        self.root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
    }

    async fn move_root(&self, source: &Path, destination: &Path) -> anyhow::Result<bool> {
        if !fs::try_exists(source).await? {
            return Ok(false);
        }
        if fs::try_exists(destination).await? {
            anyhow::bail!(
                "plan workspace destination already exists: {}",
                destination.display()
            );
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::rename(source, destination).await?;
        self.remove_empty_plan_parents(source).await?;
        Ok(true)
    }

    async fn remove_empty_plan_parents(&self, source: &Path) -> anyhow::Result<()> {
        let mut current = source.parent();
        let plans_root = self.codex_home.join(PLANS_DIR);
        while let Some(path) = current {
            if path == plans_root {
                break;
            }
            if fs::read_dir(path).await?.next_entry().await?.is_some() {
                break;
            }
            fs::remove_dir(path).await?;
            current = path.parent();
        }
        Ok(())
    }
}

pub fn canonicalize_workspace_tasks_csv(content: &str) -> anyhow::Result<PlanWorkspacePlan> {
    let raw_csv = normalize_workspace_csv(content)?;
    let rows = parse_thread_plan_csv(raw_csv.as_str())?;
    let plan_text = render_plan_text(rows.as_slice());
    Ok(PlanWorkspacePlan {
        raw_csv,
        rows,
        plan_text,
    })
}

fn parse_workspace_plan(content: &str) -> anyhow::Result<Option<PlanWorkspacePlan>> {
    if !workspace_csv_has_rows(content) {
        return Ok(None);
    }
    canonicalize_workspace_tasks_csv(content).map(Some)
}

fn normalize_workspace_csv(content: &str) -> anyhow::Result<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(render_empty_plan_csv());
    }

    let normalized = if content.ends_with('\n') {
        content.to_string()
    } else {
        format!("{content}\n")
    };

    if !workspace_csv_has_rows(normalized.as_str()) {
        let header = normalized.trim();
        let expected = codex_state::THREAD_PLAN_CSV_HEADERS.join(",");
        if header != expected {
            anyhow::bail!("plan csv headers must be {expected}; found {header}");
        }
        return Ok(render_empty_plan_csv());
    }

    canonicalize_thread_plan_csv(normalized.as_str())
}

fn workspace_csv_has_rows(content: &str) -> bool {
    content.lines().skip(1).any(|line| !line.trim().is_empty())
}

async fn read_to_string_if_exists(path: &Path) -> anyhow::Result<Option<String>> {
    if !fs::try_exists(path).await? {
        return Ok(None);
    }
    Ok(Some(fs::read_to_string(path).await?))
}

fn active_root(codex_home: &Path, repo_name: &str, thread_id: &str) -> PathBuf {
    codex_home
        .join(PLANS_DIR)
        .join(repo_name)
        .join(THREADS_DIR)
        .join(thread_id)
}

fn archived_root(codex_home: &Path, repo_name: &str, thread_id: &str) -> PathBuf {
    codex_home
        .join(PLANS_DIR)
        .join(repo_name)
        .join(ARCHIVED_THREADS_DIR)
        .join(thread_id)
}

fn repo_slug_for_workspace(cwd: &Path) -> String {
    let repo_root = get_git_repo_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let name = repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("workspace");
    sanitize_repo_name(name)
}

fn sanitize_repo_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut previous_dash = false;
    for ch in name.chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            previous_dash = false;
            Some(ch.to_ascii_lowercase())
        } else if ch == '-' || ch == '_' || ch == '.' || ch.is_ascii_whitespace() {
            if previous_dash {
                None
            } else {
                previous_dash = true;
                Some('-')
            }
        } else {
            None
        };
        if let Some(ch) = normalized {
            out.push(ch);
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "workspace".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::PlanWorkspace;
    use super::PlanWorkspaceFile;
    use super::PlanWorkspacePlanSource;
    use super::canonicalize_workspace_tasks_csv;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    const SAMPLE_CSV: &str = "\
id,status,step,path,details,inputs,outputs,depends_on,acceptance
plan-01,in_progress,Build workspace,codex-rs/core/src/plan_workspace.rs,add file-first persistence,plan requirements,workspace snapshot,,workspace files are updated
";

    #[test]
    fn canonicalize_workspace_tasks_csv_accepts_header_only_scaffold() {
        let plan = canonicalize_workspace_tasks_csv(
            "id,status,step,path,details,inputs,outputs,depends_on,acceptance\nplan-01,pending,Step,file.rs,,,,,\n",
        )
        .expect("csv should canonicalize");
        assert_eq!(plan.rows.len(), 1);
        assert!(plan.plan_text.contains("Step"));
    }

    #[tokio::test]
    async fn workspace_scaffold_and_restore_prefers_draft() {
        let tmp = tempdir().expect("tempdir");
        let workspace = PlanWorkspace::new(tmp.path(), tmp.path(), "thread-1");
        workspace.ensure_scaffold().await.expect("scaffold");
        workspace
            .write_file(PlanWorkspaceFile::TasksCsv, SAMPLE_CSV)
            .await
            .expect("write draft");
        workspace
            .persist_active_plan(
                "id,status,step,path,details,inputs,outputs,depends_on,acceptance\nplan-01,completed,Active,file.rs,,,,,\n",
                /*update_public_draft*/ false,
            )
            .await
            .expect("persist active");

        let resolved = workspace
            .resolve_plan_for_restore()
            .await
            .expect("resolve")
            .expect("plan should exist");
        assert_eq!(resolved.source, PlanWorkspacePlanSource::Draft);
        assert_eq!(resolved.plan.rows[0].row_id, "plan-01");
        assert_eq!(resolved.plan.rows[0].step, "Build workspace");
    }

    #[tokio::test]
    async fn rendered_plan_document_prefers_draft_tasks_markdown() {
        let tmp = tempdir().expect("tempdir");
        let workspace = PlanWorkspace::new(tmp.path(), tmp.path(), "thread-1");
        workspace
            .write_file(PlanWorkspaceFile::Requirements, "Need draft first")
            .await
            .expect("write requirements");
        workspace
            .write_file(PlanWorkspaceFile::TasksCsv, SAMPLE_CSV)
            .await
            .expect("write draft");
        workspace
            .persist_active_plan(
                "id,status,step,path,details,inputs,outputs,depends_on,acceptance\nplan-01,completed,Active,file.rs,,,,,\n",
                /*update_public_draft*/ false,
            )
            .await
            .expect("persist active");

        let rendered = workspace
            .rendered_plan_document()
            .await
            .expect("render document");
        assert!(rendered.contains("Build workspace"));
        assert!(!rendered.contains("\n1. Active"));
    }

    #[tokio::test]
    async fn workspace_can_move_to_archived_and_restore() {
        let tmp = tempdir().expect("tempdir");
        let workspace = PlanWorkspace::new(tmp.path(), tmp.path(), "thread-1");
        workspace.ensure_scaffold().await.expect("scaffold");

        assert!(
            workspace
                .move_to_archived()
                .await
                .expect("archive workspace")
        );
        assert!(!workspace.root().exists());
        assert!(workspace.archived_root().exists());

        assert!(
            workspace
                .restore_from_archived()
                .await
                .expect("restore archived workspace")
        );
        assert!(workspace.root().exists());
        assert!(!workspace.archived_root().exists());
    }
}
