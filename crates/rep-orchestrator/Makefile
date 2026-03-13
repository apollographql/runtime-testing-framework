# Test actions

.PHONY: db-tests
db-tests:
	@RTF_DB_HOST=localhost \
		RTF_DB_PORT=5432 \
		RTF_DB_NAME=rtf_rep \
		RTF_DB_USER=service_user \
		RTF_DB_PASS=password \
		cargo test --features db_tests $(ARGS)


# CI actions

.PHONY: ci-db-tests
ci-db-tests: test-db-up-detached db-tests test-db-down


# Management of the test DB stack

.PHONY: test-db-up
test-db-up:
	@docker compose -f docker-compose-test-db.yaml up

.PHONY: test-db-up-detached
test-db-up-detached:
	@docker compose -f docker-compose-test-db.yaml up --detach

.PHONY: test-db-down
test-db-down:
	@docker compose -f docker-compose-test-db.yaml down

.PHONY: db-shell
db-shell:
	@docker exec -it db psql -U postgres rtf_rep

.PHONY: clear-db-data
clear-db-data:
	@docker compose exec -T db psql -U postgres rtf_rep < docker/db/clear.sql
