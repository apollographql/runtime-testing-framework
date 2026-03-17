# Test actions

.PHONY: db-tests
db-tests:
	@RTF_DB_HOST=localhost \
		RTF_DB_PORT=5432 \
		RTF_DB_NAME=rtf_rep \
		RTF_DB_USER=service_user \
		RTF_DB_PASS=password \
		cargo test --features db_tests $(ARGS)

.PHONY: integration-tests
integration-tests: clear-db-data
	@RTF_DB_HOST=localhost \
		RTF_DB_PORT=5432 \
		RTF_DB_NAME=rtf_rep \
		RTF_DB_USER=service_user \
		RTF_DB_PASS=password \
		cargo test --test '*' $(ARGS)

.PHONY: test-all
test-all: db-tests integration-tests


# CI actions

.PHONY: ci-db-tests
ci-db-tests: test-db-up-detached db-tests test-db-down

.PHONY: ci-tests
ci-tests: up-detached wait-for-server test-all down

.PHONY: wait-for-server
wait-for-server:
	@echo "Waiting for server to start..."
	@until docker compose -f docker/docker-compose.yaml ps | grep -E '^orchestrator\s' | grep healthy; do \
		echo "..."; \
		sleep 1; \
	done


# Management of the integration test stack

.PHONY: orchestrator-shell
orchestrator-shell:
	@docker exec -it orchestrator bash

.PHONY: up
up:
	@docker compose -f docker/docker-compose.yaml up

.PHONY: up-detached
up-detached:
	@docker compose -f docker/docker-compose.yaml up --detach

.PHONY: down
down:
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
	@docker compose -f docker/docker-compose.yaml exec -T db psql -U postgres rtf_rep < docker/db/clear.sql
