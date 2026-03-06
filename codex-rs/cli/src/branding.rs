use std::ffi::OsStr;

pub const CLI_NAME_ENV_VAR: &str = "CODEX_CLI_NAME";
pub const LEGACY_CLI_NAME: &str = "codex";
pub const PRIMARY_CLI_NAME: &str = "realmx";

pub fn display_cli_name() -> String {
    display_cli_name_from(
        std::env::var_os(CLI_NAME_ENV_VAR).as_deref(),
        std::env::args_os().next().as_deref(),
    )
}

pub fn display_cli_name_from(env_name: Option<&OsStr>, arg0: Option<&OsStr>) -> String {
    env_name
        .and_then(normalize_cli_name)
        .or_else(|| arg0.and_then(normalize_cli_name))
        .unwrap_or(PRIMARY_CLI_NAME)
        .to_string()
}

pub fn command_example(args: &str) -> String {
    let cli_name = display_cli_name();
    if args.is_empty() {
        cli_name
    } else {
        format!("{cli_name} {args}")
    }
}

pub fn rewrite_command_for_cli(command: &str, cli_name: &str) -> String {
    if command == LEGACY_CLI_NAME {
        cli_name.to_string()
    } else if let Some(rest) = command.strip_prefix(&format!("{LEGACY_CLI_NAME} ")) {
        format!("{cli_name} {rest}")
    } else {
        command.to_string()
    }
}

fn normalize_cli_name(value: &OsStr) -> Option<&'static str> {
    let file_name = value.to_str()?.rsplit(['/', '\\']).next()?;
    let normalized = file_name.to_ascii_lowercase();
    let stem = normalized.strip_suffix(".exe").unwrap_or(&normalized);

    if stem == PRIMARY_CLI_NAME || stem.starts_with(&format!("{PRIMARY_CLI_NAME}-")) {
        Some(PRIMARY_CLI_NAME)
    } else if stem == LEGACY_CLI_NAME || stem.starts_with(&format!("{LEGACY_CLI_NAME}-")) {
        Some(LEGACY_CLI_NAME)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn display_cli_name_prefers_explicit_env() {
        assert_eq!(
            display_cli_name_from(Some(OsStr::new("realmx")), Some(OsStr::new("codex"))),
            PRIMARY_CLI_NAME
        );
    }

    #[test]
    fn display_cli_name_defaults_to_primary_brand_for_unknown_entrypoints() {
        assert_eq!(
            display_cli_name_from(None, Some(OsStr::new("/tmp/custom-wrapper"))),
            PRIMARY_CLI_NAME
        );
    }

    #[test]
    fn display_cli_name_normalizes_platform_specific_binaries() {
        assert_eq!(
            display_cli_name_from(None, Some(OsStr::new("/tmp/realmx-x86_64-apple-darwin"))),
            PRIMARY_CLI_NAME
        );
        assert_eq!(
            display_cli_name_from(None, Some(OsStr::new("C:\\bin\\realmx.exe"))),
            PRIMARY_CLI_NAME
        );
        assert_eq!(
            display_cli_name_from(None, Some(OsStr::new("C:\\bin\\codex.exe"))),
            LEGACY_CLI_NAME
        );
    }

    #[test]
    fn rewrite_command_for_cli_rewrites_legacy_prefix() {
        assert_eq!(
            rewrite_command_for_cli("codex resume abc", PRIMARY_CLI_NAME),
            "realmx resume abc"
        );
    }
}
