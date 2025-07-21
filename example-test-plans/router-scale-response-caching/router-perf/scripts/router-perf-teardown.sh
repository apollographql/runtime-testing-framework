#!/usr/bin/bash

OUTDIR="${OUTDIR:-$(pwd)}"
RESULTS_DIR="$OUTDIR/results"

echo "extracting data from redis"

# Extract information about redis utilization (we do this before we shut down redis)
echo "INFO" | redis-cli > "${RESULTS_DIR}/redis.info"
# Just grab 100 keys from redis. Might not be helpful, but downloading the full dataset is definitely not useful
echo 'SCAN 0 MATCH "*" COUNT 100' | redis-cli > "${RESULTS_DIR}/redis.keys"
# Extract information about redis cluster utilization (we do this before we shut down redis cluster)
echo "INFO" | redis-cli -c --pass router -p 6385 > "${RESULTS_DIR}/redis-cluster.info"
# Just grab 100 keys from redis. Might not be helpful, but downloading the full dataset is definitely not useful
echo 'SCAN 0 MATCH "*" COUNT 100' | redis-cli -c --pass router -p 6385 > "${RESULTS_DIR}/redis-cluster.keys"

echo "extracting data from postgres"
# for now just exec into the postgres container. this will need to be changed if we start using a managed postgres
docker exec -it postgres psql --quiet -U postgres --command "select count(*) from cache" postgres > "${RESULTS_DIR}/postgres_cache_count"
docker exec -it postgres psql --quiet -U postgres --command "select count(*) from invalidation_key" postgres > "${RESULTS_DIR}/postgres_invalidation_key_count"

# We may have prometheus metrics to harvest, let's try the default location
curl http://127.0.0.1:9090/metrics > "${RESULTS_DIR}/prometheus.metrics"

echo "stopping the router"
# Sometimes a router won't shutdown: we give it 5 seconds and then we're more forceful
pkill -x router
for router_pid in ${ROUTER_PIDS}; do
  timeout 5s tail --pid="${router_pid}" -f /dev/null || kill -3 "${router_pid}"
done

echo "stopping additional services"
pkill -3 -x top
pkill -3 -x strace
pkill -3 -x router-side
pkill -x subgraph
pkill -x snapshot
docker compose -f "$REDIS_DOCKER_COMPOSE" down > /dev/null 2>&1
for service in otel redis postgres; do 
  docker ps --filter "name=${service}" --format '{{.ID}}' | xargs --no-run-if-empty docker kill
done

echo "removing cgroups"
if [ "$ROUTER_CGROUP" = "true" ]; then
  sudo cgdelete cpu,memory:/router/leaf
  sudo cgdelete cpu,memory:/router
fi

sudo cgdelete cpu,memory:/subgraph/leaf
sudo cgdelete cpu,memory:/subgraph

echo "processing top output"
jc -q --top-s < "${RESULTS_DIR}/top.user"  > "${RESULTS_DIR}/top.user.json"
jc -q --top-s < "${RESULTS_DIR}/top.router"  > "${RESULTS_DIR}/top.router.json"
jc -q --top-s < "${RESULTS_DIR}/top.redis"  > "${RESULTS_DIR}/top.redis.json"
jc -q --top-s < "${RESULTS_DIR}/top.router-side"  > "${RESULTS_DIR}/top.router-side.json"

echo "removing temp data"
# Cleanup or we will run out of disk space very quickly (-f so we don't get a warning if the file isn't present)
rm -f perf.data
rm -f /tmp/router.*
