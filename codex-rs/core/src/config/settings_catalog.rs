use anyhow::Result;
use anyhow::anyhow;
use anyhow::bail;
use schemars::schema::ArrayValidation;
use schemars::schema::InstanceType;
use schemars::schema::Metadata;
use schemars::schema::ObjectValidation;
use schemars::schema::RootSchema;
use schemars::schema::Schema;
use schemars::schema::SchemaObject;
use schemars::schema::SingleOrVec;
use schemars::schema::SubschemaValidation;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingNodeKind {
    Boolean,
    Integer,
    Number,
    String,
    Array,
    Object,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SettingScopeSupport {
    pub global: bool,
    pub profile: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingDescriptor {
    pub key_path: String,
    pub title: String,
    pub description: Option<String>,
    pub kind: SettingNodeKind,
    pub enum_values: Vec<String>,
    pub default_value: Option<Value>,
    pub scopes: SettingScopeSupport,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsCatalog {
    pub nodes: Vec<SettingDescriptor>,
}

pub fn settings_catalog() -> Result<SettingsCatalog> {
    let root_nodes = collect_schema_nodes(
        crate::config::schema::config_schema(),
        /*skipped_root_key*/ Some("profiles"),
    )?;
    let profile_nodes = collect_schema_nodes(
        crate::config::schema::profile_schema(),
        /*skipped_root_key*/ None,
    )?;
    merge_catalog_nodes(root_nodes, profile_nodes)
}

#[derive(Clone, Debug, PartialEq)]
struct CollectedSettingNode {
    title: String,
    description: Option<String>,
    kind: SettingNodeKind,
    enum_values: Vec<String>,
    default_value: Option<Value>,
}

fn collect_schema_nodes(
    root_schema: RootSchema,
    skipped_root_key: Option<&str>,
) -> Result<BTreeMap<String, CollectedSettingNode>> {
    let resolver = SchemaResolver {
        definitions: root_schema.definitions,
    };
    let root = resolver.resolve_object(&root_schema.schema)?;
    let mut nodes = BTreeMap::new();
    let mut path = Vec::new();
    if let Some(properties) = properties(&root) {
        for (key, schema) in properties {
            if skipped_root_key.is_some_and(|skipped_key| skipped_key == key) {
                continue;
            }

            path.push(key.clone());
            collect_nodes(schema, &resolver, &mut path, &mut nodes)?;
            path.pop();
        }
    }
    Ok(nodes)
}

fn collect_nodes(
    schema: &Schema,
    resolver: &SchemaResolver,
    path: &mut Vec<String>,
    nodes: &mut BTreeMap<String, CollectedSettingNode>,
) -> Result<()> {
    let schema = resolver.resolve_schema(schema)?;
    let key_path = path.join(".");
    if !key_path.is_empty() {
        let node = CollectedSettingNode {
            title: path.last().cloned().unwrap_or_default(),
            description: schema
                .metadata
                .as_deref()
                .and_then(|metadata| metadata.description.clone()),
            kind: detect_kind(&schema),
            enum_values: enum_values(&schema),
            default_value: schema
                .metadata
                .as_deref()
                .and_then(|metadata| metadata.default.clone()),
        };
        if nodes.insert(key_path.clone(), node).is_some() {
            bail!("duplicate settings catalog key `{key_path}`");
        }
    }

    match detect_kind(&schema) {
        SettingNodeKind::Object => {
            if let Some(properties) = properties(&schema) {
                for (key, child) in properties {
                    path.push(key.clone());
                    collect_nodes(child, resolver, path, nodes)?;
                    path.pop();
                }
            }
        }
        SettingNodeKind::Array => {
            if let Some(child) = array_item_schema(&schema) {
                path.push("[item]".to_string());
                collect_nodes(child, resolver, path, nodes)?;
                path.pop();
            }
        }
        SettingNodeKind::Boolean
        | SettingNodeKind::Integer
        | SettingNodeKind::Number
        | SettingNodeKind::String
        | SettingNodeKind::Unknown => {}
    }

    Ok(())
}

fn merge_catalog_nodes(
    root_nodes: BTreeMap<String, CollectedSettingNode>,
    profile_nodes: BTreeMap<String, CollectedSettingNode>,
) -> Result<SettingsCatalog> {
    let mut merged = BTreeMap::new();
    for (key_path, root_node) in root_nodes {
        merged.insert(
            key_path.clone(),
            merge_catalog_node(
                key_path.clone(),
                Some(root_node),
                profile_nodes.get(&key_path),
            )?,
        );
    }
    for (key_path, profile_node) in profile_nodes {
        if let std::collections::btree_map::Entry::Vacant(entry) = merged.entry(key_path.clone()) {
            entry.insert(merge_catalog_node(
                key_path,
                /*root_node*/ None,
                Some(&profile_node),
            )?);
        }
    }

    Ok(SettingsCatalog {
        nodes: merged.into_values().collect(),
    })
}

fn merge_catalog_node(
    key_path: String,
    root_node: Option<CollectedSettingNode>,
    profile_node: Option<&CollectedSettingNode>,
) -> Result<SettingDescriptor> {
    let scopes = SettingScopeSupport {
        global: root_node.is_some(),
        profile: profile_node.is_some(),
    };
    let title = root_node
        .as_ref()
        .map(|node| node.title.clone())
        .or_else(|| profile_node.map(|node| node.title.clone()))
        .unwrap_or_default();
    let description = root_node
        .as_ref()
        .and_then(|node| node.description.clone())
        .or_else(|| profile_node.and_then(|node| node.description.clone()));
    let kind = merge_kind(
        root_node.as_ref().map(|node| node.kind),
        profile_node.map(|node| node.kind),
        &key_path,
    )?;
    let enum_values = merge_enum_values(
        root_node
            .as_ref()
            .map(|node| node.enum_values.as_slice())
            .unwrap_or_default(),
        profile_node
            .map(|node| node.enum_values.as_slice())
            .unwrap_or_default(),
    );
    let default_value = merge_default_value(
        root_node
            .as_ref()
            .and_then(|node| node.default_value.clone()),
        profile_node.and_then(|node| node.default_value.clone()),
        &key_path,
    )?;

    Ok(SettingDescriptor {
        key_path,
        title,
        description,
        kind,
        enum_values,
        default_value,
        scopes,
    })
}

fn merge_kind(
    root_kind: Option<SettingNodeKind>,
    profile_kind: Option<SettingNodeKind>,
    key_path: &str,
) -> Result<SettingNodeKind> {
    match (root_kind, profile_kind) {
        (Some(root_kind), Some(profile_kind)) if root_kind == profile_kind => Ok(root_kind),
        (Some(SettingNodeKind::Unknown), Some(profile_kind)) => Ok(profile_kind),
        (Some(root_kind), Some(SettingNodeKind::Unknown)) => Ok(root_kind),
        (Some(root_kind), Some(profile_kind)) => bail!(
            "conflicting settings catalog kinds for `{key_path}`: {root_kind:?} vs {profile_kind:?}"
        ),
        (Some(root_kind), None) => Ok(root_kind),
        (None, Some(profile_kind)) => Ok(profile_kind),
        (None, None) => Err(anyhow!("missing settings catalog node for `{key_path}`")),
    }
}

fn merge_enum_values(root_values: &[String], profile_values: &[String]) -> Vec<String> {
    let mut merged = root_values.to_vec();
    for value in profile_values {
        if !merged.contains(value) {
            merged.push(value.clone());
        }
    }
    merged
}

fn merge_default_value(
    root_value: Option<Value>,
    profile_value: Option<Value>,
    key_path: &str,
) -> Result<Option<Value>> {
    match (root_value, profile_value) {
        (Some(root_value), Some(profile_value)) if root_value == profile_value => {
            Ok(Some(root_value))
        }
        (Some(root_value), Some(profile_value)) => bail!(
            "conflicting settings catalog defaults for `{key_path}`: {root_value} vs {profile_value}"
        ),
        (Some(root_value), None) => Ok(Some(root_value)),
        (None, Some(profile_value)) => Ok(Some(profile_value)),
        (None, None) => Ok(None),
    }
}

fn properties(schema: &SchemaObject) -> Option<&BTreeMap<String, Schema>> {
    schema.object.as_deref().map(|object| &object.properties)
}

fn array_item_schema(schema: &SchemaObject) -> Option<&Schema> {
    match schema.array.as_deref()?.items.as_ref()? {
        SingleOrVec::Single(schema) => Some(schema),
        SingleOrVec::Vec(schemas) => schemas.first(),
    }
}

fn enum_values(schema: &SchemaObject) -> Vec<String> {
    schema
        .enum_values
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            Value::Bool(value) => Some(value.to_string()),
            Value::Null | Value::Array(_) | Value::Object(_) => None,
        })
        .collect()
}

fn detect_kind(schema: &SchemaObject) -> SettingNodeKind {
    let Some(instance_type) = schema.instance_type.as_ref() else {
        return if schema.object.is_some() {
            SettingNodeKind::Object
        } else {
            SettingNodeKind::Unknown
        };
    };

    if instance_type.contains(&InstanceType::Boolean) {
        SettingNodeKind::Boolean
    } else if instance_type.contains(&InstanceType::Integer) {
        SettingNodeKind::Integer
    } else if instance_type.contains(&InstanceType::Number) {
        SettingNodeKind::Number
    } else if instance_type.contains(&InstanceType::String) {
        SettingNodeKind::String
    } else if instance_type.contains(&InstanceType::Array) {
        SettingNodeKind::Array
    } else if instance_type.contains(&InstanceType::Object) || schema.object.is_some() {
        SettingNodeKind::Object
    } else {
        SettingNodeKind::Unknown
    }
}

struct SchemaResolver {
    definitions: BTreeMap<String, Schema>,
}

impl SchemaResolver {
    fn resolve_schema(&self, schema: &Schema) -> Result<SchemaObject> {
        match schema {
            Schema::Bool(value) => Ok(Schema::Bool(*value).into_object()),
            Schema::Object(schema) => self.resolve_object(schema),
        }
    }

    fn resolve_object(&self, schema: &SchemaObject) -> Result<SchemaObject> {
        let mut resolved = schema.clone();
        if let Some(reference) = schema.reference.as_deref() {
            let target = self.resolve_reference(reference)?;
            resolved = merge_schema_objects(self.resolve_schema(target)?, resolved);
        }

        if let Some(subschemas) = schema.subschemas.as_deref() {
            for schema in subschemas.all_of.as_deref().unwrap_or_default() {
                resolved = merge_schema_objects(self.resolve_schema(schema)?, resolved);
            }

            if let Some(candidate) = select_schema_candidate(subschemas) {
                resolved = merge_schema_objects(self.resolve_schema(candidate)?, resolved);
            }
        }

        resolved.reference = None;
        resolved.subschemas = None;
        Ok(resolved)
    }

    fn resolve_reference(&self, reference: &str) -> Result<&Schema> {
        let Some(name) = reference.strip_prefix("#/definitions/") else {
            bail!("unsupported settings catalog reference `{reference}`");
        };
        self.definitions
            .get(name)
            .ok_or_else(|| anyhow!("missing settings catalog definition `{name}`"))
    }
}

fn select_schema_candidate(subschemas: &SubschemaValidation) -> Option<&Schema> {
    subschemas
        .any_of
        .as_deref()
        .and_then(select_non_null_candidate)
        .or_else(|| {
            subschemas
                .one_of
                .as_deref()
                .and_then(select_non_null_candidate)
        })
}

fn select_non_null_candidate(schemas: &[Schema]) -> Option<&Schema> {
    schemas.iter().find(|schema| match schema {
        Schema::Bool(_) => true,
        Schema::Object(schema) => {
            schema.reference.is_some()
                || schema.object.is_some()
                || schema.array.is_some()
                || schema
                    .instance_type
                    .as_ref()
                    .is_some_and(contains_non_null_instance_type)
        }
    })
}

fn contains_non_null_instance_type(instance_type: &SingleOrVec<InstanceType>) -> bool {
    match instance_type {
        SingleOrVec::Single(instance_type) => **instance_type != InstanceType::Null,
        SingleOrVec::Vec(instance_types) => instance_types
            .iter()
            .any(|instance_type| *instance_type != InstanceType::Null),
    }
}

fn merge_schema_objects(base: SchemaObject, mut overlay: SchemaObject) -> SchemaObject {
    overlay.metadata = merge_metadata(base.metadata, overlay.metadata);
    if overlay.instance_type.is_none() {
        overlay.instance_type = base.instance_type;
    }
    if overlay.format.is_none() {
        overlay.format = base.format;
    }
    if overlay.enum_values.is_none() {
        overlay.enum_values = base.enum_values;
    }
    if overlay.const_value.is_none() {
        overlay.const_value = base.const_value;
    }
    if overlay.number.is_none() {
        overlay.number = base.number;
    }
    if overlay.string.is_none() {
        overlay.string = base.string;
    }
    if overlay.array.is_none() {
        overlay.array = base.array;
    } else if let (Some(base_array), Some(overlay_array)) = (base.array, overlay.array.as_mut()) {
        merge_array_validation(base_array.as_ref(), overlay_array.as_mut());
    }
    if overlay.object.is_none() {
        overlay.object = base.object;
    } else if let (Some(base_object), Some(overlay_object)) = (base.object, overlay.object.as_mut())
    {
        merge_object_validation(base_object.as_ref(), overlay_object.as_mut());
    }
    overlay
}

fn merge_array_validation(base: &ArrayValidation, overlay: &mut ArrayValidation) {
    if overlay.items.is_none() {
        overlay.items = base.items.clone();
    }
    if overlay.additional_items.is_none() {
        overlay.additional_items = base.additional_items.clone();
    }
    if overlay.contains.is_none() {
        overlay.contains = base.contains.clone();
    }
}

fn merge_metadata(
    base: Option<Box<Metadata>>,
    overlay: Option<Box<Metadata>>,
) -> Option<Box<Metadata>> {
    match (base, overlay) {
        (Some(base), Some(mut overlay)) => {
            if overlay.id.is_none() {
                overlay.id = base.id;
            }
            if overlay.title.is_none() {
                overlay.title = base.title;
            }
            if overlay.description.is_none() {
                overlay.description = base.description;
            }
            if overlay.default.is_none() {
                overlay.default = base.default;
            }
            overlay.deprecated |= base.deprecated;
            overlay.read_only |= base.read_only;
            overlay.write_only |= base.write_only;
            if overlay.examples.is_empty() {
                overlay.examples = base.examples;
            }
            Some(overlay)
        }
        (Some(base), None) => Some(base),
        (None, Some(overlay)) => Some(overlay),
        (None, None) => None,
    }
}

fn merge_object_validation(base: &ObjectValidation, overlay: &mut ObjectValidation) {
    for (key, value) in &base.properties {
        overlay
            .properties
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    for (key, value) in &base.pattern_properties {
        overlay
            .pattern_properties
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    if overlay.additional_properties.is_none() {
        overlay.additional_properties = base.additional_properties.clone();
    }
    if overlay.property_names.is_none() {
        overlay.property_names = base.property_names.clone();
    }
    for required in &base.required {
        overlay.required.insert(required.clone());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use pretty_assertions::assert_eq;

    use super::SettingScopeSupport;
    use super::settings_catalog;

    #[test]
    fn settings_catalog_has_unique_key_paths() {
        let catalog = settings_catalog().expect("load settings catalog");
        let key_paths: BTreeSet<_> = catalog
            .nodes
            .iter()
            .map(|node| node.key_path.clone())
            .collect();

        assert_eq!(catalog.nodes.len(), key_paths.len());
        assert!(!key_paths.contains("profiles"));
        assert!(
            !key_paths
                .iter()
                .any(|key_path| key_path.starts_with("profiles."))
        );
    }

    #[test]
    fn settings_catalog_marks_scope_support_correctly() {
        let catalog = settings_catalog().expect("load settings catalog");

        let model = catalog
            .nodes
            .iter()
            .find(|node| node.key_path == "model")
            .expect("model setting");
        assert_eq!(
            model.scopes,
            SettingScopeSupport {
                global: true,
                profile: true,
            }
        );

        let microphone = catalog
            .nodes
            .iter()
            .find(|node| node.key_path == "audio.microphone")
            .expect("audio.microphone setting");
        assert_eq!(
            microphone.scopes,
            SettingScopeSupport {
                global: true,
                profile: false,
            }
        );

        let include_apply_patch_tool = catalog
            .nodes
            .iter()
            .find(|node| node.key_path == "include_apply_patch_tool")
            .expect("include_apply_patch_tool setting");
        assert_eq!(
            include_apply_patch_tool.scopes,
            SettingScopeSupport {
                global: false,
                profile: true,
            }
        );
    }

    #[test]
    fn settings_catalog_merges_root_and_profile_metadata() {
        let catalog = settings_catalog().expect("load settings catalog");
        let service_tier = catalog
            .nodes
            .iter()
            .find(|node| node.key_path == "service_tier")
            .expect("service_tier setting");

        assert_eq!(
            service_tier.scopes,
            SettingScopeSupport {
                global: true,
                profile: true,
            }
        );
        assert_eq!(
            service_tier.enum_values,
            vec!["fast".to_string(), "flex".to_string()]
        );
        assert!(
            service_tier
                .description
                .as_deref()
                .is_some_and(|description| description.contains("service tier")),
            "expected root description to survive merge, got {:?}",
            service_tier.description
        );
    }
}
