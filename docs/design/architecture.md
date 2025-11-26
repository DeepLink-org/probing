# System Architecture

Probing is designed as a simple two-layer structure:

1. **Probe**: Injected into target processes to gain full access to all resources of the target process, including the Python interpreter, files, memory, etc. The probe runs an embedded HTTP server that listens on a Unix domain socket (for local connections) or TCP port (for remote connections).
2. **CLI**: Command-line interface for controlling probes within processes, reading data, or executing code. Communicates with probes via HTTP protocol over Unix domain sockets (local) or TCP (remote). Users can also control probes directly through the HTTP API.

The overall design minimizes unnecessary components to reduce overall complexity and deployment difficulty.

Inside the probe, there are three main components:

1. **Engine**: Contains core data storage and processing capabilities, and provides infrastructure such as configuration management and extension mechanisms.
2. **Server**: Responsible for interaction with the CLI.
3. **Extensions**: Provide performance data and debugging capabilities for the probe.
