//! Secret handling utilities.
//!
//! Per the configuration-security directive: secret values are never stored in
//! committed config files (only env-var *names* may be), never appear in
//! `Debug`/`Display` output, and are resolved through this module at load time.

use std::fmt;

/// A secret string that redacts itself in `Debug`/`Display` output.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Explicit accessor for the secret value. Only call sites that need the
    /// raw material (e.g. MAC computation) should use this.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            write!(f, "Secret(empty)")
        } else {
            write!(f, "Secret(***)")
        }
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Resolve a secret from an environment variable named by `env_var_name`.
///
/// Fail-closed: when an env-var name is configured, the variable MUST exist and
/// be non-empty — a missing variable is a configuration error, not a silent
/// fallthrough to an empty secret.
pub fn resolve_secret_from_env(env_var_name: &str) -> Result<Secret, String> {
    if env_var_name.is_empty() {
        return Err("empty environment variable name".to_string());
    }
    match std::env::var(env_var_name) {
        Ok(value) if !value.is_empty() => Ok(Secret::new(value)),
        Ok(_) => Err(format!(
            "environment variable {} is set but empty",
            env_var_name
        )),
        Err(std::env::VarError::NotPresent) => {
            Err(format!("environment variable {} is not set", env_var_name))
        }
        Err(e) => Err(format!(
            "environment variable {} unreadable: {}",
            env_var_name, e
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_leaks_value() {
        let secret = Secret::new("super-secret-value-123");
        let rendered = format!("{:?}", secret);
        assert!(!rendered.contains("super-secret-value-123"));
        assert!(rendered.contains("***"));
    }

    #[test]
    fn display_never_leaks_value() {
        let secret = Secret::new("super-secret-value-456");
        assert!(!format!("{}", secret).contains("super-secret-value-456"));
    }

    #[test]
    fn empty_secret_reports_empty() {
        assert!(Secret::new("").is_empty());
        assert_eq!(format!("{:?}", Secret::new("")), "Secret(empty)");
    }

    #[test]
    fn env_resolution_reads_value() {
        std::env::set_var("TEST_SECRET_RESOLVE_OK", "abc123");
        let s = resolve_secret_from_env("TEST_SECRET_RESOLVE_OK").expect("must resolve");
        assert_eq!(s.expose(), "abc123");
        std::env::remove_var("TEST_SECRET_RESOLVE_OK");
    }

    #[test]
    fn env_resolution_fails_closed_on_missing_var() {
        let err = resolve_secret_from_env("TEST_SECRET_DEFINITELY_NOT_SET_9137")
            .expect_err("missing var must error");
        assert!(err.contains("not set"));
    }

    #[test]
    fn env_resolution_fails_closed_on_empty_var() {
        std::env::set_var("TEST_SECRET_RESOLVE_EMPTY", "");
        let err =
            resolve_secret_from_env("TEST_SECRET_RESOLVE_EMPTY").expect_err("empty must error");
        assert!(err.contains("empty"));
        std::env::remove_var("TEST_SECRET_RESOLVE_EMPTY");
    }
}
