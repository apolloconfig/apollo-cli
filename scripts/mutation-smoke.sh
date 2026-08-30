#!/usr/bin/env bash

set -Eeuo pipefail
umask 077

readonly APOLLO_REVISION_DEFAULT="6ca0319decfacf886403d1eb4122ae29a0476003"
readonly APOLLO_REPOSITORY="https://github.com/apolloconfig/apollo.git"
readonly APOLLO_OPENAPI_SPEC_URL="https://raw.githubusercontent.com/apolloconfig/apollo-openapi/v0.3.10/apollo-openapi.yaml"
readonly APOLLO_OPENAPI_SPEC_SHA256="c0cbd94952618c5e56f4948c2707bac8f7907dba913c7184b55d23cfdf39896b"
readonly PORTAL_URL="${APOLLO_SMOKE_PORTAL_URL:-http://127.0.0.1:8070}"
readonly CONFIG_URL="${APOLLO_SMOKE_CONFIG_URL:-http://127.0.0.1:8080}"
readonly ADMIN_URL="${APOLLO_SMOKE_ADMIN_URL:-http://127.0.0.1:8090}"
readonly SMOKE_ENV="LOCAL"
readonly APOLLO_REVISION="${APOLLO_SMOKE_APOLLO_REVISION:-${APOLLO_REVISION_DEFAULT}}"
readonly PORTAL_USERNAME="${APOLLO_SMOKE_PORTAL_USERNAME:-apollo}"
readonly PORTAL_PASSWORD="${APOLLO_SMOKE_PORTAL_PASSWORD:-admin}"
readonly WAIT_TIMEOUT_SECONDS="${APOLLO_SMOKE_WAIT_TIMEOUT_SECONDS:-360}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
REPOSITORY_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd -P)"
TEMP_BASE="${TMPDIR:-/tmp}"
TEMP_BASE="${TEMP_BASE%/}"
SMOKE_WORK_DIR="$(mktemp -d "${TEMP_BASE}/apollo-cli-smoke.XXXXXX")"
RESULTS_DIR="${SMOKE_WORK_DIR}/results"
PORTAL_LOG="${SMOKE_WORK_DIR}/apollo-assembly.log"
APOLLO_PROCESS_ID=""
CURRENT_STEP="initialization"
LAST_STDOUT=""
LAST_STDERR=""
TOKEN_RESULT=""
CAPTURE_INDEX=0
SENSITIVE_VALUES=()

mkdir -p "${RESULTS_DIR}"

sanitize_stream() {
  local line secret
  while IFS= read -r line || [[ -n "${line}" ]]; do
    for secret in "${SENSITIVE_VALUES[@]-}"; do
      if [[ -n "${secret}" ]]; then
        line="${line//${secret}/[REDACTED]}"
      fi
    done
    printf '%s\n' "${line}"
  done
}

cleanup() {
  local status=$?
  local diagnostic_excerpt=""
  set +e

  if [[ ${status} -ne 0 ]]; then
    printf 'Mutation smoke failed during: %s\n' "${CURRENT_STEP}" >&2
    if [[ -s "${LAST_STDOUT}" ]]; then
      printf '%s\n' 'Sanitized CLI stdout:' >&2
      sanitize_stream < "${LAST_STDOUT}" >&2
    fi
    if [[ -s "${LAST_STDERR}" ]]; then
      printf '%s\n' 'Sanitized CLI stderr:' >&2
      sanitize_stream < "${LAST_STDERR}" >&2
    fi
    if [[ -s "${PORTAL_LOG}" ]]; then
      diagnostic_excerpt="$(grep -E -B 5 -A 40 \
        'Create app failed|No available admin server' "${PORTAL_LOG}" 2>/dev/null \
        | tail -n 120 || true)"
      if [[ -n "${diagnostic_excerpt}" ]]; then
        printf '%s\n' 'Sanitized Apollo failure context:' >&2
        printf '%s\n' "${diagnostic_excerpt}" | sanitize_stream >&2
      fi
      printf '%s\n' 'Sanitized Apollo assembly log tail:' >&2
      tail -n 80 "${PORTAL_LOG}" | sanitize_stream >&2
    fi
  fi

  if [[ -n "${APOLLO_PROCESS_ID}" ]]; then
    kill "${APOLLO_PROCESS_ID}" >/dev/null 2>&1 || true
    wait "${APOLLO_PROCESS_ID}" >/dev/null 2>&1 || true
  fi

  if [[ -d "${SMOKE_WORK_DIR}" && "${SMOKE_WORK_DIR}" == "${TEMP_BASE}/apollo-cli-smoke."* ]]; then
    rm -rf -- "${SMOKE_WORK_DIR}"
  fi
}
trap cleanup EXIT

fail() {
  printf '%s\n' "$1" >&2
  return 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "Required command is not available: $1"
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

assert_files_do_not_contain_sensitive_data() {
  local label=$1
  shift
  local file secret
  for file in "$@"; do
    [[ -f "${file}" ]] || continue
    for secret in "${SENSITIVE_VALUES[@]-}"; do
      [[ -n "${secret}" ]] || continue
      if LC_ALL=C grep -F -- "${secret}" "${file}" >/dev/null 2>&1; then
        fail "${label} exposed a credential or config value"
      fi
    done
  done
}

next_capture_paths() {
  CAPTURE_INDEX=$((CAPTURE_INDEX + 1))
  LAST_STDOUT="${RESULTS_DIR}/$(printf '%03d' "${CAPTURE_INDEX}").stdout.json"
  LAST_STDERR="${RESULTS_DIR}/$(printf '%03d' "${CAPTURE_INDEX}").stderr.log"
}

run_cli_with_token() {
  local token=$1
  local label=$2
  shift 2
  local status

  CURRENT_STEP="${label}"
  next_capture_paths
  set +e
  APOLLO_TOKEN="${token}" "${CLI_BINARY}" \
    --profile "${PROFILE_NAME}" --output json "$@" \
    >"${LAST_STDOUT}" 2>"${LAST_STDERR}"
  status=$?
  set -e

  if [[ ${status} -ne 0 ]]; then
    fail "${label} returned exit status ${status}"
  fi
  jq -e . "${LAST_STDOUT}" >/dev/null || fail "${label} did not return valid JSON"
  assert_files_do_not_contain_sensitive_data "${label}" "${LAST_STDOUT}" "${LAST_STDERR}"
  printf 'ok - %s\n' "${label}"
}

run_cli() {
  local label=$1
  shift
  run_cli_with_token "${PRIMARY_TOKEN}" "${label}" "$@"
}

run_cli_expect_failure() {
  local token=$1
  local label=$2
  local expected_category=$3
  shift 3
  local status

  CURRENT_STEP="${label}"
  next_capture_paths
  set +e
  APOLLO_TOKEN="${token}" "${CLI_BINARY}" \
    --profile "${PROFILE_NAME}" --output json "$@" \
    >"${LAST_STDOUT}" 2>"${LAST_STDERR}"
  status=$?
  set -e

  if [[ ${status} -eq 0 ]]; then
    fail "${label} unexpectedly succeeded"
  fi
  jq -e --arg category "${expected_category}" \
    '.error.category == $category' "${LAST_STDERR}" >/dev/null \
    || fail "${label} did not return error category ${expected_category}"
  assert_files_do_not_contain_sensitive_data "${label}" "${LAST_STDOUT}" "${LAST_STDERR}"
  printf 'ok - %s\n' "${label}"
}

assert_jq() {
  local label=$1
  local file=$2
  shift 2
  CURRENT_STEP="${label}"
  jq -e "$@" "${file}" >/dev/null || fail "JSON assertion failed: ${label}"
}

server_get() {
  local token=$1
  local path=$2
  local output_file=$3
  local expected_status=$4
  local status

  status="$(curl --connect-timeout 3 --max-time 20 -sS \
    -o "${output_file}" -w '%{http_code}' \
    -H "Authorization: Bearer ${token}" \
    "${PORTAL_URL}${path}")"
  if [[ "${status}" != "${expected_status}" ]]; then
    fail "Server state read returned HTTP ${status}; expected ${expected_status}"
  fi
  if [[ "${expected_status}" == "200" ]]; then
    jq -e . "${output_file}" >/dev/null || fail "Server state read did not return valid JSON"
  fi
}

assert_item_value() {
  local cluster=$1
  local namespace=$2
  local key=$3
  local expected_value=$4
  local output_file="${RESULTS_DIR}/server-item-${cluster}-${namespace}-${key}.json"

  CURRENT_STEP="verify ${cluster}/${namespace}/${key} server state"
  server_get "${PRIMARY_TOKEN}" \
    "/openapi/v1/envs/${SMOKE_ENV}/apps/${APP_ID}/clusters/${cluster}/namespaces/${namespace}/items/${key}" \
    "${output_file}" 200
  jq -e --arg key "${key}" --arg value "${expected_value}" \
    '.key == $key and .value == $value' "${output_file}" >/dev/null \
    || fail "Server item state did not match the expected key and value"
}

assert_item_absent() {
  local cluster=$1
  local namespace=$2
  local key=$3
  local output_file="${RESULTS_DIR}/server-item-absent-${cluster}-${namespace}-${key}.json"

  CURRENT_STEP="verify ${cluster}/${namespace}/${key} is absent"
  server_get "${PRIMARY_TOKEN}" \
    "/openapi/v1/envs/${SMOKE_ENV}/apps/${APP_ID}/clusters/${cluster}/namespaces/${namespace}/items/${key}" \
    "${output_file}" 404
}

create_user_token() {
  local label=$1
  local payload=$2
  local response_file="${RESULTS_DIR}/token-${label}.json"
  local status

  CURRENT_STEP="create ${label} user token"
  status="$(curl --connect-timeout 3 --max-time 20 -sS \
    -o "${response_file}" -w '%{http_code}' \
    --user "${PORTAL_USERNAME}:${PORTAL_PASSWORD}" \
    -H 'Content-Type: application/json' \
    -X POST "${PORTAL_URL}/openapi/v1/user-tokens" \
    --data "${payload}")"
  if [[ "${status}" != "200" ]]; then
    fail "Creating ${label} user token returned HTTP ${status}"
  fi
  TOKEN_RESULT="$(jq -er '.tokenValue | select(startswith("apollo_pat_"))' "${response_file}")" \
    || fail "Creating ${label} user token returned an invalid response"
  : > "${response_file}"
}

wait_for_apollo() {
  local deadline=$((SECONDS + WAIT_TIMEOUT_SECONDS))
  local services

  CURRENT_STEP="wait for Apollo Portal and AdminService readiness"
  while (( SECONDS < deadline )); do
    if ! kill -0 "${APOLLO_PROCESS_ID}" >/dev/null 2>&1; then
      fail "Apollo assembly exited before its services became ready"
    fi
    if curl --connect-timeout 2 --max-time 5 -fsS "${CONFIG_URL}/health" >/dev/null 2>&1 \
      && curl --connect-timeout 2 --max-time 5 -fsS "${ADMIN_URL}/health" >/dev/null 2>&1 \
      && curl --connect-timeout 2 --max-time 5 -fsS "${PORTAL_URL}/signin" >/dev/null 2>&1; then
      services="$(curl --connect-timeout 2 --max-time 5 -fsS \
        "${CONFIG_URL}/services/admin" 2>/dev/null || true)"
      if [[ "${services}" == *"apollo-adminservice"* ]]; then
        printf '%s\n' 'Apollo Portal, ConfigService, and AdminService are ready'
        return 0
      fi
    fi
    sleep 3
  done
  fail "Timed out after ${WAIT_TIMEOUT_SECONDS}s waiting for Apollo readiness"
}

wait_for_portal_admin_cache() {
  local deadline=$((SECONDS + WAIT_TIMEOUT_SECONDS))
  local readiness_body="${RESULTS_DIR}/portal-admin-readiness.json"
  local status

  CURRENT_STEP="wait for Portal AdminService cache readiness"
  while (( SECONDS < deadline )); do
    if ! kill -0 "${APOLLO_PROCESS_ID}" >/dev/null 2>&1; then
      fail "Apollo assembly exited before Portal's AdminService cache became ready"
    fi
    status="$(curl --connect-timeout 2 --max-time 5 -sS \
      -o "${readiness_body}" -w '%{http_code}' \
      -H "Authorization: Bearer ${PRIMARY_TOKEN}" \
      "${PORTAL_URL}/openapi/v1/envs/${SMOKE_ENV}/apps/${APP_ID}/clusters/default/namespaces/application" \
      2>/dev/null || true)"
    if [[ "${status}" == "404" ]]; then
      : > "${readiness_body}"
      printf '%s\n' "Portal's AdminService cache is ready"
      return 0
    fi
    sleep 3
  done
  fail "Timed out after ${WAIT_TIMEOUT_SECONDS}s waiting for Portal's AdminService cache"
}

for command_name in awk cargo curl git java jq; do
  require_command "${command_name}"
done
if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
  fail "Required SHA-256 command is not available: sha256sum or shasum"
fi

CURRENT_STEP="prepare pinned Apollo source"
if [[ -n "${APOLLO_SMOKE_APOLLO_SOURCE:-}" ]]; then
  APOLLO_SOURCE_DIR="$(cd "${APOLLO_SMOKE_APOLLO_SOURCE}" && pwd -P)"
  actual_revision="$(git -C "${APOLLO_SOURCE_DIR}" rev-parse HEAD)"
  if [[ "${actual_revision}" != "${APOLLO_REVISION}" ]]; then
    fail "Local Apollo source is at ${actual_revision}, expected pinned revision ${APOLLO_REVISION}"
  fi
  if ! git -C "${APOLLO_SOURCE_DIR}" diff --quiet \
    || ! git -C "${APOLLO_SOURCE_DIR}" diff --cached --quiet; then
    fail "Local Apollo source has tracked changes; use a clean checkout of ${APOLLO_REVISION}"
  fi
else
  APOLLO_SOURCE_DIR="${SMOKE_WORK_DIR}/apollo"
  git init -q "${APOLLO_SOURCE_DIR}"
  git -C "${APOLLO_SOURCE_DIR}" remote add origin "${APOLLO_REPOSITORY}"
  git -C "${APOLLO_SOURCE_DIR}" fetch --depth=1 origin "${APOLLO_REVISION}"
  git -C "${APOLLO_SOURCE_DIR}" checkout -q --detach FETCH_HEAD
fi
printf 'Using Apollo revision %s\n' "${APOLLO_REVISION}"

CURRENT_STEP="build Apollo CLI"
(cd "${REPOSITORY_ROOT}" && cargo build --release --locked)
CLI_BINARY="${REPOSITORY_ROOT}/target/release/apollo"

CURRENT_STEP="build Apollo assembly"
OPENAPI_SPEC="${SMOKE_WORK_DIR}/apollo-openapi-v0.3.10.yaml"
curl -fL --retry 5 --retry-all-errors --connect-timeout 10 --max-time 60 -sS \
  -o "${OPENAPI_SPEC}" "${APOLLO_OPENAPI_SPEC_URL}"
actual_openapi_sha256="$(sha256_file "${OPENAPI_SPEC}")"
if [[ "${actual_openapi_sha256}" != "${APOLLO_OPENAPI_SPEC_SHA256}" ]]; then
  fail "Apollo OpenAPI specification checksum did not match the pinned digest"
fi
(cd "${APOLLO_SOURCE_DIR}" && ./mvnw -B -ntp -q -pl apollo-assembly -am \
  -DskipTests -Dapollo.openapi.spec.url="${OPENAPI_SPEC}" package)

ASSEMBLY_JAR=""
while IFS= read -r candidate; do
  ASSEMBLY_JAR="${candidate}"
  break
done < <(find "${APOLLO_SOURCE_DIR}/apollo-assembly/target" -maxdepth 1 -type f \
  -name 'apollo-assembly-*.jar' ! -name '*-sources.jar' ! -name '*-javadoc.jar' | sort)
[[ -n "${ASSEMBLY_JAR}" ]] || fail "No runnable Apollo assembly jar was produced"

CURRENT_STEP="start Apollo assembly"
SPRING_PROFILES_ACTIVE="github,database-discovery,auth" \
SPRING_SQL_CONFIG_INIT_MODE="always" \
SPRING_SQL_PORTAL_INIT_MODE="always" \
SPRING_CONFIG_DATASOURCE_URL="jdbc:h2:mem:apollo-config-db;mode=mysql;DB_CLOSE_ON_EXIT=FALSE;DB_CLOSE_DELAY=-1;BUILTIN_ALIAS_OVERRIDE=TRUE;DATABASE_TO_UPPER=FALSE" \
SPRING_PORTAL_DATASOURCE_URL="jdbc:h2:mem:apollo-portal-db;mode=mysql;DB_CLOSE_ON_EXIT=FALSE;DB_CLOSE_DELAY=-1;BUILTIN_ALIAS_OVERRIDE=TRUE;DATABASE_TO_UPPER=FALSE" \
LOGGING_FILE_NAME="${SMOKE_WORK_DIR}/apollo-assembly-file.log" \
java -jar "${ASSEMBLY_JAR}" >"${PORTAL_LOG}" 2>&1 &
APOLLO_PROCESS_ID=$!
wait_for_apollo

nonce="${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}-$(date +%s)-${RANDOM}"
nonce="${nonce//[^a-zA-Z0-9-]/-}"
readonly APP_ID="cli-smoke-${nonce}"
readonly PROFILE_NAME="smoke-${nonce}"
readonly SOURCE_CLUSTER="default"
readonly TARGET_CLUSTER="smoke-target"
readonly CONFIG_NAMESPACE="smoke-config"
readonly SOURCE_ONLY_KEY="source-only"
readonly SHARED_KEY="shared-key"
readonly TARGET_ONLY_KEY="target-only"
readonly CONFIRMATION_KEY="confirmation-guard"
readonly DENIED_KEY="permission-guard"
readonly SOURCE_ONLY_VALUE="source-${nonce}"
readonly SOURCE_V1_VALUE="source-v1-${nonce}"
readonly SOURCE_V2_VALUE="source-v2-${nonce}"
readonly TARGET_OLD_VALUE="target-old-${nonce}"
readonly TARGET_ONLY_VALUE="target-only-${nonce}"
readonly CONFIRMATION_VALUE="confirmation-${nonce}"
readonly DENIED_VALUE="denied-${nonce}"

full_token_payload="$(jq -cn --arg name "cli-smoke-full-${nonce}" --arg env "${SMOKE_ENV}" '{
  name: $name,
  operations: [
    "config:read", "config:modify", "config:release", "namespace:create",
    "namespace:delete", "cluster:create", "app:create", "app:manage-role", "system:admin"
  ],
  envs: [$env],
  rateLimit: 0
}')"
create_user_token "full" "${full_token_payload}"
readonly PRIMARY_TOKEN="${TOKEN_RESULT}"
SENSITIVE_VALUES+=(
  "${PRIMARY_TOKEN}"
  "${SOURCE_ONLY_VALUE}"
  "${SOURCE_V1_VALUE}"
  "${SOURCE_V2_VALUE}"
  "${TARGET_OLD_VALUE}"
  "${TARGET_ONLY_VALUE}"
  "${CONFIRMATION_VALUE}"
  "${DENIED_VALUE}"
)
wait_for_portal_admin_cache

export APOLLO_CLI_HOME="${SMOKE_WORK_DIR}/cli-home"
CURRENT_STEP="configure isolated Apollo CLI profile"
next_capture_paths
"${CLI_BINARY}" --output json profile add "${PROFILE_NAME}" \
  --server "${PORTAL_URL}" --auth-mode user-token --use \
  >"${LAST_STDOUT}" 2>"${LAST_STDERR}"
jq -e --arg profile "${PROFILE_NAME}" --arg home "${APOLLO_CLI_HOME}" \
  '.profile == $profile and .activeProfile == $profile
   and (.configPath | startswith($home + "/"))' "${LAST_STDOUT}" >/dev/null \
  || fail "The isolated CLI profile was not configured as expected"
assert_files_do_not_contain_sensitive_data "isolated profile setup" "${LAST_STDOUT}" "${LAST_STDERR}"
printf '%s\n' 'ok - configure isolated Apollo CLI profile'

app_body="$(jq -cn --arg app "${APP_ID}" '{
  assignAppRoleToSelf: true,
  admins: ["apollo"],
  app: {
    appId: $app,
    name: $app,
    orgId: "TEST1",
    orgName: "Sample Department 1",
    ownerName: "apollo",
    ownerEmail: "apollo@localhost"
  }
}')"
run_cli "create isolated test app" --yes api post /openapi/v1/apps --body "${app_body}"
assert_jq "app creation returns a redacted raw API operation" "${LAST_STDOUT}" \
  --arg path "/openapi/v1/apps" \
  '.operation.operation == "api.post" and .operation.request.method == "POST"
   and .operation.request.path == $path and (.operation.request | has("body") | not)'

cluster_body="$(jq -cn --arg app "${APP_ID}" --arg cluster "${TARGET_CLUSTER}" \
  '{appId: $app, name: $cluster}')"
run_cli "create target cluster" --yes api post \
  "/openapi/v1/envs/${SMOKE_ENV}/apps/${APP_ID}/clusters" --body "${cluster_body}"
run_cli "create source namespace" --yes namespace create \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${SOURCE_CLUSTER}" "${CONFIG_NAMESPACE}"
run_cli "verify target namespace" namespace get \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${TARGET_CLUSTER}" "${CONFIG_NAMESPACE}"

run_cli "set source-only config" --yes config set \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${SOURCE_CLUSTER}" --namespace "${CONFIG_NAMESPACE}" \
  "${SOURCE_ONLY_KEY}" "${SOURCE_ONLY_VALUE}"
assert_jq "--yes retains config target summary" "${LAST_STDOUT}" \
  --arg app "${APP_ID}" --arg cluster "${SOURCE_CLUSTER}" --arg namespace "${CONFIG_NAMESPACE}" --arg key "${SOURCE_ONLY_KEY}" \
  '.operation.operation == "config.set" and .operation.target.app == $app
   and .operation.target.cluster == $cluster and .operation.target.namespace == $namespace
   and .operation.key == $key'

run_cli "set source shared config" --yes config set \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${SOURCE_CLUSTER}" --namespace "${CONFIG_NAMESPACE}" \
  "${SHARED_KEY}" "${SOURCE_V1_VALUE}"
run_cli "set target shared config" --yes config set \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${TARGET_CLUSTER}" --namespace "${CONFIG_NAMESPACE}" \
  "${SHARED_KEY}" "${TARGET_OLD_VALUE}"
run_cli "set target-only config" --yes config set \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${TARGET_CLUSTER}" --namespace "${CONFIG_NAMESPACE}" \
  "${TARGET_ONLY_KEY}" "${TARGET_ONLY_VALUE}"

run_cli_expect_failure "${PRIMARY_TOKEN}" "reject mutation without --yes" \
  confirmation_required config set \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${TARGET_CLUSTER}" --namespace "${CONFIG_NAMESPACE}" \
  "${CONFIRMATION_KEY}" "${CONFIRMATION_VALUE}"
assert_jq "confirmation error retains a redacted operation" "${LAST_STDERR}" \
  --arg key "${CONFIRMATION_KEY}" \
  '.error.operation.operation == "config.set" and .error.operation.key == $key'
assert_item_absent "${TARGET_CLUSTER}" "${CONFIG_NAMESPACE}" "${CONFIRMATION_KEY}"

readonly_token_payload="$(jq -cn --arg name "cli-smoke-readonly-${nonce}" --arg app "${APP_ID}" --arg env "${SMOKE_ENV}" '{
  name: $name,
  operations: ["config:read"],
  appIds: [$app],
  envs: [$env],
  rateLimit: 0
}')"
create_user_token "readonly" "${readonly_token_payload}"
readonly READONLY_TOKEN="${TOKEN_RESULT}"
SENSITIVE_VALUES+=("${READONLY_TOKEN}")
run_cli_expect_failure "${READONLY_TOKEN}" "reject mutation without permission" \
  permission_denied --yes config set \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${SOURCE_CLUSTER}" --namespace "${CONFIG_NAMESPACE}" \
  "${DENIED_KEY}" "${DENIED_VALUE}"
assert_item_absent "${SOURCE_CLUSTER}" "${CONFIG_NAMESPACE}" "${DENIED_KEY}"

run_cli "preview initial config merge" config diff \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${SOURCE_CLUSTER}" --namespace "${CONFIG_NAMESPACE}" \
  --target-env "${SMOKE_ENV}" --target-cluster "${TARGET_CLUSTER}"
assert_jq "initial diff reports add/update/preserve contract" "${LAST_STDOUT}" \
  '.data.result == "preview" and .data.strategy == "merge"
   and .data.targetOnlyBehavior == "preserve"
   and .data.changes == {"create":1,"update":1,"delete":0,"unchanged":0}'

run_cli "apply initial config merge" --yes config apply \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${SOURCE_CLUSTER}" --namespace "${CONFIG_NAMESPACE}" \
  --target-env "${SMOKE_ENV}" --target-cluster "${TARGET_CLUSTER}"
assert_jq "initial apply reports the approved change set" "${LAST_STDOUT}" \
  '.data.result == "applied"
   and .data.changes == {"create":1,"update":1,"delete":0,"unchanged":0}
   and .operation.changes == .data.changes
   and .operation.strategy == "merge" and .operation.targetOnlyBehavior == "preserve"'
assert_item_value "${TARGET_CLUSTER}" "${CONFIG_NAMESPACE}" "${SOURCE_ONLY_KEY}" "${SOURCE_ONLY_VALUE}"
assert_item_value "${TARGET_CLUSTER}" "${CONFIG_NAMESPACE}" "${SHARED_KEY}" "${SOURCE_V1_VALUE}"
assert_item_value "${TARGET_CLUSTER}" "${CONFIG_NAMESPACE}" "${TARGET_ONLY_KEY}" "${TARGET_ONLY_VALUE}"

run_cli "read target config through CLI" config list \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${TARGET_CLUSTER}" \
  --namespace "${CONFIG_NAMESPACE}" --size 500
assert_jq "CLI config read keeps all values redacted" "${LAST_STDOUT}" \
  '.data.content | length == 3 and all(.value == "[REDACTED]")'

run_cli "create first release" --yes release create \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${TARGET_CLUSTER}" \
  --namespace "${CONFIG_NAMESPACE}" \
  --title "smoke-v1-${nonce}"
RELEASE_V1_ID="$(jq -er '.data.id' "${LAST_STDOUT}")"

run_cli "update source shared config" --yes config set \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${SOURCE_CLUSTER}" \
  --namespace "${CONFIG_NAMESPACE}" \
  "${SHARED_KEY}" "${SOURCE_V2_VALUE}"
run_cli "apply config update" --yes config apply \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${SOURCE_CLUSTER}" \
  --namespace "${CONFIG_NAMESPACE}" --target-env "${SMOKE_ENV}" \
  --target-cluster "${TARGET_CLUSTER}"
assert_jq "update apply reports one update" "${LAST_STDOUT}" \
  '.data.result == "applied"
   and .data.changes == {"create":0,"update":1,"delete":0,"unchanged":1}'
assert_item_value "${TARGET_CLUSTER}" "${CONFIG_NAMESPACE}" "${SHARED_KEY}" "${SOURCE_V2_VALUE}"
assert_item_value "${TARGET_CLUSTER}" "${CONFIG_NAMESPACE}" "${TARGET_ONLY_KEY}" "${TARGET_ONLY_VALUE}"

target_state_before_noop="${RESULTS_DIR}/target-before-noop.json"
server_get "${PRIMARY_TOKEN}" \
  "/openapi/v1/envs/${SMOKE_ENV}/apps/${APP_ID}/clusters/${TARGET_CLUSTER}/namespaces/${CONFIG_NAMESPACE}/items?page=0&size=500" \
  "${target_state_before_noop}" 200
run_cli "apply no-op config merge" --yes config apply \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${SOURCE_CLUSTER}" \
  --namespace "${CONFIG_NAMESPACE}" --target-env "${SMOKE_ENV}" \
  --target-cluster "${TARGET_CLUSTER}"
assert_jq "no-op apply is deterministic" "${LAST_STDOUT}" \
  '.data.result == "no-op"
   and .data.changes == {"create":0,"update":0,"delete":0,"unchanged":2}'
target_state_after_noop="${RESULTS_DIR}/target-after-noop.json"
server_get "${PRIMARY_TOKEN}" \
  "/openapi/v1/envs/${SMOKE_ENV}/apps/${APP_ID}/clusters/${TARGET_CLUSTER}/namespaces/${CONFIG_NAMESPACE}/items?page=0&size=500" \
  "${target_state_after_noop}" 200
jq -S '.content | sort_by(.key)' "${target_state_before_noop}" > "${RESULTS_DIR}/target-before-noop.normalized.json"
jq -S '.content | sort_by(.key)' "${target_state_after_noop}" > "${RESULTS_DIR}/target-after-noop.normalized.json"
cmp -s "${RESULTS_DIR}/target-before-noop.normalized.json" \
  "${RESULTS_DIR}/target-after-noop.normalized.json" \
  || fail "No-op apply changed target server state"

run_cli "create second release" --yes release create \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${TARGET_CLUSTER}" \
  --namespace "${CONFIG_NAMESPACE}" \
  --title "smoke-v2-${nonce}"
RELEASE_V2_ID="$(jq -er '.data.id' "${LAST_STDOUT}")"
[[ "${RELEASE_V1_ID}" != "${RELEASE_V2_ID}" ]] || fail "Release IDs were not unique"

run_cli "list active releases" release list \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${TARGET_CLUSTER}" \
  --namespace "${CONFIG_NAMESPACE}" --size 20
assert_jq "release list contains both releases and redacts configurations" "${LAST_STDOUT}" \
  --argjson v1 "${RELEASE_V1_ID}" --argjson v2 "${RELEASE_V2_ID}" \
  '(.data | map(.id)) as $ids
   | ($ids | index($v1)) != null and ($ids | index($v2)) != null
   and all(.data[]; .configurations == "[REDACTED]")'

active_releases_before_rollback="${RESULTS_DIR}/active-releases-before-rollback.json"
server_get "${PRIMARY_TOKEN}" \
  "/openapi/v1/envs/${SMOKE_ENV}/apps/${APP_ID}/clusters/${TARGET_CLUSTER}/namespaces/${CONFIG_NAMESPACE}/releases/active?page=0&size=20" \
  "${active_releases_before_rollback}" 200
jq -e --argjson id "${RELEASE_V2_ID}" --arg key "${SHARED_KEY}" --arg value "${SOURCE_V2_VALUE}" \
  'any(.[]; .id == $id and .configurations[$key] == $value)' \
  "${active_releases_before_rollback}" >/dev/null \
  || fail "Second release did not contain the applied server state"

run_cli "roll back to first release" --yes release rollback \
  --env "${SMOKE_ENV}" "${RELEASE_V2_ID}" --to-release-id "${RELEASE_V1_ID}"
assert_jq "rollback reports both release IDs" "${LAST_STDOUT}" \
  --argjson v1 "${RELEASE_V1_ID}" --argjson v2 "${RELEASE_V2_ID}" \
  '.operation.operation == "release.rollback"
   and .operation.releaseId == $v2 and .operation.toReleaseId == $v1'

run_cli "list releases after rollback" release list \
  --env "${SMOKE_ENV}" --app "${APP_ID}" --cluster "${TARGET_CLUSTER}" \
  --namespace "${CONFIG_NAMESPACE}" --size 20
assert_jq "rolled-back release is no longer active" "${LAST_STDOUT}" \
  --argjson v1 "${RELEASE_V1_ID}" --argjson v2 "${RELEASE_V2_ID}" \
  '(.data | map(.id)) as $ids
   | ($ids | index($v1)) != null and ($ids | index($v2)) == null'

active_releases_after_rollback="${RESULTS_DIR}/active-releases-after-rollback.json"
server_get "${PRIMARY_TOKEN}" \
  "/openapi/v1/envs/${SMOKE_ENV}/apps/${APP_ID}/clusters/${TARGET_CLUSTER}/namespaces/${CONFIG_NAMESPACE}/releases/active?page=0&size=20" \
  "${active_releases_after_rollback}" 200
jq -e --argjson v1 "${RELEASE_V1_ID}" --argjson v2 "${RELEASE_V2_ID}" \
  --arg shared "${SHARED_KEY}" --arg shared_value "${SOURCE_V1_VALUE}" \
  --arg kept "${TARGET_ONLY_KEY}" --arg kept_value "${TARGET_ONLY_VALUE}" \
  'any(.[]; .id == $v1 and .configurations[$shared] == $shared_value
      and .configurations[$kept] == $kept_value)
   and (any(.[]; .id == $v2) | not)' \
  "${active_releases_after_rollback}" >/dev/null \
  || fail "Rollback did not restore the first release as the active server state"

assert_files_do_not_contain_sensitive_data "Apollo assembly log" "${PORTAL_LOG}"
CURRENT_STEP="completed"
printf '%s\n' 'Apollo CLI mutation smoke completed successfully'
