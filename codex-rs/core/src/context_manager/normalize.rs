use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::InputModality;
use codex_protocol::protocol::COLLABORATION_MODE_CLOSE_TAG;
use codex_protocol::protocol::COLLABORATION_MODE_OPEN_TAG;
use std::collections::HashSet;

use crate::util::error_or_panic;
use tracing::info;

const IMAGE_CONTENT_OMITTED_PLACEHOLDER: &str =
    "image content omitted because you do not support image input";

pub(crate) fn retain_latest_collaboration_mode_message(items: &mut Vec<ResponseItem>) {
    let last_mode_item_index = items
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| message_collaboration_mode_positions(item).map(|_| idx))
        .next_back();

    let Some(last_mode_item_index) = last_mode_item_index else {
        return;
    };

    let mut normalized_items = Vec::with_capacity(items.len());
    for (idx, item) in items.drain(..).enumerate() {
        if let Some(mode_positions) = message_collaboration_mode_positions(&item) {
            let keep_collaboration_mode = idx == last_mode_item_index;
            if let Some(normalized) =
                strip_collaboration_mode_sections(item, &mode_positions, keep_collaboration_mode)
            {
                normalized_items.push(normalized);
            }
        } else {
            normalized_items.push(item);
        }
    }

    *items = normalized_items;
}

pub(crate) fn ensure_call_outputs_present(items: &mut Vec<ResponseItem>) {
    // Collect synthetic outputs to insert immediately after their calls.
    // Store the insertion position (index of call) alongside the item so
    // we can insert in reverse order and avoid index shifting.
    let mut missing_outputs_to_insert: Vec<(usize, ResponseItem)> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        match item {
            ResponseItem::FunctionCall { call_id, .. } => {
                let has_output = items.iter().any(|i| match i {
                    ResponseItem::FunctionCallOutput {
                        call_id: existing, ..
                    } => existing == call_id,
                    _ => false,
                });

                if !has_output {
                    info!("Function call output is missing for call id: {call_id}");
                    missing_outputs_to_insert.push((
                        idx,
                        ResponseItem::FunctionCallOutput {
                            call_id: call_id.clone(),
                            output: FunctionCallOutputPayload::from_text("aborted".to_string()),
                        },
                    ));
                }
            }
            ResponseItem::ToolSearchCall {
                call_id: Some(call_id),
                ..
            } => {
                let has_output = items.iter().any(|i| match i {
                    ResponseItem::ToolSearchOutput {
                        call_id: Some(existing),
                        ..
                    } => existing == call_id,
                    _ => false,
                });

                if !has_output {
                    info!("Tool search output is missing for call id: {call_id}");
                    missing_outputs_to_insert.push((
                        idx,
                        ResponseItem::ToolSearchOutput {
                            call_id: Some(call_id.clone()),
                            status: "completed".to_string(),
                            execution: "client".to_string(),
                            tools: Vec::new(),
                        },
                    ));
                }
            }
            ResponseItem::CustomToolCall { call_id, .. } => {
                let has_output = items.iter().any(|i| match i {
                    ResponseItem::CustomToolCallOutput {
                        call_id: existing, ..
                    } => existing == call_id,
                    _ => false,
                });

                if !has_output {
                    error_or_panic(format!(
                        "Custom tool call output is missing for call id: {call_id}"
                    ));
                    missing_outputs_to_insert.push((
                        idx,
                        ResponseItem::CustomToolCallOutput {
                            call_id: call_id.clone(),
                            name: None,
                            output: FunctionCallOutputPayload::from_text("aborted".to_string()),
                        },
                    ));
                }
            }
            // LocalShellCall is represented in upstream streams by a FunctionCallOutput
            ResponseItem::LocalShellCall { call_id, .. } => {
                if let Some(call_id) = call_id.as_ref() {
                    let has_output = items.iter().any(|i| match i {
                        ResponseItem::FunctionCallOutput {
                            call_id: existing, ..
                        } => existing == call_id,
                        _ => false,
                    });

                    if !has_output {
                        error_or_panic(format!(
                            "Local shell call output is missing for call id: {call_id}"
                        ));
                        missing_outputs_to_insert.push((
                            idx,
                            ResponseItem::FunctionCallOutput {
                                call_id: call_id.clone(),
                                output: FunctionCallOutputPayload::from_text("aborted".to_string()),
                            },
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    // Insert synthetic outputs in reverse index order to avoid re-indexing.
    for (idx, output_item) in missing_outputs_to_insert.into_iter().rev() {
        items.insert(idx + 1, output_item);
    }
}

pub(crate) fn remove_orphan_outputs(items: &mut Vec<ResponseItem>) {
    let function_call_ids: HashSet<String> = items
        .iter()
        .filter_map(|i| match i {
            ResponseItem::FunctionCall { call_id, .. } => Some(call_id.clone()),
            _ => None,
        })
        .collect();

    let tool_search_call_ids: HashSet<String> = items
        .iter()
        .filter_map(|i| match i {
            ResponseItem::ToolSearchCall {
                call_id: Some(call_id),
                ..
            } => Some(call_id.clone()),
            _ => None,
        })
        .collect();

    let local_shell_call_ids: HashSet<String> = items
        .iter()
        .filter_map(|i| match i {
            ResponseItem::LocalShellCall {
                call_id: Some(call_id),
                ..
            } => Some(call_id.clone()),
            _ => None,
        })
        .collect();

    let custom_tool_call_ids: HashSet<String> = items
        .iter()
        .filter_map(|i| match i {
            ResponseItem::CustomToolCall { call_id, .. } => Some(call_id.clone()),
            _ => None,
        })
        .collect();

    items.retain(|item| match item {
        ResponseItem::FunctionCallOutput { call_id, .. } => {
            let has_match =
                function_call_ids.contains(call_id) || local_shell_call_ids.contains(call_id);
            if !has_match {
                error_or_panic(format!(
                    "Orphan function call output for call id: {call_id}"
                ));
            }
            has_match
        }
        ResponseItem::CustomToolCallOutput { call_id, .. } => {
            let has_match = custom_tool_call_ids.contains(call_id);
            if !has_match {
                error_or_panic(format!(
                    "Orphan custom tool call output for call id: {call_id}"
                ));
            }
            has_match
        }
        ResponseItem::ToolSearchOutput { execution, .. } if execution == "server" => true,
        ResponseItem::ToolSearchOutput {
            call_id: Some(call_id),
            ..
        } => {
            let has_match = tool_search_call_ids.contains(call_id);
            if !has_match {
                error_or_panic(format!("Orphan tool search output for call id: {call_id}"));
            }
            has_match
        }
        ResponseItem::ToolSearchOutput { call_id: None, .. } => true,
        _ => true,
    });
}

pub(crate) fn remove_corresponding_for(items: &mut Vec<ResponseItem>, item: &ResponseItem) {
    match item {
        ResponseItem::FunctionCall { call_id, .. } => {
            remove_first_matching(items, |i| {
                matches!(
                    i,
                    ResponseItem::FunctionCallOutput {
                        call_id: existing, ..
                    } if existing == call_id
                )
            });
        }
        ResponseItem::FunctionCallOutput { call_id, .. } => {
            if let Some(pos) = items.iter().position(|i| {
                matches!(i, ResponseItem::FunctionCall { call_id: existing, .. } if existing == call_id)
            }) {
                items.remove(pos);
            } else if let Some(pos) = items.iter().position(|i| {
                matches!(i, ResponseItem::LocalShellCall { call_id: Some(existing), .. } if existing == call_id)
            }) {
                items.remove(pos);
            }
        }
        ResponseItem::ToolSearchCall {
            call_id: Some(call_id),
            ..
        } => {
            remove_first_matching(items, |i| {
                matches!(
                    i,
                    ResponseItem::ToolSearchOutput {
                        call_id: Some(existing),
                        ..
                    } if existing == call_id
                )
            });
        }
        ResponseItem::ToolSearchOutput {
            call_id: Some(call_id),
            ..
        } => {
            remove_first_matching(
                items,
                |i| {
                    matches!(
                        i,
                        ResponseItem::ToolSearchCall {
                            call_id: Some(existing),
                            ..
                        } if existing == call_id
                    )
                },
            );
        }
        ResponseItem::CustomToolCall { call_id, .. } => {
            remove_first_matching(items, |i| {
                matches!(
                    i,
                    ResponseItem::CustomToolCallOutput {
                        call_id: existing, ..
                    } if existing == call_id
                )
            });
        }
        ResponseItem::CustomToolCallOutput { call_id, .. } => {
            remove_first_matching(
                items,
                |i| matches!(i, ResponseItem::CustomToolCall { call_id: existing, .. } if existing == call_id),
            );
        }
        ResponseItem::LocalShellCall {
            call_id: Some(call_id),
            ..
        } => {
            remove_first_matching(items, |i| {
                matches!(
                    i,
                    ResponseItem::FunctionCallOutput {
                        call_id: existing, ..
                    } if existing == call_id
                )
            });
        }
        _ => {}
    }
}

fn remove_first_matching<F>(items: &mut Vec<ResponseItem>, predicate: F)
where
    F: Fn(&ResponseItem) -> bool,
{
    if let Some(pos) = items.iter().position(predicate) {
        items.remove(pos);
    }
}

fn message_collaboration_mode_positions(item: &ResponseItem) -> Option<Vec<usize>> {
    let ResponseItem::Message { role, content, .. } = item else {
        return None;
    };
    if role != "developer" {
        return None;
    }

    let positions: Vec<usize> = content
        .iter()
        .enumerate()
        .filter_map(|(idx, content_item)| {
            let ContentItem::InputText { text } = content_item else {
                return None;
            };
            is_collaboration_mode_section(text).then_some(idx)
        })
        .collect();

    (!positions.is_empty()).then_some(positions)
}

fn strip_collaboration_mode_sections(
    item: ResponseItem,
    collaboration_mode_positions: &[usize],
    keep_collaboration_mode: bool,
) -> Option<ResponseItem> {
    let ResponseItem::Message {
        id,
        role,
        content,
        end_turn,
        phase,
    } = item
    else {
        return Some(item);
    };

    let normalized_content: Vec<ContentItem> = content
        .into_iter()
        .enumerate()
        .filter_map(|(idx, content_item)| {
            let is_collaboration_mode = collaboration_mode_positions.binary_search(&idx).is_ok();
            let keep_content_item = if is_collaboration_mode {
                keep_collaboration_mode
                    && !matches!(
                        &content_item,
                        ContentItem::InputText { text }
                            if is_empty_collaboration_mode_section(text)
                    )
            } else {
                true
            };
            if keep_content_item {
                Some(content_item)
            } else {
                None
            }
        })
        .collect();

    if normalized_content.is_empty() {
        None
    } else {
        Some(ResponseItem::Message {
            id,
            role,
            content: normalized_content,
            end_turn,
            phase,
        })
    }
}

fn is_collaboration_mode_section(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with(COLLABORATION_MODE_OPEN_TAG)
        && trimmed.ends_with(COLLABORATION_MODE_CLOSE_TAG)
}

fn is_empty_collaboration_mode_section(text: &str) -> bool {
    let trimmed = text.trim();
    if !is_collaboration_mode_section(trimmed) {
        return false;
    }

    let Some(without_open_tag) = trimmed.strip_prefix(COLLABORATION_MODE_OPEN_TAG) else {
        return false;
    };
    let Some(inner) = without_open_tag.strip_suffix(COLLABORATION_MODE_CLOSE_TAG) else {
        return false;
    };

    inner.trim().is_empty()
}

/// Strip image content from messages and tool outputs when the model does not support images.
/// When `input_modalities` contains `InputModality::Image`, no stripping is performed.
pub(crate) fn strip_images_when_unsupported(
    input_modalities: &[InputModality],
    items: &mut [ResponseItem],
) {
    let supports_images = input_modalities.contains(&InputModality::Image);
    if supports_images {
        return;
    }

    for item in items.iter_mut() {
        match item {
            ResponseItem::Message { content, .. } => {
                let mut normalized_content = Vec::with_capacity(content.len());
                for content_item in content.iter() {
                    match content_item {
                        ContentItem::InputImage { .. } => {
                            normalized_content.push(ContentItem::InputText {
                                text: IMAGE_CONTENT_OMITTED_PLACEHOLDER.to_string(),
                            });
                        }
                        _ => normalized_content.push(content_item.clone()),
                    }
                }
                *content = normalized_content;
            }
            ResponseItem::FunctionCallOutput { output, .. }
            | ResponseItem::CustomToolCallOutput { output, .. } => {
                if let Some(content_items) = output.content_items_mut() {
                    let mut normalized_content_items = Vec::with_capacity(content_items.len());
                    for content_item in content_items.iter() {
                        match content_item {
                            FunctionCallOutputContentItem::InputImage { .. } => {
                                normalized_content_items.push(
                                    FunctionCallOutputContentItem::InputText {
                                        text: IMAGE_CONTENT_OMITTED_PLACEHOLDER.to_string(),
                                    },
                                );
                            }
                            _ => normalized_content_items.push(content_item.clone()),
                        }
                    }
                    *content_items = normalized_content_items;
                }
            }
            ResponseItem::ImageGenerationCall { result, .. } => {
                result.clear();
            }
            _ => {}
        }
    }
}
