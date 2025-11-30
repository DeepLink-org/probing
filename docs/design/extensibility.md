# Extensibility Framework

Probing's core capability lies in its ability to both deeply access target processes to obtain critical performance and fault data, and flexibly inject diagnostic logic and custom code into target processes. To meet diverse monitoring and analysis needs, users can customize and enhance Probing's functionality through its powerful extension system, enabling more effective problem-solving in specific scenarios.

Probing provides two main extension paths: lightweight Python-based extensions and high-performance Rust-based extensions. Python extensions excel in flexibility in real production environments due to their convenience of not requiring Probing rebuilds, while Rust extensions provide developers with lower-level control and higher execution efficiency, suitable for scenarios with extreme performance requirements or deep system integration needs.

## Extending Probing with Python

### Direct Python Calls from SQL

Probing supports directly calling Python functions or variables in SQL queries as a quick external table:

```bash
# Call Python standard library functions
probing $ENDPOINT query "SELECT * from python.`time.time()`"

# Call custom package functions
probing $ENDPOINT query "SELECT * FROM python.`pkg.some_func()`"
```

Under the `python` namespace, use backticks "`" to reference Python code. Probing will execute the code and return the result as a table for subsequent SQL execution.

### Writing Data to Probing from Python

Probing supports using `dataclass` to define custom data tables:

```python
from dataclasses import dataclass
from probing.core import table

@table
@dataclass
class MetricData:
    timestamp: int
    cpu_usage: float
    memory_mb: int
    process_name: str

# Write data
MetricData.append(MetricData(1234567890, 85.2, 1024, "python"))

# Or more directly
MetricData(1234567890, 85.2, 1024, "python").save()
```

Written data can be queried via SQL:
```bash
probing $ENDPOINT query "SELECT * FROM python.metric_data"
```

## Rust Extension System

Using Rust, developers can more directly and deeply extend Probing's data query and processing capabilities.

### Core Interface

```rust
pub trait EngineExtension: Debug + Send + Sync + EngineCall + EngineDatasource {
    fn name(&self) -> String;                                    // Extension name
    fn set(&mut self, key: &str, value: &str) -> Result<...>;   // Set configuration
    fn get(&self, key: &str) -> Result<String, ...>;            // Get configuration
    fn options(&self) -> Vec<EngineExtensionOption>;             // List all configuration options
}
```

The `EngineDatasource` interface is responsible for feeding data to Probing's data processing core `DataFusion`, while the `EngineCall` interface endows the Probing server with additional remote API call capabilities. This elegantly designed interface system together forms the foundation of Probing's extension system.

### Data Source Core Interface - EngineDatasource

Probing's data processing capabilities are built on a flexible data source abstraction. When we talk about performance diagnostics and monitoring, data collection is the foundation of all analysis. The `EngineDatasource` interface is the core of this data bridge, responsible for connecting various data sources to Probing's query engine.

```rust
pub trait EngineDatasource {
    fn datasrc(&self, namespace: &str, name: Option<&str>)
        -> Option<Arc<dyn Plugin + Sync + Send>>;
}
```

`EngineExtension` provides data source plugins to Probing by implementing the `EngineDatasource` interface. In Probing, all data is organized and accessed in the form of `namespace.table_name`. Based on this, data source plugins are mainly divided into two types:

- `CustomTable` (Static table plugin): These plugins provide data tables with fixed structure and content, very suitable for displaying system inherent information, configuration items, or infrequently changing datasets.
- `CustomNamespace` (Dynamic namespace plugin): These plugins can dynamically generate table lists and content based on query requests, suitable for encapsulating complex data sources with frequently changing data structures or content, or interacting with external systems to obtain real-time data.

These two plugin types together form the skeleton of Probing's data access. Static tables can be imagined as pre-defined, fixed-structure views in a data warehouse, while dynamic namespaces are more like intelligent, on-demand data proxies that can flexibly connect and transform data from various external systems. This two-layer design ensures both ease of use and efficiency when processing simple, static data, and provides sufficient flexibility and powerful functionality for integrating complex, dynamic data sources.

Through this unified interface design, users can integrate various originally scattered, heterogeneous data sources into Probing's powerful SQL query system. This means developers no longer need to write specialized parsing and processing code for each new data format or data source. Whether monitoring system metrics like CPU usage, analyzing network traffic, checking application logs, or even querying custom business data, all can be accomplished through a standard, consistent SQL interface, greatly simplifying the complexity of data acquisition and analysis.

This seemingly simple `EngineDatasource` interface contains powerful extension capabilities. It allows Probing to access a wide variety of heterogeneous data sources in a unified and standardized way—from performance counters within the operating system, internal state information exposed by applications, to query results from external databases, and even API responses from remote microservices. When an extension implements this interface, it is actually contributing a new, explorable and analyzable dimension to Probing's vast data universe. Specifically, the interface design brings the following key advantages:

- **Clear namespace management**: Through the `namespace` parameter, Probing can logically group and isolate data from different sources and types, forming a hierarchical, easily manageable data view.
- **Flexible dynamic data discovery**: The optional design of the `name` parameter allows the `EngineDatasource` interface to support both precise queries for specific data tables and dynamic discovery and enumeration of all available data sources under the entire namespace, which is particularly important for exploring unknown or dynamically changing data environments.
- **Powerful polymorphic plugin system**: The interface's return value uses Rust's trait object design, allowing different types and implementations of data source plugins to be seamlessly integrated into Probing's unified query framework and processed consistently.

A typical `EngineDatasource` implementation might connect to the operating system's performance counters to collect system load information, query an application's internal state to monitor its health, pull business data from a specialized database, or even call a remote service API to obtain third-party information. Through the adaptation and transformation of the `EngineDatasource` interface, Probing unifies these diverse data sources into structured, SQL-queryable table forms, enabling developers to use familiar SQL syntax for complex, cross-data-source deep data analysis and insights.

### Data Source Plugins

When we delve deeper into Probing's data access mechanism, we find that its true power comes from diverse data source plugin implementations. Probing provides two core plugin types, each handling different data access responsibilities.

**Static Table Plugin (TablePlugin):**

Static table plugins are the most intuitive data providers in Probing's extension system. They are like pre-defined data views with fixed structure and content, very suitable for displaying system configuration, static information, or cached data.

```rust
pub trait CustomTable {
    fn name() -> &'static str;      // Table name
    fn schema() -> SchemaRef;       // Table schema
    fn data() -> Vec<RecordBatch>;  // Data batches
}

// Register plugin
let plugin = TablePluginHelper::<MyTable>::create("namespace", "table_name");
```

Plugins implementing this trait can provide data access capabilities with minimal code. Developers only need to define the table name, data structure, and content, and Probing will automatically handle SQL query parsing, data filtering, and result return operations. This simplicity makes it very easy to quickly develop data tables for specific purposes.

In performance diagnostic scenarios, static tables can be used to display system basic information, such as CPU core count, memory configuration, runtime environment variables, or compilation options. Although simple, this information is crucial for understanding the overall system state.

**Dynamic Table Plugin (SchemaPlugin):**
```rust
#[async_trait]
pub trait CustomNamespace {
    async fn table_names(&self) -> Result<Vec<String>>;
    async fn table(&self, name: &str) -> Result<Arc<dyn TableProvider>>;
}
```

Unlike static tables, dynamic namespace plugins provide greater flexibility. They can dynamically generate table lists and content based on query requests, enabling real-time interaction with external data sources. This design is very suitable for handling changing datasets, such as real-time monitoring metrics, log streams, or remote API responses.

The core advantage of dynamic namespaces lies in their adaptive capability. The `table_names` method allows plugins to dynamically discover available data tables based on the current environment, while the `table` method is responsible for creating query interfaces for specific tables on demand. This lazy loading strategy not only improves resource utilization efficiency but also enables plugins to adapt to complex and changing data source environments.

In practical applications, a dynamic namespace plugin might connect to the operating system's process list and dynamically create independent data tables for each process, displaying their resource usage, open file handles, or memory mappings. This dynamically generated table collection provides developers with a rich and precise system view.

### API Interface Plugin

In addition to data source plugins, Probing also provides another powerful extension point: API interfaces. Through the `EngineCall` trait, extensions can add custom HTTP endpoints to the Probing server, implementing more complex interaction logic and command execution capabilities.

```rust
#[async_trait]
pub trait EngineCall {
    async fn call(
        &self,
        path: &str,                           // API path
        params: &HashMap<String, String>,     // Query parameters
        body: &[u8],                         // Request body
    ) -> Result<Vec<u8>, EngineError>;       // Response data
}
```

This interface design is concise and flexible, similar to a miniature web service framework. Extensions can implement this trait to handle various requests from clients:

- Execute dynamically generated code snippets
- Inject diagnostic logic into target processes
- Provide interactive debugging control
- Modify monitoring parameters in real-time
- Export analysis results to external formats

The combination of API interfaces and data sources forms Probing's complete capability loop: data sources provide the "eyes" to observe system state, while API interfaces are the "hands" to intervene in system behavior. Through the synergy of these two capabilities, extensions can implement complete workflows from problem discovery to fault diagnosis to dynamic repair.

**Existing Rust Extension Examples:**
- **C/C++ Extension**: Native code debugging, memory analysis, system call tracing
- **Python Extension**: REPL, flame graphs, call stack tracing, object inspection
- **Torch Extension**: PyTorch model monitoring and performance analysis

### Configuration Management

```rust
// Extension configuration option definition
pub struct EngineExtensionOption {
    pub key: String,           // Configuration key
    pub value: Option<String>, // Current value
    pub help: &'static str,    // Help information
}

// Runtime configuration
manager.set_option("my_option", "new_value")?;
let value = manager.get_option("my_option")?;
```

## Extension Development Guide

### Python Extension Development

1. **Lightweight Extension**: Write Python functions directly, call them in SQL
2. **Heavy Extension**: Use `@table` decorator to create data tables with persistence support
3. **Test Integration**: Ensure data type conversion is correct and SQL queries work normally

### Rust Extension Development

1. **Implement Core Traits**: Implement `EngineCall` and/or `EngineDatasource` based on requirements
2. **Define Configuration Options**: List all configurable parameters in the `options()` method
3. **Register Plugin**: Register to the system through `EngineExtensionManager`
4. **Performance Optimization**: Leverage Arrow columnar storage and DataFusion query optimization

The extension system is deeply integrated with the DataFusion query engine, supporting standard SQL syntax to access all data sources provided by extensions.
