server-test:
	go build && ./subgraph --schema testdata/supergraph1.graphql --header foo=bar --header name=joe

client-test:
	curl 'http://localhost:8080' -v \
		-H 'accept: application/json' \
		-H 'content-type: application/json' \
		--data-binary @testdata/query1.graphql.json