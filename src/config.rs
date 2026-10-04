use std::{
    collections::HashMap,
    env, fmt,
    path::{Path, PathBuf},
};

const DEFAULT_MODEL: &str = "gpt-4.1";
const DEFAULT_CONTEXT_TOKEN_LIMIT: usize = 16_000;
const DEFAULT_MAX_TOOL_CALLS: usize = 8;
const DEFAULT_MAX_TOOL_OUTPUT_BYTES: usize = 256 * 1024;
const APP_DIR_NAME: &str = "ben";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("OPENAI_API_KEY is not set")]
    MissingApiKey,
    #[error("OPENAI_API_KEY is empty")]
    EmptyApiKey,
    #[error("model identifier is empty")]
    EmptyModel,
    #[error("configuration value for {0} is invalid")]
    InvalidSetting(&'static str),
    #[error("no application data directory is available")]
    DataDirUnavailable,
}

pub struct SecretString(String);

impl SecretString {
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

#[derive(Debug)]
pub struct Config {
    pub model: String,
    pub context_token_limit: usize,
    pub max_tool_calls: usize,
    pub max_tool_output_bytes: usize,
    pub data_dir: PathBuf,
    api_key: SecretString,
}

impl Config {
    pub fn load(model_override: Option<&str>) -> Result<Self, ConfigError> {
        const SETTINGS: [&str; 8] = [
            "OPENAI_API_KEY",
            "BEN_MODEL",
            "BEN_CONTEXT_TOKEN_LIMIT",
            "BEN_MAX_TOOL_CALLS",
            "BEN_MAX_TOOL_OUTPUT_BYTES",
            "BEN_DATA_DIR",
            "XDG_DATA_HOME",
            "HOME",
        ];
        let mut values = HashMap::new();
        for key in SETTINGS {
            if let Some(value) = env::var_os(key) {
                let value = value
                    .into_string()
                    .map_err(|_| ConfigError::InvalidSetting(key))?;
                values.insert(key.to_owned(), value);
            }
        }
        Self::from_values(&values, model_override)
    }

    pub(crate) fn from_values(
        values: &HashMap<String, String>,
        model_override: Option<&str>,
    ) -> Result<Self, ConfigError> {
        let api_key = values
            .get("OPENAI_API_KEY")
            .ok_or(ConfigError::MissingApiKey)?;
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(ConfigError::EmptyApiKey);
        }

        let model = model_override
            .or_else(|| values.get("BEN_MODEL").map(String::as_str))
            .unwrap_or(DEFAULT_MODEL)
            .trim();
        if model.is_empty() {
            return Err(ConfigError::EmptyModel);
        }

        Ok(Self {
            model: model.to_owned(),
            context_token_limit: parse_limit(
                values,
                "BEN_CONTEXT_TOKEN_LIMIT",
                DEFAULT_CONTEXT_TOKEN_LIMIT,
                1_000_000,
            )?,
            max_tool_calls: parse_limit(values, "BEN_MAX_TOOL_CALLS", DEFAULT_MAX_TOOL_CALLS, 128)?,
            max_tool_output_bytes: parse_limit(
                values,
                "BEN_MAX_TOOL_OUTPUT_BYTES",
                DEFAULT_MAX_TOOL_OUTPUT_BYTES,
                16 * 1024 * 1024,
            )?,
            api_key: SecretString(api_key.to_owned()),
            data_dir: resolve_data_dir(values)?,
        })
    }

    pub fn api_key(&self) -> &SecretString {
        &self.api_key
    }
}

/// Local data root: an explicit override, then the XDG location, then the
/// platform default under the user's home directory.
fn resolve_data_dir(values: &HashMap<String, String>) -> Result<PathBuf, ConfigError> {
    let setting = |name: &str| {
        values
            .get(name)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
    };
    if let Some(directory) = setting("BEN_DATA_DIR") {
        return Ok(PathBuf::from(directory));
    }
    if let Some(base) = setting("XDG_DATA_HOME") {
        return Ok(Path::new(base).join(APP_DIR_NAME));
    }
    let home = setting("HOME").ok_or(ConfigError::DataDirUnavailable)?;
    Ok(Path::new(home)
        .join(".local")
        .join("share")
        .join(APP_DIR_NAME))
}

fn parse_limit(
    values: &HashMap<String, String>,
    name: &'static str,
    default: usize,
    maximum: usize,
) -> Result<usize, ConfigError> {
    let Some(value) = values.get(name) else {
        return Ok(default);
    };
    let parsed = value
        .parse::<usize>()
        .map_err(|_| ConfigError::InvalidSetting(name))?;
    if parsed == 0 || parsed > maximum {
        return Err(ConfigError::InvalidSetting(name));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigError};
    use std::{collections::HashMap, path::PathBuf};

    fn values(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    /// Every successful load needs a home directory to resolve local data from.
    fn with_home(entries: &[(&str, &str)]) -> HashMap<String, String> {
        let mut values = values(entries);
        values.insert("HOME".to_owned(), "/home/ben-tester".to_owned());
        values
    }

    #[test]
    fn reports_missing_api_key_with_setup_error() {
        let error = Config::from_values(&HashMap::new(), None).unwrap_err();

        assert!(matches!(error, ConfigError::MissingApiKey));
    }

    #[test]
    fn rejects_an_empty_api_key() {
        let env = values(&[("OPENAI_API_KEY", "  ")]);

        let error = Config::from_values(&env, None).unwrap_err();

        assert!(matches!(error, ConfigError::EmptyApiKey));
    }

    #[test]
    fn loads_model_and_validated_context_and_tool_limits() {
        let env = with_home(&[
            ("OPENAI_API_KEY", "test-secret-value"),
            ("BEN_MODEL", "environment-model"),
            ("BEN_CONTEXT_TOKEN_LIMIT", "12000"),
            ("BEN_MAX_TOOL_CALLS", "5"),
            ("BEN_MAX_TOOL_OUTPUT_BYTES", "4096"),
        ]);

        let config = Config::from_values(&env, Some("cli-model")).unwrap();

        assert_eq!(config.model, "cli-model");
        assert_eq!(config.context_token_limit, 12_000);
        assert_eq!(config.max_tool_calls, 5);
        assert_eq!(config.max_tool_output_bytes, 4096);
        assert_eq!(config.api_key().expose_secret(), "test-secret-value");
    }

    #[test]
    fn applies_documented_model_and_resource_limit_defaults() {
        let env = with_home(&[("OPENAI_API_KEY", "test-secret-value")]);

        let config = Config::from_values(&env, None).unwrap();

        assert_eq!(config.model, "gpt-4.1");
        assert_eq!(config.context_token_limit, 16_000);
        assert_eq!(config.max_tool_calls, 8);
        assert_eq!(config.max_tool_output_bytes, 256 * 1024);
    }

    #[test]
    fn debug_output_redacts_api_key_bytes() {
        let env = with_home(&[("OPENAI_API_KEY", "secret-must-not-appear")]);
        let config = Config::from_values(&env, None).unwrap();

        assert!(!format!("{config:?}").contains("secret-must-not-appear"));
    }

    #[test]
    fn data_dir_prefers_the_explicit_override() {
        let env = with_home(&[
            ("OPENAI_API_KEY", "test-secret-value"),
            ("BEN_DATA_DIR", "/srv/ben-data"),
            ("XDG_DATA_HOME", "/srv/xdg"),
        ]);

        let config = Config::from_values(&env, None).unwrap();

        assert_eq!(config.data_dir, PathBuf::from("/srv/ben-data"));
    }

    #[test]
    fn data_dir_falls_back_to_the_platform_locations() {
        let xdg = with_home(&[
            ("OPENAI_API_KEY", "test-secret-value"),
            ("XDG_DATA_HOME", "/srv/xdg"),
        ]);
        let home = values(&[
            ("OPENAI_API_KEY", "test-secret-value"),
            ("HOME", "/home/ben-tester"),
        ]);

        assert_eq!(
            Config::from_values(&xdg, None).unwrap().data_dir,
            PathBuf::from("/srv/xdg/ben")
        );
        assert_eq!(
            Config::from_values(&home, None).unwrap().data_dir,
            PathBuf::from("/home/ben-tester/.local/share/ben")
        );
    }

    #[test]
    fn data_dir_needs_a_directory_source() {
        let env = values(&[("OPENAI_API_KEY", "test-secret-value")]);

        assert!(matches!(
            Config::from_values(&env, None),
            Err(ConfigError::DataDirUnavailable)
        ));
    }
}
