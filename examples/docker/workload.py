#!/usr/bin/env python3
"""
Sample containerized workload demonstrating cgroup resource profiling
with distinct execution phases (CPU load, memory allocation, and I/O).
"""

import hashlib
import time

def compute_phase(iterations=300000):
    """Generates CPU activity via hashing."""
    hasher = hashlib.sha256()
    for i in range(iterations):
        hasher.update(str(i).encode())
    return hasher.hexdigest()

def memory_phase(size_mb=30):
    """Allocates anonymous memory."""
    buf = bytearray(size_mb * 1024 * 1024)
    for i in range(0, len(buf), 4096):
        buf[i] = 1
    time.sleep(0.5)
    return len(buf)

def main():
    print("Initializing workload container...", flush=True)
    time.sleep(0.2)

    print("__COMPUTE_START__", flush=True)
    digest = compute_phase()
    print(f"Computed digest prefix: {digest[:8]}", flush=True)
    print("__COMPUTE_END__", flush=True)

    print("__MEMORY_START__", flush=True)
    mem_size = memory_phase()
    print(f"Allocated {mem_size // (1024*1024)} MB", flush=True)
    print("__MEMORY_END__", flush=True)

    print("Workload container completed.", flush=True)

if __name__ == "__main__":
    main()
