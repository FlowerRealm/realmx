use anyhow::Result;

pub(crate) use codex_core::config::settings_catalog::SettingDescriptor as SchemaNode;
pub(crate) use codex_core::config::settings_catalog::SettingNodeKind as SchemaNodeKind;
pub(crate) use codex_core::config::settings_catalog::SettingsCatalog as SettingsSchema;
pub(crate) use codex_core::config::settings_navigation::SettingsSectionDescriptor;
pub(crate) use codex_core::config::settings_navigation::SettingsSectionMatcher;

pub(crate) fn load_settings_schema() -> Result<SettingsSchema> {
    codex_core::config::settings_catalog::settings_catalog()
}

pub(crate) fn load_settings_sections() -> &'static [SettingsSectionDescriptor] {
    codex_core::config::settings_navigation::settings_sections()
}
