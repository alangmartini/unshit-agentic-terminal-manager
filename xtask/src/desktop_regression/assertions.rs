use terminal_manager_diagnostics::FailureClassification;
use terminal_manager_diagnostics::SuiteFailure;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteError {
    pub kind: FailureClassification,
    pub message: String,
    pub first_bad_signal: Option<String>,
}

impl SuiteError {
    pub fn setup(message: impl Into<String>) -> Self {
        Self {
            kind: FailureClassification::Setup,
            message: message.into(),
            first_bad_signal: None,
        }
    }

    pub fn assertion(message: impl Into<String>, first_bad_signal: impl Into<String>) -> Self {
        Self {
            kind: FailureClassification::Assertion,
            message: message.into(),
            first_bad_signal: Some(first_bad_signal.into()),
        }
    }

    pub fn protocol(message: impl Into<String>, first_bad_signal: impl Into<String>) -> Self {
        Self {
            kind: FailureClassification::Protocol,
            message: message.into(),
            first_bad_signal: Some(first_bad_signal.into()),
        }
    }

    pub fn cross_layer(message: impl Into<String>, first_bad_signal: impl Into<String>) -> Self {
        Self {
            kind: FailureClassification::CrossLayerInvariant,
            message: message.into(),
            first_bad_signal: Some(first_bad_signal.into()),
        }
    }

    pub fn to_suite_failure(&self) -> SuiteFailure {
        SuiteFailure {
            kind: self.kind,
            message: self.message.clone(),
            first_bad_signal: self.first_bad_signal.clone(),
        }
    }
}

pub type SuiteResult<T> = Result<T, SuiteError>;

pub fn assert_close(actual: i32, expected: i32, tolerance: i32, name: &str) -> SuiteResult<()> {
    let delta = (actual - expected).abs();
    if delta > tolerance {
        return Err(SuiteError::assertion(
            format!("{name} differs by {delta} > {tolerance}"),
            name,
        ));
    }

    Ok(())
}

pub fn assert_true(condition: bool, message: &str, first_bad_signal: &str) -> SuiteResult<()> {
    if !condition {
        return Err(SuiteError::assertion(message, first_bad_signal));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_assertion_allows_values_inside_tolerance() {
        assert!(assert_close(102, 100, 2, "right-edge").is_ok());
    }

    #[test]
    fn close_assertion_classifies_values_outside_tolerance() {
        let err = assert_close(104, 100, 2, "right-edge").unwrap_err();

        assert_eq!(err.kind, FailureClassification::Assertion);
        assert!(err.message.contains("right-edge"));
        assert_eq!(err.first_bad_signal.as_deref(), Some("right-edge"));
    }
}
