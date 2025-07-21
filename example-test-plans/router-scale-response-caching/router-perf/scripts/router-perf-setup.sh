#!/usr/bin/bash

# TODO:
#   - support setting response headers in subgraphs
#   - run REST API snapshot servers for Apollo Connectors
#   - support flamegraph

OUTDIR="${OUTDIR:-$(pwd)}"
RESULTS_DIR="$OUTDIR/results"

echo "sourcing data files"
set -a
. "$ROUTER_CGROUP_CONFIG"
set +a

# Wait for an application to become available (using nc)
function wait_for {
  address=$1
  port=$2
  application=$3
  duration=$4

  i=0
  while [ $i -lt "${duration}" ]; do
    nc -z "${address}" "${port}" && break;
    sleep 1;
    i=$(( i + 1 ))
  done

  if [ $i -eq "${duration}" ]; then
    echo "${application} has not started, terminating..."
    exit 1
  fi
}


# Prefix each line of command output with a label.
function label {
  local LABEL="$1"
  shift

  "$@" 1> >(sed "s/^/[$LABEL] /") 2> >(sed "s/^/[$LABEL] /" 1>&2)
}

# Fetch or build the router
function fetch_router {
  rm -f ~/.cargo/bin/router
  cd "$OUTDIR" || exit 1
  sh "$ROUTER_INSTALL_SCRIPT"

  mkdir -p ~/.cargo/bin
  
  # depending on the install script, the router might be in either:
  # * ~/.cargo/bin/router
  # * ./router
  if [[ -f "./router" ]]; then
    mv ./router ~/.cargo/bin
  fi

  cd ~ || exit 1

  if [[ ! -f ".cargo/bin/router" ]]; then
    echo "ERROR: Couldn't find a router in ~/.cargo/bin/. Exiting"
    exit 1
  fi

  if ! router config validate "$ROUTER_CONFIG"; then
    echo "ERROR: Router config file is invalid. Exiting"
    exit 1
  fi
}

function cleanup_supporting_services {
  echo "cleaning up supporting services"
  for service in otel redis postgres; do 
    docker ps --filter "name=${service}" --format '{{.ID}}' | xargs --no-run-if-empty docker kill
  done

  if docker compose ls | grep "$REDIS_DOCKER_COMPOSE" ; then
    docker compose -f "$REDIS_DOCKER_COMPOSE" down > /dev/null 2>&1
  fi

  pkill -3 -x top
  pkill -3 -x strace
  pkill -3 -x router-side
  pkill -3 -x router
  pkill -x subgraph
}

function run_supporting_services {
  # Jaeger runs for the whole life of the host, don't kill it...
  # It is bound to port 6666 because we are relaying traces to it from otel collector on port 4317
  if [[ $(docker ps -q --filter name=^/jaeger$ | wc -l) != 1 ]]; then
    label "start jaeger" docker run \
      --name jaeger \
      --rm \
      --detach \
      -e COLLECTOR_OTLP_ENABLED=true \
      -p 16686:16686 \
      -p 0.0.0.0:6666:4317 \
      jaegertracing/all-in-one
  fi

  # Our otel collector is configured to relay tracing details to Jaeger
  label "start otel" docker run \
    --name otel \
    --rm \
    --detach \
    -p 4317:4317 \
    -p 4318:4318 \
    --add-host=host.docker.internal:host-gateway \
    --mount type=bind,source="${OTEL_CONFIG}",target=/etc/otelcol-contrib/config.yaml \
    otel/opentelemetry-collector-contrib:0.103.1
  label "otel" docker logs --follow otel > "${RESULTS_DIR}/otel.log" 2>&1 &

  label "start redis" docker run \
    --name redis \
    --rm \
    --detach \
    -p 6379:6379 \
    redis
  wait_for 127.0.0.1 6379 "redis" 10

  label "start postgres" docker run \
    --name postgres \
    --rm \
    --detach \
    -p 5432:5432 \
    -e POSTGRES_USER=postgres \
    -e POSTGRES_DB=postgres \
    cimg/postgres:17.0
  wait_for 127.0.0.1 5432 "postgres" 10

  label "redis-cluster" docker compose -f "$REDIS_DOCKER_COMPOSE" up --detach > /dev/null 2>&1
  wait_for 127.0.0.1 6385 "redis-cluster" 10
}

function run_subgraphs {
  # Create a cgroup to constrain the subgraphs
  sudo cgcreate -g cpu,memory:/subgraph
  echo "+cpu +memory" | sudo tee /sys/fs/cgroup/subgraph/cgroup.subtree_control > /dev/null
  sudo cgset -r memory.swap.max="0" /subgraph
  sudo cgset -r cpu.max="1600000 100000" /subgraph
  sudo cgset -r memory.high="8G" /subgraph
  sudo cgset -r memory.max="8G" /subgraph
  # Make a leaf for our process (https://unix.stackexchange.com/questions/680167/ebusy-when-trying-to-add-process-to-cgroup-v2)
  sudo mkdir /sys/fs/cgroup/subgraph/leaf

  port=$(( 4000 + NUM_ROUTERS ))

  # We need to build out overrides for the locations of each subgraph
  sg_tmp="$(mktemp /tmp/sg.XXXXXX)"
  echo "override_subgraph_url:" >> "$sg_tmp"

  # Kick off the subgraph binaries
  for file in "$SUBGRAPH_DIR"/*; do
    file="${file#"$SUBGRAPH_DIR/"}"
    name="${file%.graphql}"

    echo "  $name: http://127.0.0.1:$port" >> "$sg_tmp"

    # read $SUBGRAPH_CONFIG yaml, combining parameters of `default` and `override.$name`, and use those values for the
    # subgraph arguments
    subgraph_params=$(overridePath="(.override.${name} // {})" yq -o=j '.default + eval(strenv(overridePath))' "${SUBGRAPH_CONFIG}")

    # TODO: I'd prefer if we could automatically generate all of these but I had a bunch of issues with bash expansion
    #  and did it the less optimal way instead
    label "subgraph:${name}" subgraph -port="$port" -schema="$SUPERGRAPH_SCHEMA" \
      -latency="$(echo "$subgraph_params" | yq .latency)" \
      -sine-period="$(echo "$subgraph_params" | yq .sine-period)" \
      -sine-amplitude="$(echo "$subgraph_params" | yq .sine-amplitude)" \
      -header="$(echo "$subgraph_params" | yq .header)" \
      -array-length="$(echo "$subgraph_params" | yq .array-length)" \
      -id-length="$(echo "$subgraph_params" | yq .id-length)" \
      -string-length="$(echo "$subgraph_params" | yq .string-length)" \
      -cache-no-store-ratio="$(echo "$subgraph_params" | yq .cache-no-store-ratio)" \
      -int-max="$(echo "$subgraph_params" | yq .int-max)" &

    SUBGRAPH_PID=$!

    # Move our subgraph pid into the cgroup
    sudo cgclassify -g cpu,memory:/subgraph/leaf "${SUBGRAPH_PID}"
    port="$(( port + 1 ))"
  done

  # Override the addresses to use for subgraphs in the router config
  config=${sg_tmp} yq -i '.override_subgraph_url = load(strenv(config)).override_subgraph_url' "$ROUTER_CONFIG"
}

function run_router_side {
  label "side" router-side 127.0.0.1:1234 &
  ROUTER_SIDE_PID=$!
  strace -tf -p ${ROUTER_SIDE_PID} > "${RESULTS_DIR}/router-side.strace" 2>&1 &
}

function set_up_router_cgroup {
  sudo cgcreate -g cpu,memory:/router
  echo "+cpu +memory" | sudo tee /sys/fs/cgroup/router/cgroup.subtree_control > /dev/null
  sudo cgset -r memory.swap.max="0" /router

  if [ -n "$ROUTER_CPU_REQ" ]; then
    # Multiply by 100000 to get the right units
    sudo cgset -r cpu.max="$(( ROUTER_CPU_REQ * 100000 )) 100000" /router
  fi

  if [ -n "$ROUTER_MEM_REQ" ]; then
    sudo cgset -r memory.high="$ROUTER_MEM_REQ" /router
  fi

  if [ -n "$ROUTER_MEM_LIM" ]; then
    sudo cgset -r memory.max="$ROUTER_MEM_LIM" /router
  else
    sudo cgset -r memory.max="max" /router
  fi

  sudo mkdir /sys/fs/cgroup/router/leaf
}

# shellcheck disable=SC2086
function run_router {
  port_offset=$1

  port=$(( 4000 + port_offset ))
  prom_port=$(( 9090 + port_offset ))

  license_arg=""
  if [ -f "$LICENSE_FILE" ]; then
    license_arg="--license $LICENSE_FILE"
  fi

  # copy the router config file and modify it for this router
  config_file="$ROUTER_CONFIG.$port"
  cp "$ROUTER_CONFIG" "$config_file"
  listen="127.0.0.1:$port" yq -i '.supergraph.listen = strenv(listen)' "$config_file"
  listen="127.0.0.1:$port" yq -i '.health_check.listen = strenv(listen)' "$config_file"
  listen="127.0.0.1:$port" yq -i '.experimental_response_cache.invalidation.listen = strenv(listen)' "$config_file"
  listen="127.0.0.1:$prom_port" yq -i '.telemetry.exporters.metrics.prometheus.listen = strenv(listen)' "$config_file"

  # NB: prefixing this with `label "router"` causes the output to not be properly redirected and then 'returned'
  # as one of the ROUTER_PIDS
  router -s "$SUPERGRAPH_SCHEMA" -c "$config_file" $license_arg > "$RESULTS_DIR/router_${port}.log" &
  router_pid="$!"

  if [ "${ROUTER_CGROUP}" = "true" ]; then
    sudo cgclassify -g cpu,memory:/router/leaf "$router_pid"
  fi

  echo "$router_pid"
}

function run_routers {
  num_routers=$1

  declare -a router_pids

  for (( i=0 ; i<num_routers ; i++ )); do
    router_pid="$(run_router "$i")"
    router_pids[i]=$router_pid
  done

  echo "${router_pids[*]}"
}

# == Main script ==

# Cleanup or we will run out of disk space very quickly (-f so we don't get a warning if the file isn't present)
rm -f perf.data
rm -f /tmp/router.*

rm -rf "$RESULTS_DIR"
mkdir -p "$RESULTS_DIR"

# redirect output from functions to stderr so we keep stdout clean for returning our "provides"

fetch_router
cleanup_supporting_services

top -d 0.49 -bu "$(id -u)" > "${RESULTS_DIR}/top.user" &

run_supporting_services
run_subgraphs
run_router_side

ROUTER_CGROUP=false
if [ -n "$ROUTER_CPU_REQ" ] || [ -n "$ROUTER_MEM_REQ" ] || [ -n "$ROUTER_MEM_LIM" ]; then
  ROUTER_CGROUP="true"
  set_up_router_cgroup
fi

ROUTER_PIDS="$(run_routers "${NUM_ROUTERS}")"

# It can take a while for a router to start, let's wait for up to a minute
for (( i=0 ; i<num_routers ; i++ )); do
  port=$(( 4000 + i ))
  wait_for 127.0.0.1 "${port}" "router_${port}" 60
done

# Monitor some elements of our system more directly
top -d 0.49 -bp $(pgrep -x redis-server -d,) > "${RESULTS_DIR}/top.redis" &
top -d 0.49 -bp $(pgrep -x router-side -d,) > "${RESULTS_DIR}/top.router-side" &
for pid in $ROUTER_PIDS; do 
  top -d 0.49 -bp "$pid" > "${RESULTS_DIR}/top.router.$pid" &
done

# provide data required by the teardown script
echo "{ \"router_pids\": \"$ROUTER_PIDS\", \"router_cgroup\": \"$ROUTER_CGROUP\" }" >> "$RTF_OUTPUT"
