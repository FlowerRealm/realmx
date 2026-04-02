const MODEL_PROVIDER_ID_ALLOWED_CHARS: &str = "ASCII letters, digits, '-' or '_'";

pub fn validate_model_provider_id(provider_id: &str) -> Result<(), String> {
    if provider_id.trim().is_empty() {
        return Err("Provider ID is required.".to_string());
    }

    if provider_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        Ok(())
    } else {
        Err(format!(
            "Provider ID must use {MODEL_PROVIDER_ID_ALLOWED_CHARS}."
        ))
    }
}

pub fn validate_model_provider_reference(provider_id: &str) -> Result<(), String> {
    validate_model_provider_id(provider_id)
}

pub fn model_provider_id_requirements() -> &'static str {
    MODEL_PROVIDER_ID_ALLOWED_CHARS
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn accepts_uppercase_provider_ids() {
        assert_eq!(validate_model_provider_id("OpenAI_Custom-2"), Ok(()));
    }

    #[test]
    fn rejects_empty_provider_ids() {
        assert_eq!(
            validate_model_provider_id(""),
            Err("Provider ID is required.".to_string())
        );
    }

    #[test]
    fn rejects_invalid_provider_id_characters() {
        assert_eq!(
            validate_model_provider_id("openai.custom"),
            Err("Provider ID must use ASCII letters, digits, '-' or '_'.".to_string())
        );
    }
}
