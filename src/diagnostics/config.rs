use std::{env, ffi::OsString, fmt, path::PathBuf};

pub const ENV_DIAGNOSTICS_ENABLE: &str = "TM_DIAGNOSTICS_ENABLE";
pub const ENV_DIAGNOSTICS_PIPE_NAME: &str = "TM_DIAGNOSTICS_PIPE_NAME";
pub const ENV_DIAGNOSTICS_TOKEN: &str = "TM_DIAGNOSTICS_TOKEN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticConfig {
    pub pipe_name: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticConfigError {
    MissingPipeName,
    MissingToken,
    InvalidPipeName,
}

impl fmt::Display for DiagnosticConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiagnosticConfigError::MissingPipeName => {
                write!(
                    f,
                    "{ENV_DIAGNOSTICS_PIPE_NAME} is required when diagnostics are enabled"
                )
            }
            DiagnosticConfigError::MissingToken => {
                write!(
                    f,
                    "{ENV_DIAGNOSTICS_TOKEN} is required when diagnostics are enabled"
                )
            }
            DiagnosticConfigError::InvalidPipeName => {
                write!(
                    f,
                    "{ENV_DIAGNOSTICS_PIPE_NAME} must be a per-run Windows named pipe path or name"
                )
            }
        }
    }
}

impl std::error::Error for DiagnosticConfigError {}

impl DiagnosticConfig {
    pub fn from_env() -> Result<Option<Self>, DiagnosticConfigError> {
        Self::from_values(
            env::var_os(ENV_DIAGNOSTICS_ENABLE),
            env::var_os(ENV_DIAGNOSTICS_PIPE_NAME),
            env::var_os(ENV_DIAGNOSTICS_TOKEN),
        )
    }

    pub fn from_values(
        enable: Option<OsString>,
        pipe_name: Option<OsString>,
        token: Option<OsString>,
    ) -> Result<Option<Self>, DiagnosticConfigError> {
        if !is_enabled(enable.as_ref()) {
            return Ok(None);
        }

        let pipe_name =
            os_string_to_trimmed(pipe_name).ok_or(DiagnosticConfigError::MissingPipeName)?;
        if !is_valid_pipe_name(&pipe_name) {
            return Err(DiagnosticConfigError::InvalidPipeName);
        }

        let token = os_string_to_trimmed(token).ok_or(DiagnosticConfigError::MissingToken)?;

        Ok(Some(Self { pipe_name, token }))
    }

    pub fn pipe_path(&self) -> PathBuf {
        if self.pipe_name.starts_with(r"\\.\pipe\") {
            PathBuf::from(&self.pipe_name)
        } else {
            PathBuf::from(format!(r"\\.\pipe\{}", self.pipe_name))
        }
    }
}

fn is_enabled(value: Option<&OsString>) -> bool {
    value
        .and_then(|raw| raw.to_str())
        .map(|raw| {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn os_string_to_trimmed(value: Option<OsString>) -> Option<String> {
    value
        .and_then(|raw| raw.into_string().ok())
        .map(|raw| raw.trim().to_owned())
        .filter(|raw| !raw.is_empty())
}

fn is_valid_pipe_name(pipe_name: &str) -> bool {
    if pipe_name.starts_with(r"\\.\pipe\") {
        pipe_name.len() > r"\\.\pipe\".len()
    } else {
        !pipe_name.contains('\\') && !pipe_name.contains('/')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_are_disabled_by_default_even_when_pipe_values_are_present() {
        let config = DiagnosticConfig::from_values(
            None,
            Some(OsString::from("tm-diagnostics-test")),
            Some(OsString::from("secret")),
        )
        .expect("parse");

        assert_eq!(config, None);
    }

    #[test]
    fn diagnostics_require_explicit_enable_pipe_name_and_token() {
        let missing_pipe = DiagnosticConfig::from_values(
            Some(OsString::from("1")),
            None,
            Some(OsString::from("secret")),
        )
        .expect_err("enabled diagnostics without pipe name must fail");
        assert_eq!(missing_pipe, DiagnosticConfigError::MissingPipeName);

        let missing_token = DiagnosticConfig::from_values(
            Some(OsString::from("true")),
            Some(OsString::from("tm-diagnostics-test")),
            None,
        )
        .expect_err("enabled diagnostics without token must fail");
        assert_eq!(missing_token, DiagnosticConfigError::MissingToken);

        let config = DiagnosticConfig::from_values(
            Some(OsString::from("on")),
            Some(OsString::from("tm-diagnostics-test")),
            Some(OsString::from("secret")),
        )
        .expect("parse")
        .expect("enabled config");
        assert_eq!(
            config.pipe_path(),
            PathBuf::from(r"\\.\pipe\tm-diagnostics-test")
        );
        assert_eq!(config.token, "secret");
    }

    #[test]
    fn diagnostics_accept_full_windows_pipe_paths() {
        let config = DiagnosticConfig::from_values(
            Some(OsString::from("1")),
            Some(OsString::from(r"\\.\pipe\tm-diagnostics-full-path")),
            Some(OsString::from("secret")),
        )
        .expect("parse")
        .expect("enabled config");

        assert_eq!(
            config.pipe_path(),
            PathBuf::from(r"\\.\pipe\tm-diagnostics-full-path")
        );
    }
}
