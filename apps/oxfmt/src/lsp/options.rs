use serde::{Deserialize, Deserializer, Serialize, de::Error};
use serde_json::Value;

use crate::core::config_discovery;

#[derive(Debug, Default, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FormatOptions {
    /// An empty string is treated as unset.
    pub config_path: Option<String>,
    pub language: Option<LspLanguage>,
    pub disable_nested_config: bool,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
pub enum LspLanguage {
    EtsStatic,
}

impl LspLanguage {
    pub const fn explicit(self) -> oxc_span::ExplicitLanguage {
        match self {
            Self::EtsStatic => oxc_span::ExplicitLanguage::EtsStatic,
        }
    }
}

impl FormatOptions {
    /// Nested config search: off with `fmt.disableNestedConfig`, an explicit `fmt.configPath`, or in Vite+ mode.
    pub fn use_nested_configs(&self) -> bool {
        !self.disable_nested_config
            && self.config_path.is_none()
            && config_discovery().nested_configs()
    }
}

impl<'de> Deserialize<'de> for FormatOptions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        FormatOptions::try_from(value).map_err(Error::custom)
    }
}

impl TryFrom<Value> for FormatOptions {
    type Error = String;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        // null is treated as default options
        if value == Value::Null {
            return Ok(Self::default());
        }

        let Some(object) = value.as_object() else {
            return Err("no object passed".to_string());
        };

        Ok(Self {
            config_path: object
                .get("fmt.configPath")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            language: object
                .get("fmt.language")
                .and_then(Value::as_str)
                .map(|language| match language {
                    "ets-static" => Ok(LspLanguage::EtsStatic),
                    _ => Err(format!(
                        "Unknown language '{language}'. Supported explicit languages: ets-static."
                    )),
                })
                .transpose()?,
            disable_nested_config: object
                .get("fmt.disableNestedConfig")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use super::FormatOptions;

    #[test]
    fn test_valid_options_json() {
        let json = json!({
            "fmt.configPath": "./.oxfmtrc.json",
            "fmt.disableNestedConfig": true
        });

        let options = FormatOptions::try_from(json).unwrap();
        assert_eq!(options.config_path.unwrap(), "./.oxfmtrc.json");
        assert!(options.language.is_none());
        assert!(options.disable_nested_config);
    }

    #[test]
    fn test_empty_options_json() {
        let json = json!({});

        let options = FormatOptions::try_from(json).unwrap();
        assert!(options.config_path.is_none());
        assert!(!options.disable_nested_config);
    }

    #[test]
    fn test_null_json() {
        let json = json!(null);
        let options = FormatOptions::try_from(json).unwrap();
        assert_eq!(options, FormatOptions::default());
    }

    #[test]
    fn test_invalid_options_json() {
        let json = json!({
            "fmt.configPath": true, // should be a string
            "fmt.disableNestedConfig": "true" // should be a boolean
        });

        let options = FormatOptions::try_from(json).unwrap();
        assert!(options.config_path.is_none());
        assert!(!options.disable_nested_config);
    }

    #[test]
    fn test_empty_string_config_path() {
        let json = json!({
            "fmt.configPath": ""
        });

        let options = FormatOptions::try_from(json).unwrap();
        assert_eq!(options.config_path, None);
    }

    #[test]
    fn test_use_nested_configs() {
        let options = FormatOptions::default();
        assert!(options.use_nested_configs());

        let options =
            FormatOptions { config_path: Some("config.json".into()), ..Default::default() };
        assert!(!options.use_nested_configs());

        let options = FormatOptions { disable_nested_config: true, ..Default::default() };
        assert!(!options.use_nested_configs());
    }

    #[test]
    fn test_static_ets_language() {
        let options = FormatOptions::try_from(json!({ "fmt.language": "ets-static" })).unwrap();
        assert_eq!(options.language, Some(super::LspLanguage::EtsStatic));
    }

    #[test]
    fn test_unknown_language() {
        let error = FormatOptions::try_from(json!({ "fmt.language": "ets" })).unwrap_err();
        assert!(error.contains("ets-static"));
    }
}
