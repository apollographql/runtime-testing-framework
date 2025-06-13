#!/usr/bin/bash

TEST_DIR="scale/tests"
RESULTS_DIR="$TEST_DIR/results"

# Extract information about redis utilization (we do this before we shut down redis)
echo "INFO" | redis-cli > "${RESULTS_DIR}/redis.info"
# Just grab 100 keys from redis. Might not be helpful, but downloading the full dataset is definitely not useful
echo 'SCAN 0 MATCH "*" COUNT 100' | redis-cli > "${RESULTS_DIR}/redis.keys"
# Extract information about redis cluster utilization (we do this before we shut down redis cluster)
echo "INFO" | redis-cli -c --pass router -p 6385 > "${RESULTS_DIR}/redis-cluster.info"
# Just grab 100 keys from redis. Might not be helpful, but downloading the full dataset is definitely not useful
echo 'SCAN 0 MATCH "*" COUNT 100' | redis-cli -c --pass router -p 6385 > "${RESULTS_DIR}/redis-cluster.keys"

# We may have prometheus metrics to harvest, let's try the default location
curl http://127.0.0.1:9090/metrics > "${RESULTS_DIR}/prometheus.metrics"

# Sometimes a router won't shutdown: we give it 5 seconds and then we're more forceful
pkill -x router
timeout 5s tail --pid="${ROUTER_PID}" -f /dev/null || kill -3 "${ROUTER_PID}"

pkill -3 -x top
pkill -3 -x strace
pkill -3 -x router-side
pkill -x subgraph
pkill -x snapshot
docker kill otel
docker kill redis
docker compose -f scale/scripts/docker-compose-redis.yml down > /dev/null 2>&1

if [ "$ROUTER_CGROUP" = "true" ]; then
  sudo cgdelete cpu,memory:/router/leaf
  sudo cgdelete cpu,memory:/router
fi

sudo cgdelete cpu,memory:/subgraph/leaf
sudo cgdelete cpu,memory:/subgraph

jc -q --top-s < "${RESULTS_DIR}/top.user"  > "${RESULTS_DIR}/top.user.json"
jc -q --top-s < "${RESULTS_DIR}/top.router"  > "${RESULTS_DIR}/top.router.json"
jc -q --top-s < "${RESULTS_DIR}/top.redis"  > "${RESULTS_DIR}/top.redis.json"
jc -q --top-s < "${RESULTS_DIR}/top.router-side"  > "${RESULTS_DIR}/top.router-side.json"

# Cleanup or we will run out of disk space very quickly (-f so we don't get a warning if the file isn't present)
rm -f perf.data
rm -f /tmp/router.*
