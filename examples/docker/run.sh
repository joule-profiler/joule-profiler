#!/usr/bin/env bash
# run.sh - Demonstration of container energy and resource profiling with Joule Profiler
set -euo pipefail

IMAGE_NAME="joule-profiler-demo:latest"
CONTAINER_NAME="joule-demo-container"

echo "=== 1. Building demo container image ==="
docker build -t "$IMAGE_NAME" "$(dirname "$0")"

echo ""
echo "=== 2. Starting background container ==="
docker rm -f "$CONTAINER_NAME" 2>/dev/null || true
docker run -d --name "$CONTAINER_NAME" "$IMAGE_NAME" python3 -c "
import time
print('Daemon running...')
while True:
    time.sleep(1)
"

# Allow container cgroup hierarchy to initialize
sleep 0.5

echo ""
echo "=== 3. Profiling container workload via Joule Profiler ==="
echo "Running computation inside container while profiling with cgroup source..."

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
PROFILER_BIN="$REPO_ROOT/target/debug/joule-profiler"

# Use joule-profiler binary to profile the target container
# Note: Rootless execution is automatically supported via systemd cgroup v2 delegation.
"$PROFILER_BIN" \
    --sources cgroup \
    --container "$CONTAINER_NAME" \
    profile -- docker exec "$CONTAINER_NAME" python3 /app/workload.py

echo ""
echo "=== 4. Cleaning up container ==="
docker rm -f "$CONTAINER_NAME" >/dev/null

echo "Demo completed successfully!"
