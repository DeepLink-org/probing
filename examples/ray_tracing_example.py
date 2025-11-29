"""
Example: Using probing with Ray for automatic tracing.

This example demonstrates how to use probing.ext.ray to automatically
trace Ray tasks and actors using Ray's _tracing_startup_hook mechanism.

The implementation follows Ray's official tracing documentation:
https://docs.ray.io/en/latest/ray-observability/user-guides/ray-tracing.html

This follows the same pattern as Ray's OpenTelemetry example:
ray.init(_tracing_startup_hook="ray.util.tracing.setup_local_tmp_tracing:setup_tracing")
"""

import ray

# Initialize Ray with probing tracing hook (same pattern as OpenTelemetry)
ray.init(
    _tracing_startup_hook="probing.ext.ray:setup_tracing",
    runtime_env={
        # "pip": ["probing"],  # Install probing in workers
        "env_vars": {"PROBING": "1"}  # Enable probing in workers
    }
)

# Define a remote task
# This will be automatically traced when executed
@ray.remote
def compute_sum(a, b):
    """A simple computation task."""
    return a + b

# Define an actor
# Actor methods will also be automatically traced
@ray.remote
class Counter:
    """A simple counter actor."""
    
    def __init__(self, initial_value=0):
        self.value = initial_value
    
    def increment(self, amount=1):
        """Increment the counter."""
        self.value += amount
        return self.value
    
    def get_value(self):
        """Get the current counter value."""
        return self.value

# Execute tasks - they will be automatically traced
print("Executing tasks...")
results = []
for i in range(5):
    result = compute_sum.remote(i, i * 2)
    results.append(result)

# Wait for results
results = ray.get(results)
print(f"Task results: {results}")

# Use an actor - methods will be automatically traced
print("\nUsing actor...")
counter = Counter.remote(initial_value=10)
counter.increment.remote(5)
counter.increment.remote(3)
value = ray.get(counter.get_value.remote())
print(f"Counter value: {value}")

# Query tracing data using probing
print("\nQuerying trace data...")
try:
    import probing
    from probing.ext.ray import get_ray_timeline, get_ray_timeline_chrome_format
    
    # Query for Ray task spans
    task_spans = probing.query(
        "SELECT * FROM TraceEvent WHERE name LIKE 'ray.task.%'"
    )
    print(f"Found {len(task_spans)} task spans")
    
    # Query for Ray actor method spans
    actor_spans = probing.query(
        "SELECT * FROM TraceEvent WHERE name LIKE 'ray.actor.%'"
    )
    print(f"Found {len(actor_spans)} actor method spans")
    
    # Get timeline (similar to Ray dashboard)
    print("\nGetting Ray task timeline...")
    timeline = get_ray_timeline()
    print(f"Found {len(timeline)} timeline entries")
    
    # Show timeline entries
    if timeline:
        print("\nTimeline entries:")
        for entry in timeline[:5]:
            duration_ms = entry["duration"] / 1_000_000 if entry["duration"] else None
            print(f"  - {entry['name']} ({entry['type']})")
            print(f"    Start: {entry['start_time']}, Duration: {duration_ms}ms" if duration_ms else f"    Start: {entry['start_time']}")
    
    # Export to Chrome tracing format
    print("\nExporting timeline to Chrome format...")
    chrome_trace = get_ray_timeline_chrome_format()
    print(f"Chrome trace format (first 200 chars): {chrome_trace[:200]}...")
    print("Save this to a .json file and open in chrome://tracing")
            
except Exception as e:
    print(f"Error querying trace data: {e}")

for i in range(1000):
    import time
    time.sleep(1)

# Cleanup
ray.shutdown()

print("\nExample completed!")

