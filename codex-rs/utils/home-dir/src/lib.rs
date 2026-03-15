use codex_config::CODEX_HOME_ENV_VAR;
use codex_config::LEGACY_CODEX_HOME_DIR_NAME;
use codex_config::REALMX_HOME_DIR_NAME;
use codex_config::REALMX_HOME_ENV_VAR;
use dirs::home_dir;
use std::path::Path;
use std::path::PathBuf;

/// Returns the path to the Realmx configuration directory.
///
/// Resolution order:
/// 1. `REALMX_HOME`
/// 2. `CODEX_HOME`
/// 3. `~/.realmx` by default, copying `~/.codex` into place once when needed
///
/// - If an environment variable is set, the value must exist and be a
///   directory. The value will be canonicalized and this function will Err
///   otherwise.
/// - If no environment variable is set, this function may create `~/.realmx`
///   by copying the legacy `~/.codex` directory.
pub fn find_codex_home() -> std::io::Result<PathBuf> {
    let realmx_home_env = std::env::var(REALMX_HOME_ENV_VAR)
        .ok()
        .filter(|val| !val.is_empty());
    let codex_home_env = std::env::var(CODEX_HOME_ENV_VAR)
        .ok()
        .filter(|val| !val.is_empty());
    find_codex_home_from_env_with_default_home(
        realmx_home_env.as_deref(),
        codex_home_env.as_deref(),
        home_dir(),
    )
}

#[cfg(test)]
fn find_codex_home_from_env(
    realmx_home_env: Option<&str>,
    codex_home_env: Option<&str>,
) -> std::io::Result<PathBuf> {
    find_codex_home_from_env_with_default_home(realmx_home_env, codex_home_env, home_dir())
}

fn find_codex_home_from_env_with_default_home(
    realmx_home_env: Option<&str>,
    codex_home_env: Option<&str>,
    default_home: Option<PathBuf>,
) -> std::io::Result<PathBuf> {
    if let Some(val) = realmx_home_env {
        return validate_home_env_path(val, REALMX_HOME_ENV_VAR);
    }

    if let Some(val) = codex_home_env {
        return validate_home_env_path(val, CODEX_HOME_ENV_VAR);
    }

    let home = default_home.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not find home directory",
        )
    })?;
    let realmx_home = home.join(REALMX_HOME_DIR_NAME);
    let legacy_codex_home = home.join(LEGACY_CODEX_HOME_DIR_NAME);
    migrate_legacy_dir_if_needed(&legacy_codex_home, &realmx_home)?;
    Ok(realmx_home)
}

fn validate_home_env_path(val: &str, env_var_name: &str) -> std::io::Result<PathBuf> {
    let path = PathBuf::from(val);
    let metadata = std::fs::metadata(&path).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{env_var_name} points to {val:?}, but that path does not exist"),
        ),
        _ => std::io::Error::new(
            err.kind(),
            format!("failed to read {env_var_name} {val:?}: {err}"),
        ),
    })?;

    if !metadata.is_dir() {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{env_var_name} points to {val:?}, but that path is not a directory"),
        ))
    } else {
        path.canonicalize().map_err(|err| {
            std::io::Error::new(
                err.kind(),
                format!("failed to canonicalize {env_var_name} {val:?}: {err}"),
            )
        })
    }
}

/// Copies `legacy_dir` into `target_dir` once when the legacy directory exists
/// and the target directory does not yet exist.
///
/// This is intentionally existence-based migration only. If `target_dir`
/// already exists, the function does nothing and does not attempt to merge,
/// validate, or overwrite any files.
pub fn migrate_legacy_dir_if_needed(legacy_dir: &Path, target_dir: &Path) -> std::io::Result<()> {
    if target_dir.exists() || !legacy_dir.is_dir() {
        return Ok(());
    }

    copy_dir_recursive(legacy_dir, target_dir)
}

fn copy_dir_recursive(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;

    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else if file_type.is_file() {
            std::fs::copy(&source_path, &target_path)?;
        } else if file_type.is_symlink() {
            copy_symlink(&source_path, &target_path)?;
        }
    }

    Ok(())
}

#[cfg(unix)]
fn copy_symlink(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::symlink;

    let link_target = std::fs::read_link(source)?;
    symlink(link_target, target)
}

#[cfg(windows)]
fn copy_symlink(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::symlink_dir;
    use std::os::windows::fs::symlink_file;

    let link_target = std::fs::read_link(source)?;
    if source.is_dir() {
        symlink_dir(link_target, target)
    } else {
        symlink_file(link_target, target)
    }
}

#[cfg(test)]
mod tests {
    use super::find_codex_home_from_env;
    use super::find_codex_home_from_env_with_default_home;
    use super::migrate_legacy_dir_if_needed;
    use codex_config::LEGACY_CODEX_HOME_DIR_NAME;
    use codex_config::REALMX_HOME_DIR_NAME;
    use dirs::home_dir;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::io::ErrorKind;
    #[cfg(unix)]
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn find_realmx_home_env_missing_path_is_fatal() {
        let temp_home = TempDir::new().expect("temp home");
        let missing = temp_home.path().join("missing-realmx-home");
        let missing_str = missing
            .to_str()
            .expect("missing realmx home path should be valid utf-8");

        let err =
            find_codex_home_from_env(Some(missing_str), None).expect_err("missing REALMX_HOME");
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(
            err.to_string().contains("REALMX_HOME"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn find_codex_home_env_is_still_supported() {
        let temp_home = TempDir::new().expect("temp home");
        let temp_str = temp_home
            .path()
            .to_str()
            .expect("temp codex home path should be valid utf-8");

        let resolved =
            find_codex_home_from_env(None, Some(temp_str)).expect("valid legacy CODEX_HOME");
        let expected = temp_home
            .path()
            .canonicalize()
            .expect("canonicalize temp home");
        assert_eq!(resolved, expected);
    }

    #[test]
    fn realmx_home_env_takes_precedence_over_codex_home() {
        let realmx_home = TempDir::new().expect("temp realmx home");
        let codex_home = TempDir::new().expect("temp codex home");

        let resolved =
            find_codex_home_from_env(realmx_home.path().to_str(), codex_home.path().to_str())
                .expect("valid REALMX_HOME");

        let expected = realmx_home
            .path()
            .canonicalize()
            .expect("canonicalize realmx home");
        assert_eq!(resolved, expected);
    }

    #[test]
    fn find_codex_home_env_file_path_is_fatal() {
        let temp_home = TempDir::new().expect("temp home");
        let file_path = temp_home.path().join("codex-home.txt");
        fs::write(&file_path, "not a directory").expect("write temp file");
        let file_str = file_path
            .to_str()
            .expect("file codex home path should be valid utf-8");

        let err = find_codex_home_from_env(None, Some(file_str)).expect_err("file CODEX_HOME");
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("not a directory"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn find_codex_home_without_env_uses_default_realmx_home_dir() {
        let home = home_dir().expect("home dir");
        let resolved = find_codex_home_from_env_with_default_home(None, None, Some(home.clone()))
            .expect("default home");
        let expected = home.join(REALMX_HOME_DIR_NAME);
        assert_eq!(resolved, expected);
    }

    #[test]
    fn find_codex_home_without_env_migrates_legacy_dir() {
        let temp_home = TempDir::new().expect("temp home");
        let legacy_dir = temp_home.path().join(LEGACY_CODEX_HOME_DIR_NAME);
        let target_dir = temp_home.path().join(REALMX_HOME_DIR_NAME);
        fs::create_dir_all(legacy_dir.join("nested")).expect("create legacy dir");
        fs::write(legacy_dir.join("config.toml"), "model = \"o3\"\n").expect("write config");
        fs::write(legacy_dir.join("nested").join("data.txt"), "hello").expect("write nested");

        let resolved = find_codex_home_from_env_with_default_home(
            None,
            None,
            Some(temp_home.path().to_path_buf()),
        )
        .expect("migrate legacy dir");

        assert_eq!(resolved, target_dir);
        assert_eq!(
            fs::read_to_string(target_dir.join("config.toml")).expect("read migrated config"),
            "model = \"o3\"\n"
        );
        assert_eq!(
            fs::read_to_string(target_dir.join("nested").join("data.txt"))
                .expect("read migrated nested file"),
            "hello"
        );
    }

    #[test]
    fn migrate_legacy_dir_if_needed_skips_existing_target_dir() {
        let temp_home = TempDir::new().expect("temp home");
        let legacy_dir = temp_home.path().join(LEGACY_CODEX_HOME_DIR_NAME);
        let target_dir = temp_home.path().join(REALMX_HOME_DIR_NAME);
        fs::create_dir_all(&legacy_dir).expect("create legacy dir");
        fs::create_dir_all(&target_dir).expect("create target dir");
        fs::write(legacy_dir.join("config.toml"), "model = \"o3\"\n").expect("write legacy");
        fs::write(target_dir.join("config.toml"), "model = \"gpt-5\"\n").expect("write target");

        migrate_legacy_dir_if_needed(&legacy_dir, &target_dir).expect("skip existing target");

        assert_eq!(
            fs::read_to_string(target_dir.join("config.toml")).expect("read target config"),
            "model = \"gpt-5\"\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn migrate_legacy_dir_if_needed_copies_symlinks() {
        use std::os::unix::fs::symlink;

        let temp_home = TempDir::new().expect("temp home");
        let legacy_dir = temp_home.path().join(LEGACY_CODEX_HOME_DIR_NAME);
        let target_dir = temp_home.path().join(REALMX_HOME_DIR_NAME);
        fs::create_dir_all(&legacy_dir).expect("create legacy dir");
        fs::write(legacy_dir.join("config.toml"), "model = \"o3\"\n").expect("write config");
        symlink("config.toml", legacy_dir.join("config-link")).expect("create symlink");

        migrate_legacy_dir_if_needed(&legacy_dir, &target_dir).expect("migrate legacy dir");

        assert_eq!(
            fs::read_link(target_dir.join("config-link")).expect("read migrated symlink"),
            PathBuf::from("config.toml")
        );
    }
}
