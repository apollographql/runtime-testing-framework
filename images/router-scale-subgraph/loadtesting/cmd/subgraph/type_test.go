package main

import (
	"encoding/json"
	"os"
	"strings"
	"testing"

	log "github.com/sirupsen/logrus"
	"github.com/vektah/gqlparser/v2/ast"
	"github.com/vektah/gqlparser/v2/parser"
)

func contains(keyPath string, data map[string]interface{}) bool {

	keyParts := strings.Split(keyPath, ".")

	if len(keyParts) == 1 {
		if _, ok := data[keyParts[0]]; ok {
			return true
		}
	} else {
		if child, ok := data[keyParts[0]]; ok {
			childPath := strings.Join(keyParts[1:], ".")
			switch val := child.(type) {
			case map[string]interface{}:
				return contains(childPath, val)
			case []map[string]interface{}:
				if len(val) > 0 {
					return contains(childPath, val[0])
				}
			case []interface{}:
				if len(val) > 0 {
					switch firstItem := val[0].(type) {
					case map[string]interface{}:
						return contains(childPath, firstItem)
					default:
						log.Errorf("First item is: %T", firstItem)
						panic("Unhandled case.")
					}

				}
			default:
				log.Errorf("Unhandled case in contains: %T", val)
				panic("Unhandled case.")
			}

		}
	}

	return false
}

func TestMain(m *testing.M) {
	log.SetLevel(log.WarnLevel)
	code := m.Run()
	os.Exit(code)
}

func TestInterface(t *testing.T) {

	err := loadSchema("testdata/supergraph1.graphql")
	if err != nil {
		t.Fatalf("Cannot load supergraph: %s", err)
	}

	query, err := os.ReadFile("testdata/query1.graphql")
	if err != nil {
		t.Fatalf("Cannot load query: %s", err)
	}
	req := GraphQLRequest{
		Query: string(query),
	}

	schema := SchemaMutex.Get()
	defer SchemaMutex.Release()

	doc, err := parser.ParseQuery(&ast.Source{Input: req.Query, Name: "spec"})
	if err != nil {
		t.Fatalf("cannot parse query: %s", err.Error())
	}
	fragments := make(map[string]*ast.FragmentDefinition, 0)
	for _, op := range doc.Operations {
		result, err := doCreateResponse(schema, op.Name, op.SelectionSet, req.Variables, string(op.Operation), fragments, 1)
		if err != nil {
			t.Fatalf("Error creating response: %s", err)
		}
		log.Debugln(to_string(result))
		if !contains("customer", result) {
			t.Fatalf("Does not contain customer: %s.", to_string(result))
		}
		if !contains("customer.properties.id", result) {
			t.Fatalf("Does not contain customer.properties.id: %s.", to_string(result))
		}
		if contains("customer.properties.houseCommon.zipCode", result) {
			t.Fatalf("Should not customer.properties.houseCommon.zipCode: %s.", to_string(result))
		}

	}

}

func TestInterfaceEntities(t *testing.T) {

	err := loadSchema("testdata/supergraph1.graphql")
	if err != nil {
		t.Fatalf("Cannot load supergraph: %s", err)
	}

	query, err := os.ReadFile("testdata/query2.graphql")
	if err != nil {
		t.Fatalf("Cannot load query: %s", err)
	}
	req := GraphQLRequest{
		Query: string(query),
	}

	variableData, err := os.ReadFile("testdata/variables2.json")
	if err != nil {
		t.Fatalf("Cannot load variables: %s", err)
	}
	err = json.Unmarshal(variableData, &req.Variables)

	if err != nil {
		t.Fatalf("Cannot parse variables: %s", err)
	}

	schema := SchemaMutex.Get()
	defer SchemaMutex.Release()

	doc, err := parser.ParseQuery(&ast.Source{Input: req.Query, Name: "spec"})
	if err != nil {
		t.Fatalf("cannot parse query: %s", err.Error())
	}
	fragments := make(map[string]*ast.FragmentDefinition, 0)
	for _, op := range doc.Operations {
		result, err := doCreateResponse(schema, op.Name, op.SelectionSet, req.Variables, string(op.Operation), fragments, 1)
		if err != nil {
			t.Fatalf("Error creating response: %s", err)
		}
		log.Debugln(to_string(result))
		if !contains("_entities.properties.salesforceId", result) {
			t.Fatalf("Does not contain _entities.properties.salesforceId: %s.", to_string(result))
		}
		if !contains("_entities.properties.salesforceId", result) {
			t.Fatalf("Does not contain _entities.properties.salesforceId: %s.", to_string(result))
		}

	}

}

func TestInterfaceMerging(t *testing.T) {

	err := loadSchema("testdata/supergraph3.graphql")
	if err != nil {
		t.Fatalf("Cannot load supergraph: %s", err)
	}

	query, err := os.ReadFile("testdata/query3.graphql")
	if err != nil {
		t.Fatalf("Cannot load query: %s", err)
	}
	req := GraphQLRequest{
		Query: string(query),
	}

	variableData, err := os.ReadFile("testdata/variables3.json")
	if err != nil {
		t.Fatalf("Cannot load variables: %s", err)
	}
	err = json.Unmarshal(variableData, &req.Variables)

	if err != nil {
		t.Fatalf("Cannot parse variables: %s", err)
	}

	schema := SchemaMutex.Get()
	defer SchemaMutex.Release()

	doc, err := parser.ParseQuery(&ast.Source{Input: req.Query, Name: "spec"})
	if err != nil {
		t.Fatalf("cannot parse query: %s", err.Error())
	}
	fragments := make(map[string]*ast.FragmentDefinition, 0)
	for _, op := range doc.Operations {
		result, err := doCreateResponse(schema, op.Name, op.SelectionSet, req.Variables, string(op.Operation), fragments, 1)
		if err != nil {
			t.Fatalf("Error creating response: %s", err)
		}
		log.Debugln(to_string(result))
		if !contains("_entities.typeObject.nsid.id698", result) {
			t.Fatalf("Does not contain _entities.typeObject.nsid.id698: %s.", to_string(result))
		}

	}

}
