//! The `gwtest/v1` spec format (M5, §16 v0.5 "own runner"): a declarative
//! `*.spec.toml` file whose scenarios are step strings in the M4 REPL
//! command language — the interactive debugger and the CI runner share one
//! command core, and every step maps onto an existing GWP request (no
//! protocol change).
//!
//! Parsing is strict: unknown keys, empty step lists, duplicate scenario
//! names and `bye`-family steps (the runner owns the session lifecycle)
//! are all errors naming the file and scenario.

use serde::Deserialize;

/// The file suffix the discovery scan matches (`*.spec.toml`).
pub const SPEC_SUFFIX: &str = ".spec.toml";

/// One parsed spec file (`gwtest/v1`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// Optional file-level GWP target (`uds:///…` / `tcp://host:port` /
    /// `ws://host:port`, the §5.3 `GUIWRIGHT_DRIVER` form). The CLI
    /// `--uds` / `--tcp` / `--ws` flags override it.
    pub target: Option<String>,
    /// Optional file-level resolve/action deadline (ms).
    pub timeout_ms: Option<u64>,
    /// Optional file-level retry count for failed scenarios.
    pub retries: Option<u32>,
    #[serde(rename = "scenario")]
    pub scenarios: Vec<Scenario>,
}

/// One scenario: a named, ordered step list.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Unique within the file (strict mode, §8.2 discipline).
    pub name: String,
    /// The steps, in the REPL command language (M4 §2.1).
    pub steps: Vec<String>,
    /// Per-scenario deadline override (beats the file value, loses to the CLI).
    pub timeout_ms: Option<u64>,
    /// Per-scenario retry override (beats the file value, loses to the CLI).
    pub retries: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SpecError {
    #[error("{path}: {message}")]
    Parse { path: String, message: String },
    #[error("{path}: scenario `{name}`: {message}")]
    Scenario {
        path: String,
        name: String,
        message: String,
    },
}

impl Spec {
    /// Parse and validate one spec file.
    pub fn parse(path: &str, text: &str) -> Result<Self, SpecError> {
        let spec: Spec = toml::from_str(text).map_err(|err| SpecError::Parse {
            path: path.to_string(),
            message: err.to_string(),
        })?;
        if spec.scenarios.is_empty() {
            return Err(SpecError::Parse {
                path: path.to_string(),
                message: "no [[scenario]] tables — a spec needs at least one scenario".into(),
            });
        }
        let mut seen = std::collections::HashSet::new();
        for scenario in &spec.scenarios {
            if scenario.name.trim().is_empty() {
                return Err(SpecError::Scenario {
                    path: path.to_string(),
                    name: scenario.name.clone(),
                    message: "scenario `name` must not be blank".into(),
                });
            }
            if !seen.insert(scenario.name.clone()) {
                return Err(SpecError::Scenario {
                    path: path.to_string(),
                    name: scenario.name.clone(),
                    message: "duplicate scenario name in the same file (strict mode)".into(),
                });
            }
            if scenario.steps.is_empty() {
                return Err(SpecError::Scenario {
                    path: path.to_string(),
                    name: scenario.name.clone(),
                    message: "no steps — a scenario needs at least one step".into(),
                });
            }
            for step in &scenario.steps {
                if step.trim().is_empty() {
                    return Err(SpecError::Scenario {
                        path: path.to_string(),
                        name: scenario.name.clone(),
                        message: "blank step".into(),
                    });
                }
                let first = step
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if matches!(first.as_str(), "bye" | "exit" | "quit") {
                    return Err(SpecError::Scenario {
                        path: path.to_string(),
                        name: scenario.name.clone(),
                        message: format!(
                            "`{first}` closes the session — the runner owns the session lifecycle"
                        ),
                    });
                }
            }
        }
        Ok(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Spec, SpecError> {
        Spec::parse("login.spec.toml", text)
    }

    const FULL: &str = r#"
target = "uds:///tmp/app.gwp"
timeout_ms = 5_000
retries = 1

[[scenario]]
name = "save the document"
steps = [
    "get role button Save",
    "click",
    "expect role text 'Saved'",
]

[[scenario]]
name = "reset path"
timeout_ms = 2_500
retries = 2
steps = [
    "mock reset default",
    "get role button Save",
]
"#;

    #[test]
    fn parses_full_spec_with_scenario_overrides() {
        let spec = parse(FULL).expect("full spec parses");
        assert_eq!(spec.target.as_deref(), Some("uds:///tmp/app.gwp"));
        assert_eq!(spec.timeout_ms, Some(5_000));
        assert_eq!(spec.retries, Some(1));
        assert_eq!(spec.scenarios.len(), 2);
        let first = &spec.scenarios[0];
        assert_eq!(first.name, "save the document");
        assert_eq!(first.steps.len(), 3);
        assert_eq!(first.steps[0], "get role button Save");
        assert_eq!(first.timeout_ms, None);
        let second = &spec.scenarios[1];
        assert_eq!(second.timeout_ms, Some(2_500));
        assert_eq!(second.retries, Some(2));
    }

    #[test]
    fn minimal_spec_defaults_everything_optional() {
        let spec = parse(
            r#"
[[scenario]]
name = "only"
steps = ["ping"]
"#,
        )
        .expect("minimal spec parses");
        assert_eq!(spec.target, None);
        assert_eq!(spec.timeout_ms, None);
        assert_eq!(spec.retries, None);
        assert_eq!(spec.scenarios.len(), 1);
        assert_eq!(spec.scenarios[0].steps, vec!["ping".to_string()]);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = parse(
            r#"
[[scenario]]
name = "x"
steps = ["ping"]
bogus = true
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("login.spec.toml"), "{err}");
        assert!(err.to_string().contains("bogus"), "{err}");
    }

    #[test]
    fn duplicate_scenario_names_are_rejected() {
        let err = parse(
            r#"
[[scenario]]
name = "same"
steps = ["ping"]

[[scenario]]
name = "same"
steps = ["ping"]
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("`same`"), "{err}");
    }

    #[test]
    fn scenario_without_steps_is_rejected() {
        let err = parse(
            r#"
[[scenario]]
name = "empty"
steps = []
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("`empty`"), "{err}");
        assert!(err.to_string().to_lowercase().contains("step"), "{err}");
    }

    #[test]
    fn spec_without_scenarios_is_rejected() {
        let err = parse("target = \"uds:///tmp/x.sock\"").unwrap_err();
        assert!(err.to_string().contains("scenario"), "{err}");
    }

    #[test]
    fn blank_or_whitespace_steps_are_rejected() {
        let err = parse(
            r#"
[[scenario]]
name = "x"
steps = ["ping", "   "]
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("`x`"), "{err}");
    }

    #[test]
    fn session_lifecycle_commands_are_rejected() {
        for cmd in ["bye", "exit", "quit", "BYE"] {
            let err = parse(&format!(
                r#"
[[scenario]]
name = "x"
steps = ["{cmd}"]
"#,
            ))
            .unwrap_err();
            assert!(err.to_string().contains("session"), "{err}");
        }
    }

    #[test]
    fn blank_scenario_name_is_rejected() {
        let err = parse(
            r#"
[[scenario]]
name = "  "
steps = ["ping"]
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("name"), "{err}");
    }

    #[test]
    fn malformed_toml_error_names_the_file() {
        let err = parse("this is not toml ][").unwrap_err();
        assert!(err.to_string().contains("login.spec.toml"), "{err}");
    }

    #[test]
    fn wrong_step_type_is_rejected_with_context() {
        let err = parse(
            r#"
[[scenario]]
name = "x"
steps = [42]
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("login.spec.toml"), "{err}");
    }

    #[test]
    fn negative_values_are_rejected() {
        let err = parse(
            r#"
retries = -1

[[scenario]]
name = "x"
steps = ["ping"]
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("login.spec.toml"), "{err}");
    }
}
