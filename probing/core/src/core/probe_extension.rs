use std::collections::BTreeMap;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt::Debug;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::config::{ConfigExtension, ExtensionOptions};
use tokio::sync::{Mutex, RwLock};

use super::error::EngineError;
use crate::config;

/// Shared probe extension instances keyed by extension name.
pub type ProbeExtensionMap = BTreeMap<String, Arc<Mutex<dyn ProbeExtension + Send + Sync>>>;

#[derive(Clone, Debug, Default)]
pub enum Maybe<T> {
    Just(T),
    #[default]
    Nothing,
}

impl<T: FromStr> FromStr for Maybe<T> {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            Ok(Maybe::Nothing)
        } else {
            match s.parse() {
                Ok(v) => Ok(Maybe::Just(v)),
                Err(_) => Ok(Maybe::Nothing),
            }
        }
    }
}

impl<T: Display> Display for Maybe<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            Maybe::Just(s) => write!(f, "{s}"),
            Maybe::Nothing => write!(f, ""),
        }
    }
}

impl<T> From<Maybe<T>> for Option<T> {
    fn from(val: Maybe<T>) -> Self {
        match val {
            Maybe::Just(v) => Some(v),
            Maybe::Nothing => None,
        }
    }
}

impl<T: Display> From<Maybe<T>> for String {
    fn from(value: Maybe<T>) -> Self {
        match value {
            Maybe::Just(v) => v.to_string(),
            Maybe::Nothing => "".to_string(),
        }
    }
}

/// Represents a configuration option for an engine extension.
///
/// # Fields
/// * `key` - The unique identifier for this option
/// * `value` - The current value of the option, if set
/// * `help` - Static help text describing the purpose and usage of this option
pub struct ProbeExtensionOption {
    pub key: String,
    pub value: Option<String>,
    pub help: &'static str,
}

/// Body plus transport-neutral completeness metadata returned by an extension.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProbeExtensionResponse {
    pub body: Vec<u8>,
    /// True when the body is usable but omits data from one or more peers.
    pub partial: bool,
}

impl From<Vec<u8>> for ProbeExtensionResponse {
    fn from(body: Vec<u8>) -> Self {
        Self {
            body,
            partial: false,
        }
    }
}

/// Extension trait for handling HTTP API calls
#[allow(unused)]
#[async_trait]
pub trait ProbeExtensionCall: Debug + Send + Sync {
    /// Handle API calls to the extension
    ///
    /// # Arguments
    /// * `path` - The path component of the API call
    /// * `params` - URL query parameters
    /// * `body` - Request body data
    ///
    /// # Returns
    /// * `Ok(Vec<u8>)` - Response data on success
    /// * `Err(EngineError)` - Error information on failure
    async fn call(
        &self,
        path: &str,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<Vec<u8>, EngineError> {
        Err(EngineError::UnsupportedCall)
    }

    /// Handle an API call while preserving response metadata for HTTP callers.
    async fn call_response(
        &self,
        path: &str,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<ProbeExtensionResponse, EngineError> {
        self.call(path, params, body).await.map(Into::into)
    }
}

/// Configurable Probing extension: HTTP calls, SET options, and runtime side effects.
///
/// SQL catalog registration is separate — use [`ProbeDataSource`] via
/// [`super::engine::EngineBuilder::with_data_source`].
#[allow(unused)]
pub trait ProbeExtension: Debug + Send + Sync + ProbeExtensionCall {
    fn name(&self) -> String;
    fn set(&mut self, key: &str, _value: &str) -> Result<String, EngineError> {
        Err(EngineError::UnsupportedOption(key.to_string()))
    }
    fn get(&self, key: &str) -> Result<String, EngineError> {
        Err(EngineError::UnsupportedOption(key.to_string()))
    }
    fn options(&self) -> Vec<ProbeExtensionOption> {
        Vec::new()
    }
}

/// Engine extension management module for configurable functionality.
///
/// This module provides a flexible extension system that allows for runtime configuration
/// of engine components through a key-value interface. It consists of three main components:
///
/// - [`ProbeExtensionOption`]: Represents a single configuration option with metadata
/// - [`ProbeExtension`]: A trait that must be implemented by configurable extensions
/// - [`ProbeExtensionManager`]: Manages multiple extensions and their configurations
///
/// The extension system integrates with DataFusion's configuration framework through
/// implementations of [`ConfigExtension`] and [`ExtensionOptions`].
///
/// # Examples
///
/// ```rust
/// use std::sync::Arc;
/// use tokio::sync::Mutex;
/// use probing_core::core::ProbeExtensionManager;
/// use probing_core::core::{ProbeExtension, ProbeExtensionOption, ProbeExtensionCall, EngineError};
///
/// #[derive(Debug)]
/// struct MyExtension {
///     some_option: String
/// }
///
/// impl ProbeExtensionCall for MyExtension {}
///
/// impl ProbeExtension for MyExtension {
///     fn name(&self) -> String {
///         "my_extension".to_string() // This name is used to form the option namespace
///     }
///
///     fn set(&mut self, key: &str, value: &str) -> Result<String, EngineError> {
///         match key {
///             "some_option" => { // This is the local option key within the extension
///                 let old = self.some_option.clone();
///                 self.some_option = value.to_string();
///                 Ok(old)
///             }
///             _ => Err(EngineError::UnsupportedOption(key.to_string()))
///         }
///     }
///
///     fn get(&self, key: &str) -> Result<String, EngineError> {
///         match key {
///             "some_option" => Ok(self.some_option.clone()), // Local option key
///             _ => Err(EngineError::UnsupportedOption(key.to_string()))
///         }
///     }
///
///     fn options(&self) -> Vec<ProbeExtensionOption> {
///         vec![
///             ProbeExtensionOption {
///                 key: "some_option".to_string(), // Local option key
///                 value: Some(self.some_option.clone()),
///                 help: "An example option"
///             }
///         ]
///     }
/// }
///
/// // This example demonstrates usage within an async context.
/// # async fn manager_usage_example() -> Result<(), EngineError> {
///     let mut manager = ProbeExtensionManager::default();
///     // Register extensions. The first argument "my_ext_instance_key" is an internal key for the manager
///     // and does not directly affect option key formation for set_option/get_option.
///     manager.register(
///         "my_ext_instance_key".to_string(),
///         Arc::new(Mutex::new(MyExtension { some_option: "default".to_string() }))
///     );
///
///     // Configure extensions. The option key is "<extension_name>.<local_option_key>".
///     // MyExtension::name() returns "my_extension". The local key is "some_option".
///     // The manager derives the namespace "my_extension." from MyExtension::name().
///     manager.set_option("my_extension.some_option", "new").await?;
///     assert_eq!(manager.get_option("my_extension.some_option").await?, "new");
///
///     // List all available options. manager.options() returns options with their local keys.
///     let options_list = manager.options().await;
///     assert!(!options_list.is_empty(), "Options list should not be empty");
///     if !options_list.is_empty() {
///         assert_eq!(options_list[0].key, "some_option"); // Key is "some_option" as returned by MyExtension::options
///         assert_eq!(options_list[0].value, Some("new".to_string())); // Value reflects the update
///     }
///     Ok(())
/// # }
///
/// // To run this example (e.g., in a test or main function):
/// // fn main() {
/// //     let rt = tokio::runtime::Runtime::new().unwrap();
/// //     rt.block_on(manager_usage_example()).unwrap();
/// // }
/// // Or if used in a #[tokio::test] or #[tokio::main] annotated function:
/// // manager_usage_example().await.unwrap();
/// ```
/// Engine-scoped extension manager.
///
/// Clones share one engine's registry, while independently built engines keep
/// separate extension instances and configuration state.
#[derive(Clone, Debug)]
pub struct ProbeExtensionManager {
    extensions: Arc<RwLock<ProbeExtensionMap>>,
}

impl Default for ProbeExtensionManager {
    fn default() -> Self {
        Self {
            extensions: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }
}

impl ProbeExtensionManager {
    /// Register an extension in this manager's engine-scoped registry.
    pub async fn register(
        &mut self,
        name: String,
        extension: Arc<Mutex<dyn ProbeExtension + Send + Sync>>,
    ) {
        self.extensions.write().await.insert(name, extension);
    }

    /// Extract namespace from extension name by removing "extension" suffix and converting to lowercase
    fn extract_namespace(extension_name: &str) -> String {
        let mut namespace = extension_name.to_lowercase();
        if namespace.ends_with("extension") {
            namespace.truncate(namespace.len() - "extension".len());
        }
        format!("{namespace}.")
    }

    /// Set an option (core implementation).
    ///
    /// This is the core implementation that updates extension configuration.
    /// ConfigStore is not updated by this method.
    pub async fn set_option(&mut self, key: &str, value: &str) -> Result<(), EngineError> {
        let extensions_clone: Vec<_> = {
            let extensions = self.extensions.read().await;
            extensions.values().cloned().collect()
        }; // Lock is released here

        for extension in extensions_clone {
            let namespace = {
                let ext = extension.lock().await;
                Self::extract_namespace(&ext.name())
            };

            if !key.starts_with(&namespace) {
                continue;
            }

            let local_key = key.trim_start_matches(&namespace).to_string();
            let result = {
                let mut ext = extension.lock().await;
                ext.set(&local_key, value)
            };

            match result {
                Ok(old) => {
                    log::info!(
                        "setting update [{}]:{local_key}={value} <= {old}",
                        namespace.trim_end_matches('.')
                    );
                    return Ok(());
                }
                Err(EngineError::UnsupportedOption(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(EngineError::UnsupportedOption(key.to_string()))
    }

    /// Set an option and update ConfigStore.
    ///
    /// This is a convenience wrapper that calls `set_option`
    /// and then updates ConfigStore.
    pub async fn set_option_with_store_update(
        &mut self,
        key: &str,
        value: &str,
    ) -> Result<(), EngineError> {
        self.set_option(key, value).await?;
        // Update ConfigStore after successfully updating the extension
        config::set(key, value).await;
        Ok(())
    }

    pub async fn get_option(&self, key: &str) -> Result<String, EngineError> {
        let extensions_clone: Vec<_> = {
            let extensions = self.extensions.read().await;
            extensions.values().cloned().collect()
        }; // Lock is released here

        for extension in extensions_clone {
            let ext = extension.lock().await;
            let namespace = Self::extract_namespace(&ext.name());
            if !key.starts_with(&namespace) {
                continue;
            }
            let local_key = key.trim_start_matches(&namespace);
            match ext.get(local_key) {
                Ok(value) => {
                    log::info!("setting read [{}]:{local_key}={value}", ext.name());
                    return Ok(value);
                }
                Err(EngineError::UnsupportedOption(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(EngineError::UnsupportedOption(key.to_string()))
    }

    pub async fn options(&self) -> Vec<ProbeExtensionOption> {
        let mut all_options = Vec::new();
        let extensions_clone: Vec<_> = {
            let extensions = self.extensions.read().await;
            extensions.values().cloned().collect()
        }; // Lock is released here

        for extension_arc in extensions_clone {
            let ext_guard = extension_arc.lock().await;
            all_options.extend(ext_guard.options());
        }
        all_options
    }

    pub async fn call(
        &self,
        path: &str,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<Vec<u8>, EngineError> {
        self.call_response(path, params, body)
            .await
            .map(|response| response.body)
    }

    pub async fn call_response(
        &self,
        path: &str,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<ProbeExtensionResponse, EngineError> {
        let extensions_clone: Vec<_> = {
            let extensions = self.extensions.read().await;
            extensions.values().cloned().collect()
        }; // Lock is released here

        for extension in extensions_clone {
            let ext = extension.lock().await;
            let name = ext.name();
            let expected_prefix = format!("/{name}/");

            // Also check without leading slash for flexibility
            let expected_prefix_no_slash = format!("{name}/");

            let (matched, local_path) = if path.starts_with(&expected_prefix) {
                (true, path[expected_prefix.len()..].to_string())
            } else if path.starts_with(&expected_prefix_no_slash) {
                (true, path[expected_prefix_no_slash.len()..].to_string())
            } else {
                (false, String::new())
            };

            if !matched {
                continue;
            }

            log::debug!("checking extension [{name}]:{path}");
            log::debug!("Extension [{name}] matched, local_path: {}", local_path);

            // Call the extension while preserving response completeness metadata.
            match ext.call_response(&local_path, params, body).await {
                Ok(value) => return Ok(value),
                Err(EngineError::UnsupportedCall) => {
                    log::debug!(
                        "Extension [{name}] returned UnsupportedCall for path: {}",
                        local_path
                    );
                    continue;
                }
                Err(e) => {
                    log::error!(
                        "Extension [{name}] call failed for path '{}': {}",
                        local_path,
                        e
                    );
                    return Err(e);
                }
            }
        }
        log::error!("No extension matched path: {}", path);
        Err(EngineError::CallError(format!("API call error: {}", path)))
    }
}

impl ConfigExtension for ProbeExtensionManager {
    const PREFIX: &'static str = "probing";
}

impl ExtensionOptions for ProbeExtensionManager {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn cloned(&self) -> Box<dyn ExtensionOptions> {
        Box::new(self.clone())
    }

    fn set(&mut self, key: &str, value: &str) -> datafusion::error::Result<()> {
        let mut manager = self.clone();
        let key = key.to_string();
        let value = value.to_string();
        crate::runtime::block_on(async move { manager.set_option(&key, &value).await })
            .map_err(datafusion::error::DataFusionError::from)?
            .map_err(datafusion::error::DataFusionError::from)
    }

    fn entries(&self) -> Vec<datafusion::config::ConfigEntry> {
        let manager = self.clone();
        match crate::runtime::block_on(async move {
            manager
                .options()
                .await
                .iter()
                .map(|option| datafusion::config::ConfigEntry {
                    key: format!("{}.{}", Self::PREFIX, option.key),
                    value: option.value.clone(),
                    description: option.help,
                })
                .collect()
        }) {
            Ok(entries) => entries,
            Err(error) => {
                log::error!("failed to enumerate probing extension options: {error}");
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    // Helper to ensure clean state before each test
    async fn setup_test() -> tokio::sync::MutexGuard<'static, ()> {
        let guard = config::TEST_STATE_LOCK.lock().await;
        config::clear().await;
        guard
    }

    // Helper to ensure clean state after each test
    async fn teardown_test() {
        config::clear().await;
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

    impl ProbeExtensionCall for TestExtension {}

    impl ProbeExtension for TestExtension {
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

        fn options(&self) -> Vec<ProbeExtensionOption> {
            vec![ProbeExtensionOption {
                key: "option".to_string(),
                value: Some(self.test_option.clone()),
                help: "Test option",
            }]
        }
    }

    #[derive(Debug)]
    struct PartialResponseExtension;

    #[async_trait]
    impl ProbeExtensionCall for PartialResponseExtension {
        async fn call_response(
            &self,
            _path: &str,
            _params: &HashMap<String, String>,
            _body: &[u8],
        ) -> Result<ProbeExtensionResponse, EngineError> {
            Ok(ProbeExtensionResponse {
                body: b"partial".to_vec(),
                partial: true,
            })
        }
    }

    impl ProbeExtension for PartialResponseExtension {
        fn name(&self) -> String {
            "partial".to_string()
        }
    }

    #[tokio::test]
    async fn call_response_preserves_extension_metadata() {
        let mut manager = ProbeExtensionManager::default();
        manager
            .register(
                "partial".to_string(),
                Arc::new(Mutex::new(PartialResponseExtension)),
            )
            .await;

        let response = manager
            .call_response("/partial/data", &HashMap::new(), &[])
            .await
            .unwrap();

        assert_eq!(response.body, b"partial");
        assert!(response.partial);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_option_syncs_to_config_store() {
        let _state_guard = setup_test().await;

        let mut manager = ProbeExtensionManager::default();
        let extension = Arc::new(Mutex::new(TestExtension::default()));
        manager
            .register("test".to_string(), extension.clone())
            .await;

        // Set option through manager using set_option_with_store_update
        manager
            .set_option_with_store_update("test.option", "new_value")
            .await
            .unwrap();

        // Verify it's in ConfigStore
        let value = config::get_str("test.option").await;
        assert_eq!(value, Some("new_value".to_string()));

        // Verify extension was updated
        let ext_guard = extension.lock().await;
        let value = ext_guard.get("option").unwrap();
        assert_eq!(value, "new_value");
        drop(ext_guard);

        teardown_test().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_option_updates_existing_value() {
        let _state_guard = setup_test().await;

        // Pre-populate ConfigStore
        config::set("test.option", "old_value").await;

        let mut manager = ProbeExtensionManager::default();
        let extension = Arc::new(Mutex::new(TestExtension::default()));
        manager.register("test".to_string(), extension).await;

        // Set option through manager using set_option_with_store_update
        manager
            .set_option_with_store_update("test.option", "new_value")
            .await
            .unwrap();

        // Verify ConfigStore was updated
        let value = config::get_str("test.option").await;
        assert_eq!(value, Some("new_value".to_string()));

        teardown_test().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_option_unsupported_key() {
        let _state_guard = setup_test().await;

        let mut manager = ProbeExtensionManager::default();
        let extension = Arc::new(Mutex::new(TestExtension::default()));
        manager.register("test".to_string(), extension).await;

        // Try to set unsupported key
        let result = manager.set_option("test.invalid", "value").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EngineError::UnsupportedOption(_)
        ));

        // Verify ConfigStore was not updated
        assert!(!config::contains_key("test.invalid").await);

        teardown_test().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_option_from_config_store() {
        let _state_guard = setup_test().await;

        // Pre-populate ConfigStore
        config::set("test.option", "stored_value").await;

        // Verify ConfigStore has the value
        let value = config::get_str("test.option").await;
        assert_eq!(value, Some("stored_value".to_string()));

        teardown_test().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn independently_built_managers_do_not_share_extensions() {
        let mut first = ProbeExtensionManager::default();
        let second = ProbeExtensionManager::default();
        first
            .register(
                "test".to_string(),
                Arc::new(Mutex::new(TestExtension::default())),
            )
            .await;

        first.set_option("test.option", "first").await.unwrap();
        assert_eq!(first.get_option("test.option").await.unwrap(), "first");
        assert!(matches!(
            second.get_option("test.option").await,
            Err(EngineError::UnsupportedOption(_))
        ));
    }
}
