use std::collections::BTreeMap;
use std::sync::RwLock;

use once_cell::sync::Lazy;
use probing_proto::prelude::Ele;

/// Global configuration key-value store.
///
/// This provides a simple, high-performance KV database for configuration
/// storage using BTreeMap for ordered iteration and proto's Ele type for
/// type-safe value storage.
///
/// Uses `RwLock` instead of `Mutex` to allow concurrent read operations,
/// which is ideal for read-heavy workloads typical of configuration access.
///
/// All configuration options from extensions can be stored here, and Python
/// code can directly access this store without cross-language call overhead.
///
/// # Examples
///
/// ```rust
/// use probing_core::config::store::ConfigStore;
/// use probing_proto::prelude::Ele;
///
/// // Set configuration values (generic method supports all types)
/// ConfigStore::set("torch.profiling", "on,mode=random");
/// ConfigStore::set("server.debug", true);        // bool
/// ConfigStore::set("server.port", 8080i32);      // i32
/// ConfigStore::set("server.timeout", 30.5f64);   // f64
///
/// // Get a configuration value
/// let value = ConfigStore::get("torch.profiling");
/// assert_eq!(value, Some(Ele::Text("on,mode=random".to_string())));
/// ```
pub static CONFIG_STORE: Lazy<RwLock<BTreeMap<String, Ele>>> =
    Lazy::new(|| RwLock::new(BTreeMap::new()));

/// Simple configuration KV store interface.
///
/// This provides direct, high-performance access to configuration values
/// without the overhead of traversing extensions or async operations.
///
/// Values are stored using proto's Ele type, which supports:
/// - Text (String)
/// - BOOL (bool)
/// - I32, I64 (integers)
/// - F32, F64 (floats)
/// - DataTime (timestamp)
/// - Nil (null value)
pub struct ConfigStore;

impl ConfigStore {
    /// Get a configuration value.
    ///
    /// This is a synchronous, zero-copy operation for maximum performance.
    ///
    /// # Arguments
    /// * `key` - The configuration key
    ///
    /// # Returns
    /// * `Some(Ele)` - The value if found
    /// * `None` - The key doesn't exist
    ///
    /// # Examples
    /// ```rust
    /// use probing_core::config::store::ConfigStore;
    ///
    /// // Get configuration
    /// if let Some(value) = ConfigStore::get("torch.profiling") {
    ///     match value {
    ///         Ele::Text(s) => println!("Config: {}", s),
    ///         _ => println!("Non-text config value"),
    ///     }
    /// }
    /// ```
    pub fn get(key: &str) -> Option<Ele> {
        CONFIG_STORE.read().unwrap().get(key).cloned()
    }

    /// Set a configuration value.
    ///
    /// This is a generic method that accepts any type that can be converted to `Ele`.
    /// Supported types include: `&str`, `String`, `bool`, `i32`, `i64`, `f32`, `f64`, and `Ele`.
    ///
    /// # Arguments
    /// * `key` - The configuration key
    /// * `value` - The configuration value (any type that implements `Into<Ele>`)
    ///
    /// # Examples
    /// ```rust
    /// use probing_core::config::store::ConfigStore;
    /// use probing_proto::prelude::Ele;
    ///
    /// // Set string values
    /// ConfigStore::set("torch.profiling", "on,mode=random");
    /// ConfigStore::set("server.name", "localhost".to_string());
    ///
    /// // Set different types
    /// ConfigStore::set("server.debug", true);        // bool
    /// ConfigStore::set("server.port", 8080i32);      // i32
    /// ConfigStore::set("server.timeout", 30.5f64);   // f64
    /// ConfigStore::set("server.ratio", 0.75f32);     // f32
    ///
    /// // Set using Ele directly
    /// ConfigStore::set("custom.key", Ele::I64(42));
    /// ```
    pub fn set<T: Into<Ele>>(key: &str, value: T) {
        CONFIG_STORE
            .write()
            .unwrap()
            .insert(key.to_string(), value.into());
    }

    /// Get a configuration value as string.
    ///
    /// This is a convenience method that extracts the string value from Ele.
    /// If the value is not Text type, it will be converted to string.
    ///
    /// # Arguments
    /// * `key` - The configuration key
    ///
    /// # Returns
    /// * `Some(String)` - The value as string if found
    /// * `None` - The key doesn't exist
    ///
    /// # Examples
    /// ```rust
    /// use probing_core::config::store::ConfigStore;
    ///
    /// if let Some(value) = ConfigStore::get_str("torch.profiling") {
    ///     println!("Config: {}", value);
    /// }
    /// ```
    pub fn get_str(key: &str) -> Option<String> {
        ConfigStore::get(key).map(|ele| match ele {
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
    ///
    /// # Arguments
    /// * `key` - The configuration key to remove
    ///
    /// # Returns
    /// * `Some(Ele)` - The removed value if it existed
    /// * `None` - The key didn't exist
    ///
    /// # Examples
    /// ```rust
    /// use probing_core::config::store::ConfigStore;
    ///
    /// ConfigStore::remove("torch.profiling");
    /// ```
    pub fn remove(key: &str) -> Option<Ele> {
        CONFIG_STORE.write().unwrap().remove(key)
    }

    /// Check if a key exists.
    ///
    /// # Arguments
    /// * `key` - The configuration key
    ///
    /// # Returns
    /// * `true` if the key exists, `false` otherwise
    ///
    /// # Examples
    /// ```rust
    /// use probing_core::config::store::ConfigStore;
    ///
    /// if ConfigStore::contains_key("torch.profiling") {
    ///     println!("Configuration exists");
    /// }
    /// ```
    pub fn contains_key(key: &str) -> bool {
        CONFIG_STORE.read().unwrap().contains_key(key)
    }

    /// Get all configuration keys.
    ///
    /// Keys are returned in sorted order (BTreeMap guarantees ordering).
    ///
    /// # Returns
    /// * `Vec<String>` - All configuration keys in sorted order
    ///
    /// # Examples
    /// ```rust
    /// use probing_core::config::store::ConfigStore;
    ///
    /// let keys = ConfigStore::keys();
    /// for key in keys {
    ///     println!("Config key: {}", key);
    /// }
    /// ```
    pub fn keys() -> Vec<String> {
        CONFIG_STORE.read().unwrap().keys().cloned().collect()
    }

    /// Get all configuration entries.
    ///
    /// Entries are returned in sorted order by key (BTreeMap guarantees ordering).
    ///
    /// # Returns
    /// * `BTreeMap<String, Ele>` - All key-value pairs in sorted order
    ///
    /// # Examples
    /// ```rust
    /// use probing_core::config::store::ConfigStore;
    ///
    /// let all = ConfigStore::all();
    /// for (key, value) in all {
    ///     println!("{} = {:?}", key, value);
    /// }
    /// ```
    pub fn all() -> BTreeMap<String, Ele> {
        CONFIG_STORE.read().unwrap().clone()
    }

    /// Clear all configuration.
    ///
    /// # Examples
    /// ```rust
    /// use probing_core::config::store::ConfigStore;
    ///
    /// ConfigStore::clear();
    /// ```
    pub fn clear() {
        CONFIG_STORE.write().unwrap().clear();
    }

    /// Get the number of configuration entries.
    ///
    /// # Returns
    /// * `usize` - The number of configuration entries
    pub fn len() -> usize {
        CONFIG_STORE.read().unwrap().len()
    }

    /// Check if the configuration store is empty.
    ///
    /// # Returns
    /// * `true` if empty, `false` otherwise
    pub fn is_empty() -> bool {
        CONFIG_STORE.read().unwrap().is_empty()
    }

    /// Get configuration entries with a prefix.
    ///
    /// This is useful for getting all configuration for a specific namespace
    /// (e.g., all "torch.*" configurations).
    ///
    /// # Arguments
    /// * `prefix` - The key prefix to filter by
    ///
    /// # Returns
    /// * `BTreeMap<String, Ele>` - All key-value pairs with the given prefix
    ///
    /// # Examples
    /// ```rust
    /// use probing_core::config::store::ConfigStore;
    ///
    /// // Get all torch.* configurations
    /// let torch_configs = ConfigStore::get_with_prefix("torch.");
    /// for (key, value) in torch_configs {
    ///     println!("{} = {:?}", key, value);
    /// }
    /// ```
    pub fn get_with_prefix(prefix: &str) -> BTreeMap<String, Ele> {
        let store = CONFIG_STORE.read().unwrap();
        store
            .range(prefix.to_string()..)
            .take_while(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Remove configuration entries with a prefix.
    ///
    /// # Arguments
    /// * `prefix` - The key prefix to filter by
    ///
    /// # Returns
    /// * `usize` - The number of entries removed
    ///
    /// # Examples
    /// ```rust
    /// use probing_core::config::store::ConfigStore;
    ///
    /// // Remove all torch.* configurations
    /// let removed = ConfigStore::remove_with_prefix("torch.");
    /// println!("Removed {} configurations", removed);
    /// ```
    pub fn remove_with_prefix(prefix: &str) -> usize {
        let mut store = CONFIG_STORE.write().unwrap();
        let keys_to_remove: Vec<String> = store
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();

        for key in &keys_to_remove {
            store.remove(key);
        }

        keys_to_remove.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to ensure clean state before each test
    fn setup_test() {
        ConfigStore::clear();
    }

    // Helper to ensure clean state after each test
    fn teardown_test() {
        ConfigStore::clear();
    }

    #[test]
    fn test_set_and_get() {
        setup_test();

        // Test string values (both &str and String)
        ConfigStore::set("test.key1", "value1");
        assert_eq!(
            ConfigStore::get("test.key1"),
            Some(Ele::Text("value1".to_string()))
        );
        assert_eq!(
            ConfigStore::get_str("test.key1"),
            Some("value1".to_string())
        );

        ConfigStore::set("test.key2", "value2".to_string());
        assert_eq!(
            ConfigStore::get("test.key2"),
            Some(Ele::Text("value2".to_string()))
        );

        // Test bool value
        ConfigStore::set("test.bool.true", true);
        assert_eq!(ConfigStore::get("test.bool.true"), Some(Ele::BOOL(true)));
        assert_eq!(
            ConfigStore::get_str("test.bool.true"),
            Some("True".to_string())
        );
        ConfigStore::set("test.bool.false", false);
        assert_eq!(ConfigStore::get("test.bool.false"), Some(Ele::BOOL(false)));
        assert_eq!(
            ConfigStore::get_str("test.bool.false"),
            Some("False".to_string())
        );

        // Test integer values
        ConfigStore::set("test.i32", 42i32);
        assert_eq!(ConfigStore::get("test.i32"), Some(Ele::I32(42)));
        assert_eq!(ConfigStore::get_str("test.i32"), Some("42".to_string()));

        ConfigStore::set("test.i64", 100i64);
        assert_eq!(ConfigStore::get("test.i64"), Some(Ele::I64(100)));
        assert_eq!(ConfigStore::get_str("test.i64"), Some("100".to_string()));

        // Test float values
        ConfigStore::set("test.f32", 3.14f32);
        assert_eq!(ConfigStore::get("test.f32"), Some(Ele::F32(3.14)));
        assert_eq!(ConfigStore::get_str("test.f32"), Some("3.14".to_string()));

        ConfigStore::set("test.f64", 2.718f64);
        assert_eq!(ConfigStore::get("test.f64"), Some(Ele::F64(2.718)));
        assert_eq!(ConfigStore::get_str("test.f64"), Some("2.718".to_string()));

        teardown_test();
    }

    #[test]
    fn test_contains_key() {
        setup_test();

        ConfigStore::set("test.key", "value");
        assert!(ConfigStore::contains_key("test.key"));
        assert!(!ConfigStore::contains_key("test.nonexistent"));

        teardown_test();
    }

    #[test]
    fn test_remove() {
        setup_test();

        ConfigStore::set("test.key", "value");
        assert!(ConfigStore::contains_key("test.key"));

        let removed = ConfigStore::remove("test.key");
        assert_eq!(removed, Some(Ele::Text("value".to_string())));
        assert!(!ConfigStore::contains_key("test.key"));

        teardown_test();
    }

    #[test]
    fn test_keys() {
        setup_test();

        ConfigStore::set("a.key", "value1");
        ConfigStore::set("b.key", "value2");
        ConfigStore::set("c.key", "value3");

        let keys = ConfigStore::keys();
        assert_eq!(
            keys.len(),
            3,
            "Expected 3 keys, but got {}: {:?}",
            keys.len(),
            keys
        );
        // BTreeMap maintains sorted order
        assert_eq!(keys[0], "a.key");
        assert_eq!(keys[1], "b.key");
        assert_eq!(keys[2], "c.key");

        teardown_test();
    }

    #[test]
    fn test_get_with_prefix() {
        setup_test();

        ConfigStore::set("torch.profiling", "on");
        ConfigStore::set("torch.mode", "random");
        ConfigStore::set("server.port", "8080");

        let torch_configs = ConfigStore::get_with_prefix("torch.");
        assert_eq!(torch_configs.len(), 2);
        assert!(torch_configs.contains_key("torch.profiling"));
        assert!(torch_configs.contains_key("torch.mode"));
        assert!(!torch_configs.contains_key("server.port"));

        teardown_test();
    }

    #[test]
    fn test_remove_with_prefix() {
        setup_test();

        ConfigStore::set("torch.profiling", "on");
        ConfigStore::set("torch.mode", "random");
        ConfigStore::set("server.port", "8080");

        let removed = ConfigStore::remove_with_prefix("torch.");
        assert_eq!(removed, 2);
        assert!(!ConfigStore::contains_key("torch.profiling"));
        assert!(!ConfigStore::contains_key("torch.mode"));
        assert!(ConfigStore::contains_key("server.port"));

        teardown_test();
    }

    #[test]
    fn test_len_and_is_empty() {
        setup_test();

        assert!(ConfigStore::is_empty(), "Store should be empty after clear");
        assert_eq!(ConfigStore::len(), 0, "Length should be 0 after clear");

        ConfigStore::set("test.key", "value");
        assert!(
            !ConfigStore::is_empty(),
            "Store should not be empty after setting a key"
        );
        assert_eq!(
            ConfigStore::len(),
            1,
            "Length should be 1 after setting one key"
        );

        ConfigStore::clear();
        assert!(ConfigStore::is_empty(), "Store should be empty after clear");
        assert_eq!(ConfigStore::len(), 0, "Length should be 0 after clear");

        teardown_test();
    }
}
