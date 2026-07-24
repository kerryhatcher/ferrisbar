use serde::de::DeserializeOwned;
use serde::Deserialize;

/// Deserializes a field as `None` instead of failing the whole document when
/// its value is present but has an unexpected JSON type (e.g. a number where
/// a string was expected). Missing keys and explicit `null` are still `None`
/// via serde's own `Option`/`#[serde(default)]` handling before this runs.
fn lenient_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: DeserializeOwned,
{
    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

#[derive(Deserialize, Default)]
pub struct Payload {
    #[serde(default, deserialize_with = "lenient_option")]
    pub model: Option<Model>,
    #[serde(default, deserialize_with = "lenient_option")]
    pub workspace: Option<Workspace>,
    #[serde(default, deserialize_with = "lenient_option")]
    pub session_id: Option<String>,
    #[serde(default, deserialize_with = "lenient_option")]
    pub context_window: Option<ContextWindow>,
}

#[derive(Deserialize, Default)]
pub struct Model {
    #[serde(default, deserialize_with = "lenient_option")]
    pub display_name: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct Workspace {
    #[serde(default, deserialize_with = "lenient_option")]
    pub current_dir: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ContextWindow {
    #[serde(default, deserialize_with = "lenient_option")]
    pub remaining_percentage: Option<f64>,
    #[serde(default, deserialize_with = "lenient_option")]
    pub total_tokens: Option<f64>,
}

impl Payload {
    pub fn model_name(&self) -> String {
        self.model
            .as_ref()
            .and_then(|m| m.display_name.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Claude".to_string())
    }

    pub fn cwd(&self, fallback: &str) -> String {
        self.workspace
            .as_ref()
            .and_then(|w| w.current_dir.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback.to_string())
    }

    pub fn session_id(&self) -> String {
        self.session_id.clone().unwrap_or_default()
    }

    pub fn remaining_percentage(&self) -> Option<f64> {
        self.context_window
            .as_ref()
            .and_then(|c| c.remaining_percentage)
    }

    pub fn total_tokens(&self) -> f64 {
        self.context_window
            .as_ref()
            .and_then(|c| c.total_tokens)
            .filter(|&t| t > 0.0)
            .unwrap_or(1_000_000.0)
    }
}

#[cfg(test)]
// These tests assert against exact literal fallback constants and
// JSON-literal pass-through values (not accumulated float arithmetic),
// so exact equality is the correct check, not a fuzzy comparison.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn model_name_defaults_to_claude_when_missing() {
        let payload: Payload = serde_json::from_str("{}").unwrap();
        assert_eq!(payload.model_name(), "Claude");
    }

    #[test]
    fn model_name_defaults_to_claude_when_empty_string() {
        let payload: Payload = serde_json::from_str(r#"{"model":{"display_name":""}}"#).unwrap();
        assert_eq!(payload.model_name(), "Claude");
    }

    #[test]
    fn model_name_uses_display_name_when_present() {
        let payload: Payload =
            serde_json::from_str(r#"{"model":{"display_name":"Sonnet"}}"#).unwrap();
        assert_eq!(payload.model_name(), "Sonnet");
    }

    #[test]
    fn cwd_falls_back_when_missing() {
        let payload: Payload = serde_json::from_str("{}").unwrap();
        assert_eq!(payload.cwd("/fallback"), "/fallback");
    }

    #[test]
    fn cwd_uses_workspace_current_dir_when_present() {
        let payload: Payload =
            serde_json::from_str(r#"{"workspace":{"current_dir":"/tmp/foo"}}"#).unwrap();
        assert_eq!(payload.cwd("/fallback"), "/tmp/foo");
    }

    #[test]
    fn session_id_defaults_to_empty_string() {
        let payload: Payload = serde_json::from_str("{}").unwrap();
        assert_eq!(payload.session_id(), "");
    }

    #[test]
    fn remaining_percentage_none_when_missing() {
        let payload: Payload = serde_json::from_str("{}").unwrap();
        assert_eq!(payload.remaining_percentage(), None);
    }

    #[test]
    fn remaining_percentage_present() {
        let payload: Payload =
            serde_json::from_str(r#"{"context_window":{"remaining_percentage":42.5}}"#).unwrap();
        assert_eq!(payload.remaining_percentage(), Some(42.5));
    }

    #[test]
    fn total_tokens_defaults_to_one_million() {
        let payload: Payload = serde_json::from_str("{}").unwrap();
        assert_eq!(payload.total_tokens(), 1_000_000.0);
    }

    #[test]
    fn total_tokens_uses_value_when_present() {
        let payload: Payload =
            serde_json::from_str(r#"{"context_window":{"total_tokens":50000}}"#).unwrap();
        assert_eq!(payload.total_tokens(), 50_000.0);
    }

    #[test]
    fn total_tokens_falls_back_when_zero() {
        let payload: Payload =
            serde_json::from_str(r#"{"context_window":{"total_tokens":0}}"#).unwrap();
        assert_eq!(payload.total_tokens(), 1_000_000.0);
    }

    #[test]
    fn other_top_level_fields_survive_when_session_id_has_wrong_type() {
        let payload: Payload = serde_json::from_str(
            r#"{"model":{"display_name":"Sonnet"},"workspace":{"current_dir":"/tmp/foo"},"session_id":123}"#,
        )
        .unwrap();
        assert_eq!(payload.model_name(), "Sonnet");
        assert_eq!(payload.cwd("/fallback"), "/tmp/foo");
        assert_eq!(payload.session_id(), "");
    }

    #[test]
    fn other_top_level_fields_survive_when_context_window_has_wrong_type() {
        let payload: Payload = serde_json::from_str(
            r#"{"model":{"display_name":"Sonnet"},"context_window":"not an object"}"#,
        )
        .unwrap();
        assert_eq!(payload.model_name(), "Sonnet");
        assert_eq!(payload.remaining_percentage(), None);
    }

    #[test]
    fn total_tokens_wrong_type_falls_back_but_remaining_percentage_still_parses() {
        let payload: Payload = serde_json::from_str(
            r#"{"context_window":{"remaining_percentage":42.5,"total_tokens":"oops"}}"#,
        )
        .unwrap();
        assert_eq!(payload.remaining_percentage(), Some(42.5));
        assert_eq!(payload.total_tokens(), 1_000_000.0);
    }

    #[test]
    fn display_name_wrong_type_falls_back_but_workspace_still_parses() {
        let payload: Payload = serde_json::from_str(
            r#"{"model":{"display_name":123},"workspace":{"current_dir":"/tmp/foo"}}"#,
        )
        .unwrap();
        assert_eq!(payload.model_name(), "Claude");
        assert_eq!(payload.cwd("/fallback"), "/tmp/foo");
    }
}
