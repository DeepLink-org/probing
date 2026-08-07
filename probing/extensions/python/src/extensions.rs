mod pprof;
pub mod python;
mod torch;

use std::collections::HashMap;

use probing_core::core::EngineError;

pub use pprof::PprofProbeExtension;
pub use python::PythonExt;
pub use torch::TorchProbeExtension;

fn bool_param(
    params: &HashMap<String, String>,
    name: &str,
    default: bool,
) -> Result<bool, EngineError> {
    match params.get(name).map(|value| value.trim()) {
        None => Ok(default),
        Some("1") => Ok(true),
        Some("0") => Ok(false),
        Some(value) if value.eq_ignore_ascii_case("true") => Ok(true),
        Some(value) if value.eq_ignore_ascii_case("false") => Ok(false),
        Some(value) => Err(EngineError::InvalidCallParameter(
            name.to_string(),
            value.to_string(),
        )),
    }
}

fn optional_i64_param(
    params: &HashMap<String, String>,
    name: &str,
) -> Result<Option<i64>, EngineError> {
    params
        .get(name)
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| EngineError::InvalidCallParameter(name.to_string(), value.clone()))
        })
        .transpose()
}

fn one_of_param<'a>(
    params: &'a HashMap<String, String>,
    name: &str,
    default: &'a str,
    allowed: &[&str],
) -> Result<&'a str, EngineError> {
    let value = params.get(name).map(String::as_str).unwrap_or(default);
    if allowed.contains(&value) {
        Ok(value)
    } else {
        Err(EngineError::InvalidCallParameter(
            name.to_string(),
            value.to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_params_reject_invalid_values() {
        let params = HashMap::from([
            ("cluster".to_string(), "sometimes".to_string()),
            ("step".to_string(), "latest".to_string()),
            ("mode".to_string(), "native".to_string()),
        ]);

        assert!(matches!(
            bool_param(&params, "cluster", true),
            Err(EngineError::InvalidCallParameter(_, _))
        ));
        assert!(matches!(
            optional_i64_param(&params, "step"),
            Err(EngineError::InvalidCallParameter(_, _))
        ));
        assert!(matches!(
            one_of_param(&params, "mode", "mixed", &["mixed", "py"]),
            Err(EngineError::InvalidCallParameter(_, _))
        ));
    }

    #[test]
    fn query_params_accept_documented_values_and_defaults() {
        let params = HashMap::from([
            ("cluster".to_string(), "0".to_string()),
            ("step".to_string(), "42".to_string()),
        ]);

        assert!(!bool_param(&params, "cluster", true).unwrap());
        assert_eq!(optional_i64_param(&params, "step").unwrap(), Some(42));
        assert_eq!(
            one_of_param(&params, "mode", "mixed", &["mixed", "py"]).unwrap(),
            "mixed"
        );
    }
}
