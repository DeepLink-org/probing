use std::collections::BTreeMap;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt::Debug;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use datafusion::config::{ConfigExtension, ExtensionOptions};
use once_cell::sync::Lazy;
use std::sync::{Mutex, RwLock};

use super::error::EngineError;
use super::Plugin;
use crate::config;

/// Global extensions registry.
///
/// This provides a global storage for all engine extensions, allowing
/// EngineExtensionManager to operate on a shared set of extensions.
/// Uses synchronous `RwLock` and `Mutex` to allow synchronous access from other threads.
pub static EXTENSIONS: Lazy<RwLock<BTreeMap<String, Arc<Mutex<dyn EngineExtension + Send + Sync>>>>> =
    Lazy::new(|| RwLock::new(BTreeMap::new()));

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
pub struct EngineExtensionOption {
    pub key: String,
    pub value: Option<String>,
    pub help: &'static str,
}

/// Extension trait for handling API calls
#[allow(unused)]
#[async_trait]
pub trait EngineCall: Debug + Send + Sync {
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
}

/// Extension trait for providing data sources
#[allow(unused)]
pub trait EngineDatasource: Debug + Send + Sync {
    /// Provide a data source plugin implementation
    ///
    /// # Arguments
    /// * `namespace` - The namespace for the data source
    /// * `name` - Optional name of the specific data source
    ///
    /// # Returns
    /// * `Some(Arc<dyn Plugin>)` - Data source plugin if available
    /// * `None` - If no matching data source is available
    fn datasrc(
        &self,
        namespace: &str,
        name: Option<&str>,
    ) -> Option<Arc<dyn Plugin + Sync + Send>> {
        None
    }
}

/// A trait for engine extensions that can be configured with key-value pairs.
///
/// This trait defines the interface for extensions that can be registered with
/// the [`EngineExtensionManager`] to provide configurable functionality.
///
/// # Required Methods
///
/// * [`name`] - Returns the unique name of this extension
/// * [`set`] - Sets a configuration option value
/// * [`get`] - Retrieves a configuration option value  
/// * [`options`] - Lists all available configuration options
///
/// # Examples
///
/// ```
/// use probing_core::core::{EngineExtension, EngineExtensionOption};
/// use probing_core::core::EngineCall;
/// use probing_core::core::EngineDatasource;
/// use probing_core::core::EngineError;
///
/// #[derive(Debug)]
/// struct MyExtension {
///     some_option: String
/// }
///
/// impl EngineCall for MyExtension {}
///
/// impl EngineDatasource for MyExtension {}
///
/// impl EngineExtension for MyExtension {
///     fn name(&self) -> String {
///         "my_extension".to_string()
///     }
///
///     fn set(&mut self, key: &str, value: &str) -> Result<String, EngineError> {
///         match key {
///             "some_option" => {
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
///             "some_option" => Ok(self.some_option.clone()),
///             _ => Err(EngineError::UnsupportedOption(key.to_string()))
///         }
///     }
///
///     fn options(&self) -> Vec<EngineExtensionOption> {
///         vec![
///             EngineExtensionOption {
///                 key: "some_option".to_string(),
///                 value: Some(self.some_option.clone()),
///                 help: "An example option"
///             }
///         ]
///     }
/// }
/// let mut ext = MyExtension { some_option: "default".to_string() };
/// assert_eq!(ext.name(), "my_extension");
/// assert_eq!(ext.set("some_option", "new").unwrap(), "default");
/// assert_eq!(ext.get("some_option").unwrap(), "new");
/// ```
#[allow(unused)]
pub trait EngineExtension: Debug + Send + Sync + EngineCall + EngineDatasource {
    fn name(&self) -> String;
    fn set(&mut self, key: &str, value: &str) -> Result<String, EngineError> {
        todo!()
    }
    fn get(&self, key: &str) -> Result<String, EngineError> {
        todo!()
    }
    fn options(&self) -> Vec<EngineExtensionOption> {
        todo!()
    }
}

/// Engine extension management module for configurable functionality.
///
/// This module provides a flexible extension system that allows for runtime configuration
/// of engine components through a key-value interface. It consists of three main components:
///
/// - [`EngineExtensionOption`]: Represents a single configuration option with metadata
/// - [`EngineExtension`]: A trait that must be implemented by configurable extensions
/// - [`EngineExtensionManager`]: Manages multiple extensions and their configurations
///
/// The extension system integrates with DataFusion's configuration framework through
/// implementations of [`ConfigExtension`] and [`ExtensionOptions`].
///
/// # Examples
///
/// ```rust
/// use std::sync::Arc;
/// use tokio::sync::Mutex;
/// use probing_core::core::EngineExtensionManager;
/// use probing_core::core::{EngineExtension, EngineExtensionOption, EngineCall, EngineDatasource, EngineError};
///
/// #[derive(Debug)]
/// struct MyExtension {
///     some_option: String
/// }
///
/// // MyExtension needs to implement EngineCall and EngineDatasource.
/// // These traits have default implementations for their methods if not overridden.
/// impl EngineCall for MyExtension {}
/// impl EngineDatasource for MyExtension {}
///
/// impl EngineExtension for MyExtension {
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
///     fn options(&self) -> Vec<EngineExtensionOption> {
///         vec![
///             EngineExtensionOption {
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
///     let mut manager = EngineExtensionManager::default();
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
/// Engine extension manager that operates on the global extensions registry.
///
/// This struct no longer holds extensions directly. Instead, it operates
/// on the global `EXTENSIONS` registry, allowing multiple instances to
/// work with the same set of extensions.
#[derive(Clone, Debug, Default)]
pub struct EngineExtensionManager;

impl EngineExtensionManager {
    /// Register an extension in the global extensions registry.
    pub fn register(
        &mut self,
        name: String,
        extension: Arc<Mutex<dyn EngineExtension + Send + Sync>>,
    ) {
        EXTENSIONS.write().unwrap().insert(name, extension);
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
    pub fn set_option(&mut self, key: &str, value: &str) -> Result<(), EngineError> {
        let extensions_clone: Vec<_> = {
            let extensions = EXTENSIONS.read().unwrap();
            extensions.values().cloned().collect()
        }; // Lock is released here
        
        for extension in extensions_clone {
            // Minimize lock scope: only lock when needed
            let (namespace, local_key) = {
                let ext = extension.lock().unwrap();
                let namespace = Self::extract_namespace(&ext.name());
                if !key.starts_with(&namespace) {
                    continue;
                }
                let local_key = key.trim_start_matches(&namespace).to_string();
                (namespace, local_key)
            };
            
            // Lock again only for the set operation, minimize lock scope
            let result = {
                let mut ext = extension.lock().unwrap();
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
    pub fn set_option_with_store_update(
        &mut self,
        key: &str,
        value: &str,
    ) -> Result<(), EngineError> {
        self.set_option(key, value)?;
        // Update ConfigStore after successfully updating the extension
        config::set(key, value);
        Ok(())
    }

    pub async fn get_option(&self, key: &str) -> Result<String, EngineError> {
        let extensions_clone: Vec<_> = {
            let extensions = EXTENSIONS.read().unwrap();
            extensions.values().cloned().collect()
        }; // Lock is released here
        
        for extension in extensions_clone {
            let ext = tokio::task::block_in_place(|| extension.lock().unwrap());
            let namespace = Self::extract_namespace(&ext.name());
            if !key.starts_with(&namespace) {
                continue;
            }
            let local_key = key.trim_start_matches(&namespace);
            if let Ok(value) = ext.get(local_key) {
                log::info!("setting read [{}]:{local_key}={value}", ext.name());
                return Ok(value);
            }
        }
        Err(EngineError::UnsupportedOption(key.to_string()))
    }

    pub async fn options(&self) -> Vec<EngineExtensionOption> {
        let mut all_options = Vec::new();
        let extensions_clone: Vec<_> = {
            let extensions = EXTENSIONS.read().unwrap();
            extensions.values().cloned().collect()
        }; // Lock is released here
        
        for extension_arc in extensions_clone {
            let ext_guard = tokio::task::block_in_place(|| extension_arc.lock().unwrap());
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
        let extensions_clone: Vec<_> = {
            let extensions = EXTENSIONS.read().unwrap();
            extensions.values().cloned().collect()
        }; // Lock is released here
        
        for extension in extensions_clone {
            // Get the extension name and check if path matches
            let (name, should_call) = tokio::task::block_in_place(|| {
                let ext = extension.lock().unwrap();
                let name = ext.name();
                let expected_prefix = format!("/{name}/");
                let should_call = path.starts_with(&expected_prefix);
                (name, should_call)
            });
            
            if !should_call {
                continue;
            }
            
            log::debug!("checking extension [{name}]:{path}");
            let local_path = path[format!("/{name}/").len()..].to_string();
            
            // Call the extension's async call method
            // We need to lock again, but we'll do it in a blocking task
            let extension_clone = extension.clone();
            let local_path_clone = local_path.clone();
            let params_clone = params.clone();
            let body_clone = body.to_vec();
            
            // Use spawn_blocking to call the async method with sync lock
            let result = tokio::task::spawn_blocking(move || {
                let ext = extension_clone.lock().unwrap();
                // We can't directly call async methods from sync context
                // So we'll use futures::executor::block_on to run the async call
                use futures::executor::block_on;
                block_on(ext.call(&local_path_clone, &params_clone, &body_clone))
            }).await;
            
            match result {
                Ok(Ok(value)) => return Ok(value),
                Ok(Err(EngineError::UnsupportedCall)) => continue,
                Ok(Err(e)) => return Err(e),
                Err(_) => continue,
            }
        }
        Err(EngineError::CallError(path.to_string()))
    }
}

impl ConfigExtension for EngineExtensionManager {
    const PREFIX: &'static str = "probing";
}

impl ExtensionOptions for EngineExtensionManager {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn cloned(&self) -> Box<dyn ExtensionOptions> {
        // EngineExtensionManager is now a zero-sized type, so cloning is trivial
        Box::new(EngineExtensionManager)
    }

    fn set(&mut self, key: &str, value: &str) -> datafusion::error::Result<()> {
        self.set_option(key, value)
            .map_err(|e| datafusion::error::DataFusionError::Execution(e.to_string()))
    }

    fn entries(&self) -> Vec<datafusion::config::ConfigEntry> {
        use futures::executor::block_on;

        block_on(async {
            self.options()
                .await
                .iter()
                .map(|option| datafusion::config::ConfigEntry {
                    key: format!("{}.{}", Self::PREFIX, option.key),
                    value: option.value.clone(),
                    description: option.help,
                })
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    // Helper to ensure clean state before each test
    fn setup_test() {
        config::clear();
        EXTENSIONS.write().unwrap().clear();
    }

    // Helper to ensure clean state after each test
    fn teardown_test() {
        config::clear();
        EXTENSIONS.write().unwrap().clear();
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_option_syncs_to_config_store() {
        setup_test();

        let mut manager = EngineExtensionManager::default();
        let extension = Arc::new(Mutex::new(TestExtension::default()));
        manager.register("test".to_string(), extension);

        // Set option through manager using set_option_with_store_update
        // Use spawn_blocking to avoid blocking the async runtime
        tokio::task::spawn_blocking(move || {
            manager.set_option_with_store_update("test.option", "new_value").unwrap()
        }).await.unwrap();

        // Verify it's in ConfigStore
        let value = config::get_str("test.option");
        assert_eq!(value, Some("new_value".to_string()));

        // Verify extension was updated
        let ext_guard = tokio::task::spawn_blocking(|| {
            let extensions = EXTENSIONS.read().unwrap();
            let ext_guard = extensions.get("test").unwrap().lock().unwrap();
            ext_guard.get("option").unwrap()
        }).await.unwrap();
        assert_eq!(ext_guard, "new_value");

        teardown_test();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_option_updates_existing_value() {
        setup_test();

        // Pre-populate ConfigStore
        config::set("test.option", "old_value");

        let mut manager = EngineExtensionManager::default();
        let extension = Arc::new(Mutex::new(TestExtension::default()));
        manager.register("test".to_string(), extension);

        // Set option through manager using set_option_with_store_update
        // Use spawn_blocking to avoid blocking the async runtime
        tokio::task::spawn_blocking(move || {
            manager.set_option_with_store_update("test.option", "new_value").unwrap()
        }).await.unwrap();

        // Verify ConfigStore was updated
        let value = config::get_str("test.option");
        assert_eq!(value, Some("new_value".to_string()));

        teardown_test();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_option_unsupported_key() {
        setup_test();

        let mut manager = EngineExtensionManager::default();
        let extension = Arc::new(Mutex::new(TestExtension::default()));
        manager.register("test".to_string(), extension);

        // Try to set unsupported key
        // Use spawn_blocking to avoid blocking the async runtime
        let result = tokio::task::spawn_blocking(move || {
            manager.set_option("test.invalid", "value")
        }).await.unwrap();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            EngineError::UnsupportedOption(_)
        ));

        // Verify ConfigStore was not updated
        assert!(!config::contains_key("test.invalid"));

        teardown_test();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_option_from_config_store() {
        setup_test();

        // Pre-populate ConfigStore
        config::set("test.option", "stored_value");

        // Verify ConfigStore has the value
        let value = config::get_str("test.option");
        assert_eq!(value, Some("stored_value".to_string()));

        teardown_test();
    }
}
