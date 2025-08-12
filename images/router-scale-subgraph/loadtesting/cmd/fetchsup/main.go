package main

import (
	"flag"
	"fmt"
	"html/template"
	"os"
	"strings"

	log "github.com/sirupsen/logrus"

	"github.com/apollosolutions/loadtesting/pkg/client"
	"github.com/joho/godotenv"
)

var ROUTING_URL string = "http://35.196.241.71:80"

var SUB_SCHEMA string = `
type SubscriptionResponse @join__type(graph: SCHEMA_SERVICE) {
	id: ID!
	name: String
	sequence: Int
  }
  
  type Subscription @join__type(graph: SCHEMA_SERVICE) {
  
	mySub: SubscriptionResponse @join__field(graph: SCHEMA_SERVICE)
  }
extend schema {
	subscription: Subscription
}
`

var ROUTER_CONF string = `
supergraph:
  listen: 0.0.0.0:${env.PORT}
subscription:
  max_opened_subscriptions: 20000
  enabled: true
  mode:
    callback:
      public_url: "http://${env.NODE_IP}:${env.CALLBACK_PORT}/callback" 
      listen: 0.0.0.0:${env.CALLBACK_PORT}
      path: /callback
      heartbeat_interval: 5s 
      subgraphs: 
{{ range . }}        - {{.SubgraphName}}
{{ end -}}
include_subgraph_errors:
  all: true
override_subgraph_url:
{{ range . }}  {{.SubgraphName}}: {{.SubgraphRoute}}
{{ end -}}
`

func writeConfig(filename string, subgraphs []client.Subgraph) {

	file, fileErr := os.OpenFile(filename, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0644)
	if fileErr != nil {
		log.Errorf("Cannot open log file: %s", fileErr)
		os.Exit(1)
	}

	defer file.Close()

	type Sub struct {
		SubgraphName  string
		SubgraphRoute string
	}
	subs := make([]Sub, len(subgraphs))
	for i, s := range subgraphs {
		subs[i] = Sub{
			SubgraphName:  s.Name,
			SubgraphRoute: ROUTING_URL,
		}
	}

	tmpl, err := template.New("conf").Parse(ROUTER_CONF)
	if err != nil {
		log.Errorf("Error creating template: %s", err.Error())
		return
	}
	tmpl.Execute(file, subs)

}

func writeSchemas(subgraphs []client.Subgraph, outputDir string) {

	if outputDir != "" {
		_ = os.MkdirAll(outputDir, 0755)
		if !strings.HasSuffix(outputDir, "/") {
			outputDir = fmt.Sprintf("%s/", outputDir)
		}
	}

	for _, subgraph := range subgraphs {
		log.Infof("Writing subgraph: %s", subgraph.Name)
		file, fileErr := os.OpenFile(fmt.Sprintf("%s%s.graphql", outputDir, subgraph.Name), os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0644)
		if fileErr != nil {
			log.Errorf("Cannot open log file: %s", fileErr)
			os.Exit(1)
		}
		file.WriteString(subgraph.ActivePartialSchema.SDL)
		file.Close()
	}

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

	var configFile = flag.String("config", "config.yaml", "set the config file name to write")
	var superGraphFile = flag.String("supergraph", "supergraph.graphql", "where to write the supergraph")
	var logLevel = flag.String("log", "info", "set the log level to 'trace', 'debug', 'info', 'warn', or 'error''")
	var useTemplate = flag.String("template", "", "use an external template for config rendering (go templates, variables in array are 'SubgraphName' and 'SubgraphRoute')")
	var outputDir = flag.String("out-dir", "subgraphs", "directory to store downloaded subgraphs in")

	flag.Parse()

	setLogLevel(*logLevel)

	apiKey := os.Getenv("APOLLO_KEY")
	if apiKey == "" {
		log.Error("APOLLO_KEY must be defined in the environment or .env")
		os.Exit(1)
	}
	graphRef := os.Getenv("APOLLO_GRAPH_REF")
	if graphRef == "" {
		log.Error("APOLLO_GRAPH_REF must be defined in the environment or .env")
		os.Exit(1)
	}
	graphRefParts := strings.Split(graphRef, "@")

	routing := os.Getenv("ROUTING_URL")
	if routing != "" {
		ROUTING_URL = routing
	}

	if *useTemplate != "" {
		log.Infof("Using external config template: %s", *useTemplate)
		data, err := os.ReadFile(*useTemplate)
		if err != nil {
			log.Errorf("Cannot read external template file: %s", err.Error())
			os.Exit(1)
		}
		ROUTER_CONF = string(data)
	}

	log.Infof("Downloading supergraph & subgraphs: |%s|", graphRef)

	res, err := client.RunOp[client.SupergraphResult]("FetchSDL", client.SupergraphQuery, apiKey, map[string]interface{}{
		"graph_id": graphRefParts[0],
		"variant":  graphRefParts[1],
	})

	if err != nil {
		log.Error(err.Error())
		os.Exit(1)
	}

	if len(res.Errors) > 0 {
		log.Error("Errors found in response...")
		for _, e := range res.Errors {
			log.Error(e.Message)
		}
		os.Exit(1)
	}
	if len(res.Extensions) > 0 {
		log.Warn("Extensions found in response...")
		for k, v := range res.Extensions {
			log.Warnf("%s %+v", k, v)
		}
		os.Exit(1)
	}

	schemaSDL := res.Data.Service.Variant.LatestApprovedLaunch.Build.Result.CoreSchema.CoreDocument
	if res.Data.Service.Variant.SourceVariant != nil {
		// This is a contract
		if schemaSDL == "" {
			log.Errorf("Supergraph content is empty.")
			os.Exit(1)
		} else {
			log.Infof("This variant is a contract, using fallback SDL location...")
			writeConfig(*configFile, res.Data.Service.Variant.SourceVariant.Subgraphs)
			writeSchemas(res.Data.Service.Variant.SourceVariant.Subgraphs, *outputDir)
		}

	} else {
		writeConfig(*configFile, res.Data.Service.Variant.Subgraphs)
		writeSchemas(res.Data.Service.Variant.Subgraphs, *outputDir)
	}

	file, errs := os.Create(*superGraphFile)
	if errs != nil {
		log.Error("Failed to create file:", errs)
		os.Exit(1)
	}
	defer file.Close()

	// Write the string "Hello, World!" to the file
	_, errs = file.WriteString(schemaSDL)
	if errs != nil {
		log.Error("Failed to write to file:", errs) //print the failed message
		os.Exit(1)
	}

}
