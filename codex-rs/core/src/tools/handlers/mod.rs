pub(crate) mod agent_jobs;
pub mod apply_patch;
mod dynamic;
mod js_repl;
mod list_dir;
mod mcp;
mod mcp_resource;
pub(crate) mod multi_agents;
pub(crate) mod multi_agents_common;
pub(crate) mod multi_agents_v2;
mod plan;
mod request_permissions;
mod request_user_input;
mod shell;
mod test_sync;
mod tool_search;
mod tool_suggest;
pub(crate) mod unified_exec;
mod view_image;

use codex_sandboxing::policy_transforms::intersect_permission_profiles;
use codex_sandboxing::policy_transforms::merge_permission_profiles;
use codex_sandboxing::policy_transforms::normalize_additional_permissions;
use codex_utils_absolute_path::AbsolutePathBufGuard;
pub use plan::PLAN_TOOL;
use serde::Deserialize;
use serde_json::Value;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use crate::codex::Session;
use crate::features::Feature;
use crate::function_tool::FunctionCallError;
use crate::git_info::resolve_root_git_project_for_trust;
use crate::sandboxing::SandboxPermissions;
pub(crate) use crate::tools::code_mode::CodeModeExecuteHandler;
pub(crate) use crate::tools::code_mode::CodeModeWaitHandler;
pub use apply_patch::ApplyPatchHandler;
use codex_protocol::config_types::ModeKind;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AskForApproval;
pub use dynamic::DynamicToolHandler;
pub use js_repl::JsReplHandler;
pub use js_repl::JsReplResetHandler;
pub use list_dir::ListDirHandler;
pub use mcp::McpHandler;
pub use mcp_resource::McpResourceHandler;
pub use plan::PlanHandler;
pub use request_permissions::RequestPermissionsHandler;
pub(crate) use request_permissions::request_permissions_tool_description;
pub use request_user_input::RequestUserInputHandler;
pub(crate) use request_user_input::request_user_input_tool_description;
pub use shell::ShellCommandHandler;
pub use shell::ShellHandler;
pub use test_sync::TestSyncHandler;
pub(crate) use tool_search::DEFAULT_LIMIT as TOOL_SEARCH_DEFAULT_LIMIT;
pub(crate) use tool_search::TOOL_SEARCH_TOOL_NAME;
pub use tool_search::ToolSearchHandler;
pub(crate) use tool_suggest::TOOL_SUGGEST_TOOL_NAME;
pub use tool_suggest::ToolSuggestHandler;
pub use unified_exec::UnifiedExecHandler;
pub use view_image::ViewImageHandler;

fn parse_arguments<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

fn parse_arguments_with_base_path<T>(
    arguments: &str,
    base_path: &Path,
) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    let _guard = AbsolutePathBufGuard::new(base_path);
    parse_arguments(arguments)
}

fn resolve_workdir_base_path(
    arguments: &str,
    default_cwd: &Path,
) -> Result<PathBuf, FunctionCallError> {
    let arguments: Value = parse_arguments(arguments)?;
    Ok(arguments
        .get("workdir")
        .and_then(Value::as_str)
        .filter(|workdir| !workdir.is_empty())
        .map(PathBuf::from)
        .map_or_else(
            || default_cwd.to_path_buf(),
            |workdir| crate::util::resolve_path(default_cwd, &workdir),
        ))
}

pub(crate) fn reject_plan_mode_target_repo_mutation(
    session: &Session,
    mode: ModeKind,
    target_repo_cwd: &Path,
    workdir: &Path,
    is_mutating: bool,
    command: Option<&[String]>,
) -> Result<(), FunctionCallError> {
    if !mode.is_plan_output_mode() || !is_mutating {
        return Ok(());
    }

    if !session
        .features()
        .enabled(Feature::PlanModePreparatoryMutations)
    {
        return Err(FunctionCallError::RespondToModel(
            "Plan mode only allows non-mutating exploration unless `features.plan_mode_preparatory_mutations` is enabled."
                .to_string(),
        ));
    }

    let target_repo_root = resolve_root_git_project_for_trust(target_repo_cwd)
        .unwrap_or_else(|| target_repo_cwd.to_path_buf());
    let target_repo_root = normalize_path_for_comparison(target_repo_root.as_path());

    let canonical_workdir = normalize_path_for_comparison(workdir);
    if canonical_workdir.starts_with(&target_repo_root) {
        return Err(plan_mode_target_repo_mutation_error());
    }

    if command.is_some_and(|command| {
        command_targets_target_repo(
            command,
            canonical_workdir.as_path(),
            target_repo_root.as_path(),
        )
    }) {
        return Err(plan_mode_target_repo_mutation_error());
    }

    Ok(())
}

fn plan_mode_target_repo_mutation_error() -> FunctionCallError {
    FunctionCallError::RespondToModel(
        "Plan mode preparatory mutations must run outside the current target repo. Use a temporary directory or scratch clone/worktree outside the repo before running mutating commands."
            .to_string(),
    )
}

fn command_targets_target_repo(
    command: &[String],
    workdir: &Path,
    target_repo_root: &Path,
) -> bool {
    let mut expects_repo_path_for_flag = false;

    for token in command {
        if expects_repo_path_for_flag {
            expects_repo_path_for_flag = false;
            if token_targets_target_repo(
                token,
                workdir,
                target_repo_root,
                /*allow_bare_token*/ true,
            ) {
                return true;
            }
            continue;
        }

        if matches!(token.as_str(), "-C" | "--work-tree" | "--git-dir") {
            expects_repo_path_for_flag = true;
            continue;
        }

        if let Some(path) = token.strip_prefix("-C")
            && !path.is_empty()
            && token_targets_target_repo(
                path,
                workdir,
                target_repo_root,
                /*allow_bare_token*/ true,
            )
        {
            return true;
        }

        if let Some((flag, value)) = token.split_once('=')
            && flag.starts_with('-')
            && token_targets_target_repo(
                value,
                workdir,
                target_repo_root,
                /*allow_bare_token*/ false,
            )
        {
            return true;
        }

        if token_targets_target_repo(
            token,
            workdir,
            target_repo_root,
            /*allow_bare_token*/ false,
        ) {
            return true;
        }
    }

    false
}

fn token_targets_target_repo(
    token: &str,
    workdir: &Path,
    target_repo_root: &Path,
    allow_bare_token: bool,
) -> bool {
    if !looks_like_path_token(token, allow_bare_token) {
        return false;
    }

    let candidate = crate::util::resolve_path(workdir, &PathBuf::from(token));
    normalize_path_for_comparison(candidate.as_path()).starts_with(target_repo_root)
}

fn looks_like_path_token(token: &str, allow_bare_token: bool) -> bool {
    if token.is_empty()
        || token == "-"
        || token == ".git"
        || token.contains("://")
        || token.contains('\0')
    {
        return false;
    }

    let path = Path::new(token);
    if path.is_absolute() {
        return true;
    }

    if matches!(token, "." | "..") || token.starts_with("./") || token.starts_with("../") {
        return true;
    }

    if token.contains(std::path::MAIN_SEPARATOR)
        || (std::path::MAIN_SEPARATOR != '/' && token.contains('/'))
        || (std::path::MAIN_SEPARATOR != '\\' && token.contains('\\'))
    {
        return true;
    }

    allow_bare_token
}

fn normalize_path_for_comparison(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| lexical_normalize_path(path))
}

fn lexical_normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

/// Validates feature/policy constraints for `with_additional_permissions` and
/// normalizes any path-based permissions. Errors if the request is invalid.
pub(crate) fn normalize_and_validate_additional_permissions(
    additional_permissions_allowed: bool,
    approval_policy: AskForApproval,
    sandbox_permissions: SandboxPermissions,
    additional_permissions: Option<PermissionProfile>,
    permissions_preapproved: bool,
    _cwd: &Path,
) -> Result<Option<PermissionProfile>, String> {
    let uses_additional_permissions = matches!(
        sandbox_permissions,
        SandboxPermissions::WithAdditionalPermissions
    );

    if !permissions_preapproved
        && !additional_permissions_allowed
        && (uses_additional_permissions || additional_permissions.is_some())
    {
        return Err(
            "additional permissions are disabled; enable `features.exec_permission_approvals` before using `with_additional_permissions`"
                .to_string(),
        );
    }

    if uses_additional_permissions {
        if !permissions_preapproved && !matches!(approval_policy, AskForApproval::OnRequest) {
            return Err(format!(
                "approval policy is {approval_policy:?}; reject command — you cannot request additional permissions unless the approval policy is OnRequest"
            ));
        }
        let Some(additional_permissions) = additional_permissions else {
            return Err(
                "missing `additional_permissions`; provide at least one of `network`, `file_system`, or `macos` when using `with_additional_permissions`"
                    .to_string(),
            );
        };
        #[cfg(not(target_os = "macos"))]
        if additional_permissions.macos.is_some() {
            return Err("`additional_permissions.macos` is only supported on macOS".to_string());
        }
        let normalized = normalize_additional_permissions(additional_permissions)?;
        if normalized.is_empty() {
            return Err(
                "`additional_permissions` must include at least one requested permission in `network`, `file_system`, or `macos`"
                    .to_string(),
            );
        }
        return Ok(Some(normalized));
    }

    if additional_permissions.is_some() {
        Err(
            "`additional_permissions` requires `sandbox_permissions` set to `with_additional_permissions`"
                .to_string(),
        )
    } else {
        Ok(None)
    }
}

pub(super) struct EffectiveAdditionalPermissions {
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<PermissionProfile>,
    pub permissions_preapproved: bool,
}

pub(super) fn implicit_granted_permissions(
    sandbox_permissions: SandboxPermissions,
    additional_permissions: Option<&PermissionProfile>,
    effective_additional_permissions: &EffectiveAdditionalPermissions,
) -> Option<PermissionProfile> {
    if !sandbox_permissions.uses_additional_permissions()
        && !matches!(sandbox_permissions, SandboxPermissions::RequireEscalated)
        && additional_permissions.is_none()
    {
        effective_additional_permissions
            .additional_permissions
            .clone()
    } else {
        None
    }
}

pub(super) async fn apply_granted_turn_permissions(
    session: &Session,
    sandbox_permissions: SandboxPermissions,
    additional_permissions: Option<PermissionProfile>,
) -> EffectiveAdditionalPermissions {
    if matches!(sandbox_permissions, SandboxPermissions::RequireEscalated) {
        return EffectiveAdditionalPermissions {
            sandbox_permissions,
            additional_permissions,
            permissions_preapproved: false,
        };
    }

    let granted_session_permissions = session.granted_session_permissions().await;
    let granted_turn_permissions = session.granted_turn_permissions().await;
    let granted_permissions = merge_permission_profiles(
        granted_session_permissions.as_ref(),
        granted_turn_permissions.as_ref(),
    );
    let effective_permissions = merge_permission_profiles(
        additional_permissions.as_ref(),
        granted_permissions.as_ref(),
    );
    let permissions_preapproved = match (effective_permissions.as_ref(), granted_permissions) {
        (Some(effective_permissions), Some(granted_permissions)) => {
            intersect_permission_profiles(effective_permissions.clone(), granted_permissions)
                == *effective_permissions
        }
        _ => false,
    };

    let sandbox_permissions =
        if effective_permissions.is_some() && !sandbox_permissions.uses_additional_permissions() {
            SandboxPermissions::WithAdditionalPermissions
        } else {
            sandbox_permissions
        };

    EffectiveAdditionalPermissions {
        sandbox_permissions,
        additional_permissions: effective_permissions,
        permissions_preapproved,
    }
}

#[cfg(test)]
mod tests {
    use super::EffectiveAdditionalPermissions;
    use super::implicit_granted_permissions;
    use super::normalize_and_validate_additional_permissions;
    use super::reject_plan_mode_target_repo_mutation;
    use crate::codex::make_session_and_context;
    use crate::features::Feature;
    use crate::sandboxing::SandboxPermissions;
    use codex_protocol::config_types::ModeKind;
    use codex_protocol::models::FileSystemPermissions;
    use codex_protocol::models::NetworkPermissions;
    use codex_protocol::models::PermissionProfile;
    use codex_protocol::protocol::AskForApproval;
    use codex_protocol::protocol::GranularApprovalConfig;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    fn network_permissions() -> PermissionProfile {
        PermissionProfile {
            network: Some(NetworkPermissions {
                enabled: Some(true),
            }),
            ..Default::default()
        }
    }

    fn file_system_permissions(path: &std::path::Path) -> PermissionProfile {
        PermissionProfile {
            file_system: Some(FileSystemPermissions {
                read: None,
                write: Some(vec![
                    AbsolutePathBuf::from_absolute_path(path).expect("absolute path"),
                ]),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn preapproved_permissions_work_when_request_permissions_tool_is_enabled_without_exec_permission_approvals_feature()
     {
        let cwd = tempdir().expect("tempdir");

        let normalized = normalize_and_validate_additional_permissions(
            false,
            AskForApproval::Granular(GranularApprovalConfig {
                sandbox_approval: true,
                rules: true,
                skill_approval: true,
                request_permissions: false,
                mcp_elicitations: true,
            }),
            SandboxPermissions::WithAdditionalPermissions,
            Some(network_permissions()),
            true,
            cwd.path(),
        )
        .expect("preapproved permissions should be allowed");

        assert_eq!(normalized, Some(network_permissions()));
    }

    #[test]
    fn fresh_additional_permissions_still_require_exec_permission_approvals_feature() {
        let cwd = tempdir().expect("tempdir");

        let err = normalize_and_validate_additional_permissions(
            false,
            AskForApproval::OnRequest,
            SandboxPermissions::WithAdditionalPermissions,
            Some(network_permissions()),
            false,
            cwd.path(),
        )
        .expect_err("fresh inline permission requests should remain disabled");

        assert_eq!(
            err,
            "additional permissions are disabled; enable `features.exec_permission_approvals` before using `with_additional_permissions`"
        );
    }

    #[test]
    fn implicit_sticky_grants_bypass_inline_permission_validation() {
        let cwd = tempdir().expect("tempdir");
        let granted_permissions = file_system_permissions(cwd.path());
        let implicit_permissions = implicit_granted_permissions(
            SandboxPermissions::UseDefault,
            None,
            &EffectiveAdditionalPermissions {
                sandbox_permissions: SandboxPermissions::WithAdditionalPermissions,
                additional_permissions: Some(granted_permissions.clone()),
                permissions_preapproved: false,
            },
        );

        assert_eq!(implicit_permissions, Some(granted_permissions));
    }

    #[test]
    fn explicit_inline_permissions_do_not_use_implicit_sticky_grant_path() {
        let cwd = tempdir().expect("tempdir");
        let requested_permissions = file_system_permissions(cwd.path());
        let implicit_permissions = implicit_granted_permissions(
            SandboxPermissions::WithAdditionalPermissions,
            Some(&requested_permissions),
            &EffectiveAdditionalPermissions {
                sandbox_permissions: SandboxPermissions::WithAdditionalPermissions,
                additional_permissions: Some(requested_permissions.clone()),
                permissions_preapproved: false,
            },
        );

        assert_eq!(implicit_permissions, None);
    }

    #[tokio::test]
    async fn plan_mode_rejects_mutations_inside_target_repo() {
        let (mut session, _turn_context) = make_session_and_context().await;
        session.enable_feature_for_test(Feature::PlanModePreparatoryMutations);
        let repo = tempdir().expect("tempdir");
        std::fs::create_dir_all(repo.path().join(".git")).expect("git dir");
        let nested = repo.path().join("nested");
        std::fs::create_dir_all(&nested).expect("nested dir");

        let err = reject_plan_mode_target_repo_mutation(
            &session,
            ModeKind::Plan,
            repo.path(),
            &nested,
            true,
            None,
        )
        .expect_err("mutating in target repo should be rejected");

        assert_eq!(
            err.to_string(),
            "Plan mode preparatory mutations must run outside the current target repo. Use a temporary directory or scratch clone/worktree outside the repo before running mutating commands."
        );
    }

    #[tokio::test]
    async fn plan_mode_rejects_mutations_inside_target_dir_without_git_root() {
        let (mut session, _turn_context) = make_session_and_context().await;
        session.enable_feature_for_test(Feature::PlanModePreparatoryMutations);
        let target_dir = tempdir().expect("tempdir");
        let nested = target_dir.path().join("nested");
        std::fs::create_dir_all(&nested).expect("nested dir");

        let err = reject_plan_mode_target_repo_mutation(
            &session,
            ModeKind::Plan,
            target_dir.path(),
            &nested,
            true,
            None,
        )
        .expect_err("mutating in target dir should be rejected even without git metadata");

        assert!(
            err.to_string()
                .contains("must run outside the current target repo")
        );
    }

    #[tokio::test]
    async fn plan_mode_allows_mutations_outside_target_repo_when_feature_enabled() {
        let (mut session, _turn_context) = make_session_and_context().await;
        session.enable_feature_for_test(Feature::PlanModePreparatoryMutations);
        let repo = tempdir().expect("tempdir");
        std::fs::create_dir_all(repo.path().join(".git")).expect("git dir");
        let outside = tempdir().expect("tempdir");

        reject_plan_mode_target_repo_mutation(
            &session,
            ModeKind::Plan,
            repo.path(),
            outside.path(),
            true,
            None,
        )
        .expect("outside repo scratch dir should be allowed");
    }

    #[tokio::test]
    async fn plan_mode_rejects_mutations_when_feature_disabled() {
        let (session, _turn_context) = make_session_and_context().await;
        let outside = tempdir().expect("tempdir");

        let err = reject_plan_mode_target_repo_mutation(
            &session,
            ModeKind::Plan,
            outside.path(),
            outside.path(),
            true,
            None,
        )
        .expect_err("feature-disabled plan mutation should be rejected");

        assert_eq!(
            err.to_string(),
            "Plan mode only allows non-mutating exploration unless `features.plan_mode_preparatory_mutations` is enabled."
        );
    }

    #[tokio::test]
    async fn plan_mode_rejects_git_dash_c_target_repo_from_scratch_dir() {
        let (mut session, _turn_context) = make_session_and_context().await;
        session.enable_feature_for_test(Feature::PlanModePreparatoryMutations);
        let root = tempdir().expect("tempdir");
        let repo = root.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        let scratch = root.path().join("scratch");
        std::fs::create_dir_all(&scratch).expect("scratch dir");

        let err = reject_plan_mode_target_repo_mutation(
            &session,
            ModeKind::Plan,
            repo.as_path(),
            scratch.as_path(),
            true,
            Some(&[
                "git".to_string(),
                "-C".to_string(),
                "../repo".to_string(),
                "commit".to_string(),
                "--amend".to_string(),
            ]),
        )
        .expect_err("git -C repo should be rejected");

        assert!(
            err.to_string()
                .contains("must run outside the current target repo")
        );
    }

    #[tokio::test]
    async fn plan_mode_rejects_mutating_command_with_explicit_target_repo_path_argument() {
        let (mut session, _turn_context) = make_session_and_context().await;
        session.enable_feature_for_test(Feature::PlanModePreparatoryMutations);
        let root = tempdir().expect("tempdir");
        let repo = root.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        let scratch = root.path().join("scratch");
        std::fs::create_dir_all(&scratch).expect("scratch dir");

        let err = reject_plan_mode_target_repo_mutation(
            &session,
            ModeKind::Plan,
            repo.as_path(),
            scratch.as_path(),
            true,
            Some(&[
                "rm".to_string(),
                "-f".to_string(),
                "../repo/src/lib.rs".to_string(),
            ]),
        )
        .expect_err("explicit repo path argument should be rejected");

        assert!(
            err.to_string()
                .contains("must run outside the current target repo")
        );
    }

    #[tokio::test]
    async fn plan_mode_rejects_git_work_tree_flag_targeting_repo() {
        let (mut session, _turn_context) = make_session_and_context().await;
        session.enable_feature_for_test(Feature::PlanModePreparatoryMutations);
        let root = tempdir().expect("tempdir");
        let repo = root.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        let scratch = root.path().join("scratch");
        std::fs::create_dir_all(&scratch).expect("scratch dir");

        let err = reject_plan_mode_target_repo_mutation(
            &session,
            ModeKind::Plan,
            repo.as_path(),
            scratch.as_path(),
            true,
            Some(&[
                "git".to_string(),
                "--work-tree".to_string(),
                repo.display().to_string(),
                "status".to_string(),
            ]),
        )
        .expect_err("git --work-tree repo should be rejected");

        assert!(
            err.to_string()
                .contains("must run outside the current target repo")
        );
    }

    #[tokio::test]
    async fn plan_mode_rejects_git_dir_flag_targeting_repo() {
        let (mut session, _turn_context) = make_session_and_context().await;
        session.enable_feature_for_test(Feature::PlanModePreparatoryMutations);
        let root = tempdir().expect("tempdir");
        let repo = root.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        let scratch = root.path().join("scratch");
        std::fs::create_dir_all(&scratch).expect("scratch dir");

        let err = reject_plan_mode_target_repo_mutation(
            &session,
            ModeKind::Plan,
            repo.as_path(),
            scratch.as_path(),
            true,
            Some(&[
                "git".to_string(),
                "--git-dir".to_string(),
                repo.join(".git").display().to_string(),
                "status".to_string(),
            ]),
        )
        .expect_err("git --git-dir repo/.git should be rejected");

        assert!(
            err.to_string()
                .contains("must run outside the current target repo")
        );
    }
}
