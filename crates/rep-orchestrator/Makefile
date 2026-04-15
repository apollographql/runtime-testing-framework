# Local kind cluster management

.PHONY: cluster-setup
cluster-setup:
	@./local-stack/setup.sh

.PHONY: cluster-teardown
cluster-teardown:
	@./local-stack/teardown.sh

# Test actions

.PHONY: db-tests
db-tests:
	@RTF_DB_HOST=localhost \
		RTF_DB_PORT=5432 \
		RTF_DB_NAME=rtf_rep \
		RTF_DB_USER=service_user \
		RTF_DB_PASS=password \
		RTF_APOLLO_KEY=dummy \
		RTF_GITHUB_TOKEN=dummy \
		RTF_KUBECONFIG_PATH=dummy \
		RTF_MGMT_CONTEXT=dummy \
		RTF_WORKLOAD_CONTEXT=dummy \
		RTF_GCS_BUCKET=dummy \
		RTF_MOCK_GCS_URL="localhost:8035" \
		cargo test --lib --features db_tests $(ARGS)

.PHONY: integration-tests
integration-tests: clear-db-data
	@RTF_DB_HOST=localhost \
		RTF_DB_PORT=5432 \
		RTF_DB_NAME=rtf_rep \
		RTF_DB_USER=service_user \
		RTF_DB_PASS=password \
		RTF_APOLLO_KEY=dummy \
		RTF_GITHUB_TOKEN=dummy \
		RTF_KUBECONFIG_PATH=$(HOME)/.kube/config \
		RTF_MGMT_CONTEXT=kind-rtf-mgmt \
		RTF_WORKLOAD_CONTEXT=kind-rtf-workload \
		RTF_GCS_BUCKET=dummy \
		RTF_MOCK_GCS_URL="localhost:8035" \
		cargo test --features k8s_tests --test '*' $(ARGS)

.PHONY: test-all
test-all: db-tests integration-tests


# CI actions

.PHONY: ci-tests
ci-tests: cluster-setup cluster-ci


# Tilt stack management

.PHONY: cluster-up
cluster-up:
	@cd local-stack && tilt up --context kind-rtf-mgmt

.PHONY: cluster-ci
cluster-ci:
	@cd local-stack && tilt ci --context kind-rtf-mgmt -- --orchestrator-tests=true

.PHONY: cluster-down
cluster-down:
	@cd local-stack && tilt down --context kind-rtf-mgmt


# DB stack management

.PHONY: db-up
db-up:
	@docker compose -f local-stack/db/docker-compose.yaml up

.PHONY: db-down
db-down:
	@docker compose -f local-stack/db/docker-compose.yaml down

.PHONY: db-shell
db-shell:
	@docker exec -it db psql -U service_user rtf_rep


# Management of the test DB container

.PHONY: clear-db-data
clear-db-data:
	@kubectl exec -i -n orchestrator --context kind-rtf-mgmt statefulset/postgres -- \
		env PGPASSWORD=password psql -U service_user rtf_rep \
		< local-stack/db/clear.sql
