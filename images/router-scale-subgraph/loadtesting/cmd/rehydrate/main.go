package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"strings"

	gql "github.com/vektah/gqlparser/v2"
	"github.com/vektah/gqlparser/v2/ast"
	"github.com/vektah/gqlparser/v2/formatter"
	"github.com/vektah/gqlparser/v2/parser"
	"github.com/vektah/gqlparser/v2/validator"

	log "github.com/sirupsen/logrus"

	"github.com/joho/godotenv"
)

var Schema *ast.Schema

func parseSchema(schemaData string) (*ast.Schema, error) {

	input := ast.Source{
		Name:    "schema.graphql",
		Input:   schemaData,
		BuiltIn: false,
	}

	ast, err := gql.LoadSchema(&input)
	if err != nil {
		log.Errorf("Could not parse supergraph schema: %s", err.Error())
		return nil, err
	}

	return ast, nil
}

// createJSONQuery - create a JSON representation of the GraphQL operation and write it to disk
// this allows for easy POSTing of this payload to a GQL endpoint.
func createJSONQuery(name, query, outputDir string, metrics *Metrics) error {
	fragments := make(map[string]*ast.FragmentDefinition, 0)

	if name != "test" && outputDir != "" {
		_ = os.MkdirAll(outputDir, 0755)
		if !strings.HasSuffix(outputDir, "/") {
			outputDir = fmt.Sprintf("%s/", outputDir)
		}
	}

	doc, gqlErr := parser.ParseQuery(&ast.Source{Input: string(query), Name: "spec"})
	if gqlErr != nil {
		log.Errorf("cannot parse query: %s", gqlErr.Error())
		return gqlErr
	}

	// create a map of fragment spreads
	for _, frag := range doc.Fragments {
		fragments[frag.Name] = frag
	}

	// ensure all fields that are repeated in a selection have appropriate aliases
	FixAliases(doc, fragments)

	// Find all variables that are used in the query doc
	okVars := FindUsedVariables(Schema, doc, fragments)

	// Remove unused variables to prevent validation errors
	for _, op := range doc.Operations {
		newList := make([]*ast.VariableDefinition, 0, len(okVars))
		for _, vari := range op.VariableDefinitions {
			if val, ok := okVars[vari.Variable]; ok {
				if strings.HasSuffix(val, "!") {
					vari.Type.NonNull = true
				}
				newList = append(newList, vari)

			}
			vari.DefaultValue = nil
		}
		op.VariableDefinitions = nil
		op.VariableDefinitions = newList
	}

	subgraphs := make(map[string]bool)

	// Fill in variable values
	vars := CreateVariableValues(doc, okVars)
	selCount := FillMissingArguments(doc, vars, fragments, subgraphs)

	if metrics != nil {
		metrics.NumFragments = len(doc.Fragments)
		metrics.TotalSelections = selCount
		metrics.SubgraphsReferenced = len(subgraphs)
		f, err := os.OpenFile(fmt.Sprintf("%s%s.metrics.json", outputDir, name), os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0644)

		if err != nil {
			log.Fatal(err)
		}

		json.NewEncoder(f).Encode(metrics)
	}

	stringOut := strings.Builder{}
	format := formatter.NewFormatter(&stringOut, formatter.WithIndent(" "))

	format.FormatQueryDocument(doc)

	// ensure operation validates against the schema
	errList := validator.Validate(Schema, doc)
	validateError := false
	validateErrorReason := ""

	if errList != nil {
		validateError = true
		validateErrorReason = errList.Error()

		log.Errorf("Cannot validate query: %s", name)
		log.Errorf(validateErrorReason)
	}

	queryJSON := GraphQLRequest{
		Query:     stringOut.String(),
		Variables: vars,
	}

	queryDetails := strings.Builder{}
	graphRef := os.Getenv("APOLLO_GRAPH_REF")
	if validateError {
		queryDetails.WriteString(validateErrorReason)
		queryDetails.WriteString("\n\n")
	}

	queryDetails.WriteString(fmt.Sprintf("Supergraph is: %s\n", graphRef))
	queryDetails.WriteString(fmt.Sprintf("Variables: %v\n\n", vars))
	queryDetails.WriteString(stringOut.String())

	if validateError && name == "test" {
		//fmt.Println(errorDetails.String())
		return fmt.Errorf(queryDetails.String())
	}

	if name == "test" {
		return nil
	}

	if validateError {
		f2, err := os.OpenFile(fmt.Sprintf("%s%s.error.txt", outputDir, name), os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0644)
		if err != nil {
			log.Error(err)
		}
		f2.WriteString(queryDetails.String())
		os.Exit(1)
	} else {
		f2, err := os.OpenFile(fmt.Sprintf("%s%s.txt", outputDir, name), os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0644)
		if err != nil {
			log.Error(err)
		}
		f2.WriteString(queryDetails.String())

	}

	f, err := os.OpenFile(fmt.Sprintf("%s%s.json", outputDir, name), os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0644)

	if err != nil {
		log.Fatal(err)
	}

	json.NewEncoder(f).Encode(queryJSON)

	return nil
}

func setLogLevel(level string) {
	switch level {
	case "trace":
		log.SetLevel(log.TraceLevel)
	case "debug":
		log.SetLevel(log.DebugLevel)
	case "info":
		log.SetLevel(log.InfoLevel)
	case "warn":
		log.SetLevel(log.WarnLevel)
	case "error":
		fallthrough
	default:
		log.SetLevel(log.ErrorLevel)
	}
}

func main() {
	godotenv.Load()

	var logLevel = flag.String("log", "info", "set the log level to 'trace', 'debug', 'info', 'warn', or 'error''")
	var writeMetrics = flag.Bool("metrics", false, "write out query metrics")
	var queryID = flag.String("query", "", "fetch a single query by ID")
	var superGraphFile = flag.String("supergraph", "", "read supergraph from local filesystem")
	var localQuery = flag.String("local", "", "local query file to parse")
	var queryDir = flag.String("out-dir", "queries", "directory to store downloaded/parsed queries in")

	flag.Parse()

	setLogLevel(*logLevel)

	apiKey := os.Getenv("APOLLO_KEY")
	graphRef := os.Getenv("APOLLO_GRAPH_REF")
	graphRefParts := strings.Split(graphRef, "@")

	if *queryID != "" {
		fmt.Printf("Fetching query %s from %s\n", *queryID, graphRef)
		err := FetchSupergraphSchema(*superGraphFile)
		if err != nil {
			fmt.Printf("Cannot fetch supergraph: %s", err)
			os.Exit(1)
		}
		sig, err := fetchOpSignature(graphRefParts[0], *queryID, apiKey)
		if err != nil {
			fmt.Printf("Cannot fetch op signature: %s", err)
			os.Exit(1)
		}
		createJSONQuery(*queryID, sig, *queryDir, nil)
		os.Exit(0)
	}

	if *localQuery != "" {
		log.Printf("Transforming single query: %s", *localQuery)

		FetchSupergraphSchema(*superGraphFile)
		fileContents, err := os.ReadFile(*localQuery)

		if err != nil {
			fmt.Printf("Cannot read local query file: %s", err)
			os.Exit(1)
		}
		createJSONQuery(*localQuery, string(fileContents), "", nil)
	} else {
		log.Printf("Fetching supergraph and top graph queries...")
		FetchSupergraphSchema(*superGraphFile)
		FetchQueries(*writeMetrics, *queryDir)
	}

}
func init() {
	// Log as JSON instead of the default ASCII formatter.
	//log.SetFormatter(&log.JSONFormatter{})
	log.SetFormatter(&log.TextFormatter{})

	// Output to stdout instead of the default stderr
	// Can be any io.Writer, see below for File example
	log.SetOutput(os.Stderr)

	// Only log the warning severity or above.
	log.SetLevel(log.WarnLevel)
}
