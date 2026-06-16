.PHONY: docker-build docker-test docker-lint docker-coverage docker-shell

docker-build:
	docker compose build

docker-test:
	docker compose run --rm test

docker-lint:
	docker compose run --rm lint

docker-coverage:
	docker compose run --rm coverage

docker-shell:
	docker compose run --rm dev bash
