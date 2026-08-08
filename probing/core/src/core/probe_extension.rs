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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtensionHttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

impl ExtensionHttpMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtensionContentType {
    Json,
    Text,
    Html,
}

impl ExtensionContentType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Text => "text/plain",
            Self::Html => "text/html",
        }
    }
}

/// Static HTTP contract published by an extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtensionRoute {
    /// Extension-local path without a leading slash.
    pub path: &'static str,
    pub method: ExtensionHttpMethod,
    pub content_type: ExtensionContentType,
    pub cors: bool,
    pub requires_engine_ready: bool,
}

impl ExtensionRoute {
    pub const fn new(
        path: &'static str,
        method: ExtensionHttpMethod,
        content_type: ExtensionContentType,
    ) -> Self {
        Self {
            path,
            method,
            content_type,
            cors: false,
            requires_engine_ready: false,
        }
    }

    pub const fn with_cors(mut self) -> Self {
        self.cors = true;
        self
    }

    pub const fn requiring_engine(mut self) -> Self {
        self.requires_engine_ready = true;
        self
    }
}

/// Static configuration contract published by an extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtensionConfigSpec {
    /// Extension-local canonical key.
    pub key: &'static str,
    pub aliases: &'static [&'static str],
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
    /// HTTP routes owned by this extension. Registration validates and indexes
    /// these contracts before the engine becomes visible.
    fn routes(&self) -> Vec<ExtensionRoute> {
        Vec::new()
    }

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

/// Runtime configuration contract, intentionally independent from HTTP calls.
pub trait ProbeExtensionConfig: Debug + Send + Sync {
    fn set(&mut self, key: &str, _value: &str) -> Result<String, EngineError> {
        Err(EngineError::UnsupportedOption(key.to_string()))
    }

    fn get(&self, key: &str) -> Result<String, EngineError> {
        Err(EngineError::UnsupportedOption(key.to_string()))
    }

    fn options(&self) -> Vec<ProbeExtensionOption> {
        Vec::new()
    }

    fn config_specs(&self) -> &'static [ExtensionConfigSpec] {
        &[]
    }
}

/// Configurable Probing extension: HTTP calls, SET options, and runtime side effects.
///
/// SQL catalog registration is separate — use [`ProbeDataSource`] via
/// [`super::engine::EngineBuilder::with_data_source`].
#[allow(unused)]
pub trait ProbeExtension: Debug + Send + Sync + ProbeExtensionCall + ProbeExtensionConfig {
    fn name(&self) -> String;
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
/// use probing_core::core::{EngineError, ExtensionConfigSpec, ProbeExtension, ProbeExtensionConfig, ProbeExtensionOption, ProbeExtensionCall};
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
/// }
///
/// impl ProbeExtensionConfig for MyExtension {
///     fn config_specs(&self) -> &'static [ExtensionConfigSpec] {
///         &[ExtensionConfigSpec { key: "some_option", aliases: &[], help: "An example option" }]
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
///     // Registration keys must match the extension's declared name.
///     manager.register(
///         "my_extension".to_string(),
///         Arc::new(Mutex::new(MyExtension { some_option: "default".to_string() }))
///     ).await?;
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
    routes: Arc<RwLock<BTreeMap<String, RegisteredExtensionRoute>>>,
    configs: Arc<RwLock<BTreeMap<String, RegisteredExtensionConfig>>>,
}

#[derive(Clone, Debug)]
struct RegisteredExtensionRoute {
    extension: Arc<Mutex<dyn ProbeExtension + Send + Sync>>,
    contract: ExtensionRoute,
}

#[derive(Clone, Debug)]
struct RegisteredExtensionConfig {
    extension: Arc<Mutex<dyn ProbeExtension + Send + Sync>>,
    local_key: &'static str,
}

impl Default for ProbeExtensionManager {
    fn default() -> Self {
        Self {
            extensions: Arc::new(RwLock::new(BTreeMap::new())),
            routes: Arc::new(RwLock::new(BTreeMap::new())),
            configs: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }
}

impl ProbeExtensionManager {
    /// Register an extension in this manager's engine-scoped registry.
    pub async fn register(
        &mut self,
        name: String,
        extension: Arc<Mutex<dyn ProbeExtension + Send + Sync>>,
    ) -> Result<(), EngineError> {
        let (actual_name, contracts, config_specs) = {
            let extension = extension.lock().await;
            (
                extension.name(),
                extension.routes(),
                extension.config_specs().to_vec(),
            )
        };
        validate_extension_name(&actual_name)?;
        if name != actual_name {
            return Err(EngineError::config(format!(
                "extension registration key '{name}' does not match declared name '{actual_name}'"
            )));
        }
        validate_config_specs(&actual_name, &config_specs)?;

        let config_namespace = Self::extract_namespace(&actual_name);
        let mut indexed_configs = Vec::new();
        for spec in config_specs {
            indexed_configs.push((format!("{config_namespace}{}", spec.key), spec.key));
            indexed_configs.extend(
                spec.aliases
                    .iter()
                    .map(|alias| (format!("{config_namespace}{alias}"), *alias)),
            );
        }

        let mut indexed = Vec::with_capacity(contracts.len());
        for contract in contracts {
            validate_route(&actual_name, contract)?;
            let full_path = extension_route_key(&actual_name, contract.path);
            if indexed.iter().any(|(path, _)| path == &full_path) {
                return Err(EngineError::config(format!(
                    "extension '{actual_name}' declares duplicate route '/{full_path}'"
                )));
            }
            indexed.push((full_path, contract));
        }

        let mut extensions = self.extensions.write().await;
        if extensions.contains_key(&actual_name) {
            return Err(EngineError::config(format!(
                "duplicate extension name '{actual_name}'"
            )));
        }
        let mut routes = self.routes.write().await;
        if let Some((path, _)) = indexed.iter().find(|(path, _)| routes.contains_key(path)) {
            return Err(EngineError::config(format!(
                "duplicate extension route '/{path}'"
            )));
        }
        let mut configs = self.configs.write().await;
        if let Some((key, _)) = indexed_configs
            .iter()
            .find(|(key, _)| configs.contains_key(key))
        {
            return Err(EngineError::config(format!(
                "duplicate registered extension config key '{key}'"
            )));
        }
        extensions.insert(actual_name, extension.clone());
        for (path, contract) in indexed {
            routes.insert(
                path,
                RegisteredExtensionRoute {
                    extension: extension.clone(),
                    contract,
                },
            );
        }
        for (key, local_key) in indexed_configs {
            configs.insert(
                key,
                RegisteredExtensionConfig {
                    extension: extension.clone(),
                    local_key,
                },
            );
        }
        Ok(())
    }

    pub async fn route(&self, path: &str) -> Option<ExtensionRoute> {
        self.routes
            .read()
            .await
            .get(&normalize_route_key(path))
            .map(|route| route.contract)
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
        let registered = self
            .configs
            .read()
            .await
            .get(key)
            .cloned()
            .ok_or_else(|| EngineError::UnsupportedOption(key.to_string()))?;
        let mut extension = registered.extension.lock().await;
        let old = extension.set(registered.local_key, value)?;
        log::info!("setting update [{key}]={value} <= {old}");
        Ok(())
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
        let registered = self
            .configs
            .read()
            .await
            .get(key)
            .cloned()
            .ok_or_else(|| EngineError::UnsupportedOption(key.to_string()))?;
        let extension = registered.extension.lock().await;
        let value = extension.get(registered.local_key)?;
        log::info!("setting read [{key}]={value}");
        Ok(value)
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
        let registered = self
            .routes
            .read()
            .await
            .get(&normalize_route_key(path))
            .cloned()
            .ok_or_else(|| EngineError::CallError(format!("API call error: {path}")))?;
        let extension = registered.extension.lock().await;
        extension
            .call_response(registered.contract.path, params, body)
            .await
    }
}

fn normalize_route_key(path: &str) -> String {
    path.trim().trim_matches('/').to_ascii_lowercase()
}

fn extension_route_key(extension_name: &str, local_path: &str) -> String {
    let local_path = local_path.trim_matches('/');
    if local_path.is_empty() {
        normalize_route_key(extension_name)
    } else {
        normalize_route_key(&format!("{extension_name}/{local_path}"))
    }
}

fn validate_extension_name(name: &str) -> Result<(), EngineError> {
    if name.is_empty()
        || name != name.to_ascii_lowercase()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(EngineError::config(format!(
            "invalid extension name '{name}': use lowercase ASCII letters, digits, or '_'"
        )));
    }
    Ok(())
}

fn validate_route(extension_name: &str, route: ExtensionRoute) -> Result<(), EngineError> {
    if route.path.starts_with('/')
        || route.path.ends_with('/')
        || route.path.contains("..")
        || route.path.contains(['?', '#'])
    {
        return Err(EngineError::config(format!(
            "invalid route path '{}' declared by extension '{extension_name}'",
            route.path
        )));
    }
    Ok(())
}

fn validate_config_specs(
    extension_name: &str,
    specs: &[ExtensionConfigSpec],
) -> Result<(), EngineError> {
    let mut keys = std::collections::BTreeSet::new();
    for spec in specs {
        for key in std::iter::once(spec.key).chain(spec.aliases.iter().copied()) {
            if key.is_empty() || key.starts_with('.') || key.ends_with('.') || key.contains("..") {
                return Err(EngineError::config(format!(
                    "invalid config key '{key}' declared by extension '{extension_name}'"
                )));
            }
            if !keys.insert(key) {
                return Err(EngineError::config(format!(
                    "duplicate config key or alias '{key}' in extension '{extension_name}'"
                )));
            }
        }
    }
    Ok(())
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
    }

    impl ProbeExtensionConfig for TestExtension {
        fn config_specs(&self) -> &'static [ExtensionConfigSpec] {
            &[ExtensionConfigSpec {
                key: "option",
                aliases: &[],
                help: "Test option",
            }]
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
        fn routes(&self) -> Vec<ExtensionRoute> {
            vec![ExtensionRoute::new(
                "data",
                ExtensionHttpMethod::Get,
                ExtensionContentType::Text,
            )]
        }

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

    impl ProbeExtensionConfig for PartialResponseExtension {}

    #[derive(Debug)]
    struct InvalidContractExtension {
        duplicate_config: bool,
    }

    impl ProbeExtensionCall for InvalidContractExtension {
        fn routes(&self) -> Vec<ExtensionRoute> {
            vec![
                ExtensionRoute::new("same", ExtensionHttpMethod::Get, ExtensionContentType::Json),
                ExtensionRoute::new(
                    "same",
                    ExtensionHttpMethod::Post,
                    ExtensionContentType::Text,
                ),
            ]
        }
    }

    impl ProbeExtensionConfig for InvalidContractExtension {
        fn config_specs(&self) -> &'static [ExtensionConfigSpec] {
            if self.duplicate_config {
                &[
                    ExtensionConfigSpec {
                        key: "sample",
                        aliases: &["rate"],
                        help: "sample",
                    },
                    ExtensionConfigSpec {
                        key: "other",
                        aliases: &["rate"],
                        help: "other",
                    },
                ]
            } else {
                &[]
            }
        }
    }

    impl ProbeExtension for InvalidContractExtension {
        fn name(&self) -> String {
            "invalid".into()
        }
    }

    #[tokio::test]
    async fn registration_rejects_duplicate_routes() {
        let mut manager = ProbeExtensionManager::default();
        let error = manager
            .register(
                "invalid".into(),
                Arc::new(Mutex::new(InvalidContractExtension {
                    duplicate_config: false,
                })),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("duplicate route"));
    }

    #[tokio::test]
    async fn registration_rejects_duplicate_config_aliases() {
        let mut manager = ProbeExtensionManager::default();
        let error = manager
            .register(
                "invalid".into(),
                Arc::new(Mutex::new(InvalidContractExtension {
                    duplicate_config: true,
                })),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("duplicate config key or alias"));
    }

    #[tokio::test]
    async fn call_response_preserves_extension_metadata() {
        let mut manager = ProbeExtensionManager::default();
        manager
            .register(
                "partial".to_string(),
                Arc::new(Mutex::new(PartialResponseExtension)),
            )
            .await
            .unwrap();

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
            .await
            .unwrap();

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
        manager
            .register("test".to_string(), extension)
            .await
            .unwrap();

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
        manager
            .register("test".to_string(), extension)
            .await
            .unwrap();

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
            .await
            .unwrap();

        first.set_option("test.option", "first").await.unwrap();
        assert_eq!(first.get_option("test.option").await.unwrap(), "first");
        assert!(matches!(
            second.get_option("test.option").await,
            Err(EngineError::UnsupportedOption(_))
        ));
    }
}
