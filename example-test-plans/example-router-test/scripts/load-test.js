import http from 'k6/http';
import exec from 'k6/execution';
import { sleep } from 'k6';
import { Rate } from 'k6/metrics';

const GRAPHQL_URL = __ENV.GRAPHQL_URL || 'http://localhost:4000';
const HEALTH_CHECK_URL = __ENV.HEALTH_CHECK_URL || '';
const RPS = Number(__ENV.RPS || 10);
const DURATION = __ENV.DURATION || '60s';
const WARMUP_DURATION = __ENV.WARMUP_DURATION || '10s';
const MAX_VUS = Number(__ENV.MAX_VUS || 50);

// Newline-delimited JSON, one {query, variables, operationName?} per line (graphos_canned_ops).
const operations = open(__ENV.CANNED_OPS_FILE)
  .split('\n')
  .filter((line) => line.trim())
  .map((line) => JSON.parse(line));
if (operations.length === 0) {
  throw new Error('No operations found in canned ops file');
}

// Serialized once at init to keep JSON encoding off the per-request path.
const payloads = operations.map((op) =>
  JSON.stringify({
    query: op.query,
    variables: op.variables,
    ...(op.operationName ? { operationName: op.operationName } : {}),
  })
);

const params = {
  headers: {
    'Content-Type': 'application/json',
    // The router 406s @defer operations unless the client declares multipart support.
    Accept: 'multipart/mixed;deferSpec=20220824, application/json',
    // Lets this traffic be filtered out by client in GraphOS Studio.
    'apollographql-client-name': 'rtf-example-router-test',
    'apollographql-client-version': 'local',
  },
};

// An HTTP 200 carrying an `errors` array, which k6's built-in http_req_failed can't see.
const graphqlErrors = new Rate('graphql_errors');

// Every VU is allocated up front so VU init cost never lands inside the measured window.
export const options = {
  scenarios: {
    warmup: {
      executor: 'ramping-arrival-rate',
      startRate: 0,
      timeUnit: '1s',
      preAllocatedVUs: MAX_VUS,
      maxVUs: MAX_VUS,
      stages: [{ target: RPS, duration: WARMUP_DURATION }],
    },
    load: {
      executor: 'constant-arrival-rate',
      rate: RPS,
      timeUnit: '1s',
      duration: DURATION,
      preAllocatedVUs: MAX_VUS,
      maxVUs: MAX_VUS,
      startTime: WARMUP_DURATION,
    },
  },
};

export function setup() {
  if (HEALTH_CHECK_URL) {
    waitForHealthy(HEALTH_CHECK_URL);
  }
  console.log(`Sending ${operations.length} operations round-robin to ${GRAPHQL_URL} at ${RPS} rps`);
}

export default function () {
  const index = exec.scenario.iterationInTest % payloads.length;
  const response = http.post(GRAPHQL_URL, payloads[index], params);
  if (response.status === 200) {
    // Substring scan works for both JSON and multipart @defer bodies without parsing either.
    graphqlErrors.add(response.body.indexOf('"errors"') !== -1);
  }
}

// Polling rather than a single check tells "still starting" apart from "down".
function waitForHealthy(url, timeoutMs = 60000, intervalMs = 2000) {
  const deadline = Date.now() + timeoutMs;
  let lastStatus = 'no response';
  while (Date.now() < deadline) {
    const response = http.get(url);
    if (response.status >= 200 && response.status < 300) {
      return;
    }
    lastStatus = response.error || response.status;
    sleep(intervalMs / 1000);
  }
  throw new Error(`${url} was not healthy within ${timeoutMs}ms (last status: ${lastStatus})`);
}
