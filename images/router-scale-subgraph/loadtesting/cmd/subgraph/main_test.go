package main

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"

	"github.com/julienschmidt/httprouter"
)

func TestExtraHeaders(t *testing.T) {

	ExtraHeaders = make(map[string]string)
	ExtraHeaders["foo"] = "bar"

	err := loadSchema("testdata/supergraph1.graphql")

	if err != nil {
		t.Fatalf("Cannot load supergraph: %s", err)
	}

	query, err := os.ReadFile("testdata/query1.graphql")

	if err != nil {
		t.Fatalf("Cannot load query: %s", err)
	}

	gql := GraphQLRequest{
		Query: string(query),
	}

	data, err := json.Marshal(gql)

	if err != nil {
		t.Fatalf("Cannot marshal query: %s", err)
	}

	router := httprouter.New()
	router.POST("/", runProcessor)

	req, _ := http.NewRequest("POST", "/", bytes.NewReader(data))
	rr := httptest.NewRecorder()

	router.ServeHTTP(rr, req)
	if status := rr.Code; status != http.StatusOK {
		t.Errorf("Wrong status: %d", status)
	}

	if rr.Header().Get("foo") != "bar" {
		t.Fatalf("Header not present on response.")
	}
}
