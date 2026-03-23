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
		cargo test --features db_tests --test '*' $(ARGS)

.PHONY: compose-integration-tests
compose-integration-tests: compose-clear-db-data
	@RTF_DB_HOST=localhost \
		RTF_DB_PORT=5432 \
		RTF_DB_NAME=rtf_rep \
		RTF_DB_USER=service_user \
		RTF_DB_PASS=password \
		RTF_APOLLO_KEY=dummy \
		RTF_GITHUB_TOKEN=dummy \
		cargo test --features db_tests --test '*' $(ARGS)

.PHONY: test-all
test-all: db-tests integration-tests


# CI actions

.PHONY: ci-db-tests
ci-db-tests: test-db-up-detached db-tests test-db-down

.PHONY: ci-compose-tests
ci-compose-tests: compose-up-detached wait-for-server db-tests compose-integration-tests compose-down

.PHONY: ci-tests
ci-tests: cluster-setup cluster-ci integration-tests cluster-down

.PHONY: wait-for-server
wait-for-server:
	@echo "Waiting for server to start..."
	@until docker compose -f docker/docker-compose.yaml ps | grep -E '^orchestrator\s' | grep healthy; do \
		echo "..."; \
		sleep 5; \
	done


# Tilt stack management

.PHONY: cluster-up
cluster-up:
	@cd local-stack && tilt up --context kind-rtf-mgmt

.PHONY: cluster-up-prod
cluster-up-prod:
	@cd local-stack && tilt up --context kind-rtf-mgmt -- --mode=prod

.PHONY: cluster-ci
cluster-ci:
	@cd local-stack && tilt ci --context kind-rtf-mgmt -- --mode=prod

.PHONY: cluster-down
cluster-down:
	@cd local-stack && tilt down --context kind-rtf-mgmt


# docker-compose stack management

.PHONY: orchestrator-shell
orchestrator-shell:
	@docker exec -it orchestrator bash

.PHONY: compose-up
compose-up:
	@docker compose -f docker/docker-compose.yaml up

.PHONY: compose-up-detached
compose-up-detached:
	@docker compose -f docker/docker-compose.yaml up --detach

.PHONY: compose-down
compose-down:
	@docker compose -f docker/docker-compose.yaml down


# Management of the test DB stack

.PHONY: test-db-up
test-db-up:
	@docker compose -f docker/docker-compose-test-db.yaml up

.PHONY: test-db-up-detached
test-db-up-detached:
	@docker compose -f docker/docker-compose-test-db.yaml up --detach

.PHONY: test-db-down
test-db-down:
	@docker compose -f docker/docker-compose-test-db.yaml down


# Management of the test DB container

.PHONY: db-shell
db-shell:
	@docker exec -it db psql -U postgres rtf_rep

.PHONY: clear-db-data
clear-db-data:
	@kubectl exec -n orchestrator --context kind-rtf-mgmt statefulset/postgres -- \
		env PGPASSWORD=password psql -U service_user rtf_rep -c \
		"DELETE FROM test_execution_status; DELETE FROM test_execution; DELETE FROM test_run_status; DELETE FROM test_run;"

.PHONY: compose-clear-db-data
compose-clear-db-data:
	@docker compose -f docker/docker-compose.yaml exec -T db psql -U postgres rtf_rep < docker/db/clear.sql
