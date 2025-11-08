use std::collections::BTreeMap;

use once_cell::sync::Lazy;
use probing_proto::prelude::Ele;
use tokio::sync::RwLock;

use crate::core::{EngineError, EngineExtensionManager};
use crate::ENGINE;

/// Global configuration key-value store.
pub static CONFIG_STORE: Lazy<RwLock<BTreeMap<String, Ele>>> =
    Lazy::new(|| RwLock::new(BTreeMap::new()));

/// Get a configuration value.
pub async fn get(key: &str) -> Option<Ele> {
    CONFIG_STORE.read().await.get(key).cloned()
}

/// Set a configuration value.
pub async fn set<T: Into<Ele>>(key: &str, value: T) {
    CONFIG_STORE
        .write()
        .await
        .insert(key.to_string(), value.into());
}

/// Get a configuration value as string.
pub async fn get_str(key: &str) -> Option<String> {
    get(key).await.map(|ele| match ele {
        Ele::Text(s) => s,
        Ele::BOOL(b) => {
            if b {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        Ele::I32(i) => i.to_string(),
        Ele::I64(i) => i.to_string(),
        Ele::F32(f) => f.to_string(),
        Ele::F64(f) => f.to_string(),
        Ele::DataTime(t) => t.to_string(),
        Ele::Nil => "nil".to_string(),
        Ele::Url(u) => u,
    })
}

/// Remove a configuration value.
pub async fn remove(key: &str) -> Option<Ele> {
    CONFIG_STORE.write().await.remove(key)
}

/// Check if a key exists.
pub async fn contains_key(key: &str) -> bool {
    CONFIG_STORE.read().await.contains_key(key)
}

/// Get all configuration keys.
pub async fn keys() -> Vec<String> {
    CONFIG_STORE.read().await.keys().cloned().collect()
}

/// Clear all configuration.
pub async fn clear() {
    CONFIG_STORE.write().await.clear();
}

/// Get the number of configuration entries.
pub async fn len() -> usize {
    CONFIG_STORE.read().await.len()
}

/// Check if the configuration store is empty.
pub async fn is_empty() -> bool {
    CONFIG_STORE.read().await.is_empty()
}

/// Set a configuration option through the engine extension system.
///
/// If the key starts with "probing", it will attempt to update the engine's
/// extension configuration first. Otherwise, it directly updates the configuration store.
///
/// # Examples
/// ```rust
/// probing_core::config::write("server.address", "0.0.0.0:8080")?;
/// probing_core::config::write("taskstats.interval", "1000")?;
/// probing_core::config::write("server.debug", "true")?;
/// # Ok::<(), probing_core::core::EngineError>(())
/// ```
pub async fn write(key: &str, value: &str) -> Result<(), EngineError> {
    if key.starts_with("probing") {
        let engine_guard = ENGINE.write().await;
        let mut state = engine_guard.context.state();

        if let Some(eem) = state
            .config_mut()
            .options_mut()
            .extensions
            .get_mut::<EngineExtensionManager>()
        {
            let extension_key = if key.starts_with("probing.") {
                &key[8..]
            } else {
                key
            };

            // Attempt to set the option on an extension.
            match eem.set_option(extension_key, value).await {
                Ok(_) => {
                    // If successful, also update the global config store.
                    set(key, value).await;
                    return Ok(());
                }
                Err(EngineError::UnsupportedOption(_)) => {
                    // If unsupported by any extension, just write to the config store.
                    // This allows for configs that don't belong to an extension.
                }
                Err(e) => {
                    // For any other error, propagate it and do not write to the config store.
                    return Err(e);
                }
            }
        }
    }

    // For non-"probing" keys or unsupported "probing" keys, write to the store.
    set(key, value).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{EngineCall, EngineDatasource, EngineExtension, EngineExtensionOption};
    use crate::{create_engine, initialize_engine};

    async fn setup_test() {
        clear().await;
    }

    async fn teardown_test() {
        clear().await;
    }

    #[derive(Debug)]
    struct TestExtension {
        test_option: String,
    }

    impl Default for TestExtension {
        fn default() -> Self {
            Self {
                test_option: "default".to_string(),
            }
        }
    }

    impl EngineCall for TestExtension {}
    impl EngineDatasource for TestExtension {}

    impl EngineExtension for TestExtension {
        fn name(&self) -> String {
            "test".to_string()
        }

        fn set(&mut self, key: &str, value: &str) -> Result<String, EngineError> {
            match key {
                "option" => {
                    let old = self.test_option.clone();
                    self.test_option = value.to_string();
                    Ok(old)
                }
                _ => Err(EngineError::UnsupportedOption(key.to_string())),
            }
        }

        fn get(&self, key: &str) -> Result<String, EngineError> {
            match key {
                "option" => Ok(self.test_option.clone()),
                _ => Err(EngineError::UnsupportedOption(key.to_string())),
            }
        }

        fn options(&self) -> Vec<EngineExtensionOption> {
            vec![EngineExtensionOption {
                key: "option".to_string(),
                value: Some(self.test_option.clone()),
                help: "Test option",
            }]
        }
    }

    #[tokio::test]
    async fn test_config_set_syncs_to_config_store() {
        setup_test().await;

        let builder = create_engine().with_extension(TestExtension::default(), "test", None);
        initialize_engine(builder)
            .await
            .expect("Failed to initialize engine");

        write("test.option", "new_value").await.unwrap();

        let value = get_str("test.option").await;
        assert_eq!(value, Some("new_value".to_string()));

        teardown_test().await;
    }

    #[tokio::test]
    async fn test_config_set_with_probing_prefix_updates_engine() {
        setup_test().await;

        let builder = create_engine();
        initialize_engine(builder)
            .await
            .expect("Failed to initialize engine");

        let _result = write("probing.test.key", "test_value").await;
        let _value = get_str("probing.test.key").await;

        teardown_test().await;
    }

    #[tokio::test]
    async fn test_config_get_from_config_store() {
        setup_test().await;

        let builder = create_engine().with_extension(TestExtension::default(), "test", None);
        initialize_engine(builder)
            .await
            .expect("Failed to initialize engine");

        write("test.option", "stored_value").await.unwrap();

        let store_value = get_str("test.option").await;
        assert_eq!(store_value, Some("stored_value".to_string()));

        let value = get_str("test.option").await.unwrap();
        assert_eq!(value, "stored_value");

        teardown_test().await;
    }

    #[tokio::test]
    async fn test_config_set_updates_extension_and_store() {
        setup_test().await;

        let builder = create_engine().with_extension(TestExtension::default(), "test", None);
        initialize_engine(builder)
            .await
            .expect("Failed to initialize engine");

        write("test.option", "extension_value").await.unwrap();

        let store_value = get_str("test.option").await;
        assert_eq!(store_value, Some("extension_value".to_string()));

        let value = get_str("test.option").await.unwrap();
        assert_eq!(value, "extension_value");

        teardown_test().await;
    }

    #[tokio::test]
    async fn test_config_set_engine_not_initialized() {
        setup_test().await;

        let _result = write("test.nonexistent", "value").await;

        teardown_test().await;
    }
}
