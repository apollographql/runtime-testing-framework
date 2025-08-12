include ../../.env

transform:
	go build

run: transform
	APOLLO_GRAPH_REF=${APOLLO_GRAPH_REF} APOLLO_KEY=${APOLLO_KEY} ./transform
