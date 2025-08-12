package main

import (
	"os"
	"testing"
)

func readFile(filename string, t *testing.T) string {
	data, err := os.ReadFile(filename)
	if err != nil {
		t.Fatalf("Cannot read schema test file: %s", err.Error())
	}
	return string(data)
}

func TestInputs(t *testing.T) {

	ast, err := parseSchema(readFile("testdata/schema1.graphql", t))

	if err != nil {
		t.Fatalf("Cannot parse schema: %s", err.Error())
	}
	Schema = ast
	err = createJSONQuery("test", readFile("testdata/query1.graphql", t), "queries", nil)

	if err != nil {
		t.Fatalf("Cannot validate query: %s", err.Error())
	}
}

func TestAliases(t *testing.T) {

	ast, err := parseSchema(readFile("testdata/schema2.graphql", t))

	if err != nil {
		t.Fatalf("Cannot parse schema: %s", err.Error())
	}
	Schema = ast
	err = createJSONQuery("test", readFile("testdata/query2.graphql", t), "queries", nil)

	if err != nil {
		t.Fatalf("Cannot validate query: %s", err.Error())
	}
}
