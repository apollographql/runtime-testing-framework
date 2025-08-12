package main

import (
	"fmt"
	"os"
	"strings"

	"github.com/apollosolutions/loadtesting/pkg/client"
	log "github.com/sirupsen/logrus"
	"github.com/vektah/gqlparser/v2/ast"
	"github.com/vektah/gqlparser/v2/formatter"
	"github.com/vektah/gqlparser/v2/parser"
)

type GraphQLRequest struct {
	Query         string                 `json:"query"`
	OperationName string                 `json:"operationName,omitempty"`
	Variables     map[string]interface{} `json:"variables,omitempty"`
}

type Metrics struct {
	ReqPerMin           float64 `json:"requests_per_minute"`
	NumFragments        int     `json:"num_fragments"`
	TotalSelections     int     `json:"total_selections"`
	SubgraphsReferenced int     `json:"subgraphs_referenced"`
}

// FetchSupergraphSchema - fetch the supergraph for an env defined Graph Ref
func FetchSupergraphSchema(filename string) error {

	apiKey := os.Getenv("APOLLO_KEY")
	graphRef := os.Getenv("APOLLO_GRAPH_REF")
	graphRefParts := strings.Split(graphRef, "@")

	schema := ""

	if filename == "" {
		log.Infof("Fetching supergraph schema '%s'...", graphRef)
		res, err := client.RunOp[client.SupergraphResult]("FetchSDL", client.SupergraphQuery, apiKey, map[string]interface{}{
			"graph_id": graphRefParts[0],
			"variant":  graphRefParts[1],
		})

		if err != nil {
			log.Errorf("Could not fetch supergraph schema: %s", err.Error())
			return err
		}
		schema = res.Data.Service.Variant.LatestApprovedLaunch.Build.Result.CoreSchema.CoreDocument
	} else {
		log.Infof("Reading local supergraph '%s'...", filename)
		fileData, err := os.ReadFile(filename)
		if err != nil {
			log.Errorf("Could not read supergraph schema: %s", err.Error())
			return err
		}
		schema = string(fileData)
	}

	ast, err := parseSchema(schema)
	if err != nil {
		return err
	}
	Schema = ast
	return nil
}

func fetchOpSignature(graphID, queryID, apiKey string) (string, error) {

	res, err := client.RunOp[client.SupergraphResult]("GetOperationSignature", client.GetOperationSignature, apiKey, map[string]interface{}{
		"serviceId":   graphID,
		"operationId": queryID,
	})
	if err != nil {
		log.Errorf("Cannot pull op signature from API: %s", err.Error())
		return "", fmt.Errorf("cannot pull op signature from API: %s", err.Error())
	}
	return res.Data.Service.Operation.Signature, nil
}

// FetchQueries - fetch top queries for an env defined Graph Ref
func FetchQueries(writeMetrics bool, outputDir string) {

	apiKey := os.Getenv("APOLLO_KEY")
	graphRef := os.Getenv("APOLLO_GRAPH_REF")
	graphRefParts := strings.Split(graphRef, "@")

	log.Infof("Fetching top queries for '%s'...", graphRef)

	res, err := client.RunOp[client.SupergraphResult]("FetchQueryIDs", client.FetchQueryIDs, apiKey, map[string]interface{}{
		"serviceId": graphRefParts[0],
		"name":      graphRefParts[1],
		"from":      "-86400",
		"to":        "-0",
		"filter": map[string]interface{}{
			"or": []string{},
		},
		"orderBy": map[string]interface{}{
			"column":    "REQUEST_COUNT",
			"direction": "DESCENDING",
		},
		"first": 20,
	})

	if err != nil {
		log.Errorf("Cannot pull query list from API: %s", err.Error())
		return
	}

	if outputDir != "" {
		_ = os.MkdirAll(outputDir, 0755)
		if !strings.HasSuffix(outputDir, "/") {
			outputDir = fmt.Sprintf("%s/", outputDir)
		}
	}

	for _, query := range res.Data.Service.Variant.ByRequests.Nodes {
		if query.DisplayName[0] == '#' {
			continue
		}
		if query.Type == "MUTATION" {
			continue
		}

		log.Infof("Found query %s (%f req/min)", query.DisplayName, query.RequestCountPerMin)

		signature, err := fetchOpSignature(graphRefParts[0], query.ID, apiKey)
		if err != nil {
			log.Errorf("Cannot pull op signature from API: %s", err.Error())
			return
		}
		os.WriteFile(fmt.Sprintf("%s%s.raw.graphql", outputDir, query.ID), []byte(signature), 0644)

		metrics := &Metrics{
			ReqPerMin: query.RequestCountPerMin,
		}

		doc, err := parser.ParseQuery(&ast.Source{Input: signature, Name: "spec"})
		if err != nil {
			log.Errorf("cannot parse query: %s", err.Error())
			return
		}

		stringOut := strings.Builder{}
		format := formatter.NewFormatter(&stringOut, formatter.WithIndent(" "))

		format.FormatQueryDocument(doc)

		file, fileErr := os.OpenFile(fmt.Sprintf("%s%s.graphql", outputDir, query.ID), os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0644)
		if fileErr != nil {
			log.Errorf("Cannot write query to disk: %s", fileErr.Error())
			return
		}
		file.WriteString(stringOut.String())
		file.Close()

		if writeMetrics {
			createJSONQuery(query.ID, signature, outputDir, metrics)
		} else {
			createJSONQuery(query.ID, signature, outputDir, nil)
		}
	}

}
