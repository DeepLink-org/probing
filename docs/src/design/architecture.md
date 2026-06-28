# System Architecture

!!! warning "Legacy document"
    This two-layer overview is **superseded** by **[Modularity & Boundaries](modularity.md)** (four-layer model, crate map, dependency rules, and ownership). It is kept for historical diagrams only. **New contributors should start with [Modularity](modularity.md)**, then [Data Layer](data-layer.md) and [Distributed overview](distributed.md).

Probing is designed as a simple two-layer structure to minimize complexity and deployment difficulty.

## High-Level Architecture

```mermaid
graph TB
    subgraph "User Interface"
        CLI[CLI Client]
        HTTP[HTTP API]
        WEB[Web UI]
    end

    subgraph "Target Process"
        PROBE[Probe]
        subgraph "Probe Components"
            ENGINE[Engine]
            SERVER[Server]
            EXT[Extensions]
        end
    end

    CLI --> |Unix Socket / TCP| PROBE
    HTTP --> |HTTP/REST| PROBE
    WEB --> |WebSocket| PROBE

    PROBE --> ENGINE
    PROBE --> SERVER
    PROBE --> EXT
```

## Components

### 1. Probe

Injected into target processes to gain full access to all resources:

- Python interpreter access
- File system access
- Memory inspection
- Network capabilities

The probe runs an embedded HTTP server that listens on:

- **Unix domain socket** - For local connections (default)
- **TCP port** - For remote connections

### 2. CLI

Command-line interface for controlling probes:

- Process discovery and listing
- Probe injection and management
- Query execution
- Code evaluation

Communication happens via HTTP protocol over Unix domain sockets (local) or TCP (remote).

### 3. HTTP API

RESTful API for programmatic access:

- All CLI commands available as endpoints
- WebSocket support for real-time data
- Integration with monitoring tools

## Probe Internal Architecture

```mermaid
graph LR
    subgraph "Probe"
        SERVER[HTTP Server]
        ENGINE[Query Engine]
        CONFIG[Config Manager]

        subgraph "Extensions"
            PYTHON[Python Extension]
            TORCH[PyTorch Extension]
            CUSTOM[Custom Tables]
        end
    end

    SERVER --> ENGINE
    SERVER --> CONFIG
    ENGINE --> PYTHON
    ENGINE --> TORCH
    ENGINE --> CUSTOM
```

### Engine

Core data storage and processing:

- **DataFusion** - SQL query engine
- **Arrow** - Columnar data format
- Time-series data storage
- Configuration management

### Server

HTTP server handling:

- Request routing
- Authentication (optional)
- WebSocket connections
- Response formatting

### Extensions

Pluggable data providers:

- **Python Extension** - Backtrace, variables
- **PyTorch Extension** - Torch traces, memory
- **Custom Tables** - User-defined data sources

## Data Flow

```mermaid
sequenceDiagram
    participant CLI
    participant Probe
    participant Engine
    participant Extension

    CLI->>Probe: query "SELECT * FROM python.torch_trace"
    Probe->>Engine: Parse & Plan Query
    Engine->>Extension: Request Data
    Extension-->>Engine: Arrow RecordBatch
    Engine-->>Probe: Query Results
    Probe-->>CLI: JSON Response
```

## Communication Protocol

### Local Connection

```
probing -t <pid> query "..."
         |
         v
    Unix Socket: /tmp/probing-<pid>.sock
         |
         v
    HTTP Request: POST /query
```

### Remote Connection

```
probing -t host:port query "..."
         |
         v
    TCP Connection: host:port
         |
         v
    HTTP Request: POST /query
```

## Security Considerations

- **Local mode**: Unix socket permissions (process owner only)
- **Remote mode**: Optional authentication token via `PROBING_TOKEN`

## Performance Characteristics

Probing is designed for minimal overhead on training workloads:

| Aspect | Design approach |
|--------|----------------|
| Overhead | Lock-free mmap writes on hot path; sampling in background threads |
| Memory | Fixed-size ring buffers per table (MEMT); bounded by retention config |
| Latency | Queries execute against in-process DataFusion; no network round-trip for local access |
| Throughput | Columnar (Arrow) scan; `information_schema` for introspection |
