package main

import (
	"bytes"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"math"
	"net/http"
	"os"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/apollosolutions/loadtesting/pkg/latency"
	"github.com/apollosolutions/loadtesting/pkg/tools"
	"github.com/fsnotify/fsnotify"
	"github.com/julienschmidt/httprouter"
	log "github.com/sirupsen/logrus"

	gql "github.com/vektah/gqlparser/v2"
	"github.com/vektah/gqlparser/v2/ast"
	"github.com/vektah/gqlparser/v2/formatter"
	"github.com/vektah/gqlparser/v2/parser"

	"github.com/joho/godotenv"
)

var SchemaMutex SchemaData

type GraphQLRequest struct {
	Query         string                 `json:"query"`
	OperationName string                 `json:"operationName,omitempty"`
	Variables     map[string]interface{} `json:"variables,omitempty"`
	Extensions    map[string]interface{} `json:"extensions,omitempty"`
}

const MAX_DEPTH = 4

var LatencyGenerator latency.SimpleLatencyGenerator
var SUBSCRIPTION_FREQUENCY int64 = 1 // 1 subscription event per second

var SubscriptionDB *sync.Map
var ExtraHeaders map[string]string
var RunValidator bool = false

var Fed2Regex = regexp.MustCompile(`(?m)extend[\n\r\s]+schema[\n\r\s]+@link`)

func mergeInterface(data1, data2 interface{}) interface{} {
	switch d1 := data1.(type) {
	case map[string]interface{}:
		result := make(map[string]interface{}, len(d1))
		d2 := data2.(map[string]interface{})
		for k, v := range d1 {
			// for all data in d1, if d2 has the same key, merge those values
			if val, ok := d2[k]; ok {
				result[k] = mergeInterface(v, val)
			} else {
				// otherwise just copy data to result
				result[k] = v
			}
		}
		// Finally copy all data in d2 into result as well
		for k, v := range d2 {
			if _, ok := d1[k]; ok {
				// Data already exists and we have already merged this data
				continue
			} else {
				// otherwise just copy data to result
				result[k] = v
			}
		}

		return result
	case []interface{}:
		d2 := data2.([]interface{})
		result := make([]interface{}, int(math.Max(float64(len(d1)), float64(len(d2)))))
		for i, val := range d1 {
			if len(d2) < (i + 1) {
				log.Warnf("In merge, array lengths do not match")
				result[i] = val
				continue // this is kinda bad
			} else {
				result[i] = mergeInterface(d1[i], d2[i])
			}
		}
		return result

	default:
		log.Debugf("Not merging type %T, setting to %v", d1, d1)
		return d1
	}

}

func getTypeConditions(schema *ast.Schema, set ast.SelectionSet, fragments map[string]*ast.FragmentDefinition) []string {

	types := make([]string, 0)
	for _, field := range set {
		switch val := field.(type) {
		case *ast.Field:
		case *ast.InlineFragment:
			types = append(types, val.TypeCondition)
		case *ast.FragmentSpread:
			frag := fragments[val.Name]
			if frag == nil {
				log.Errorf("getTypeConditions: fragment is nil: %s", val.Name)
			} else {
				t := schema.Types[frag.TypeCondition]
				if t.IsAbstractType() {
					innerTypes := getTypeConditions(schema, frag.SelectionSet, fragments)
					types = append(types, innerTypes...)
				}
			}

		}
	}
	return types
}

func doCreateResponse(schema *ast.Schema, opName string, set ast.SelectionSet, variables map[string]interface{},
	parentTypeName string, fragments map[string]*ast.FragmentDefinition, depth int) (map[string]interface{}, error) {

	log.Debugf("doCreateResponse %s %d", parentTypeName, depth)

	results := make(map[string]interface{}, len(set))
	var parentType *ast.Definition

	switch parentTypeName {
	case "query":
		parentType = schema.Query
		// TODO: this is a weird edge case.  A fragment spread can use the root operation type
		// as a condition. However, the root type can be any type per the spec and I
		// don't have a way to get that type name from the parser AFAIK.
		parentTypeName = "Query"
	case "mutation":
		parentType = schema.Mutation
		parentTypeName = "Mutation"
	case "subscription":
		parentType = schema.Subscription
		parentTypeName = "Subscription"
	default:
		parentType = schema.Types[parentTypeName]
		switch parentType.Kind {
		case ast.Union:
			// We should pick a concrete type to use based on what's being queried.
			// 1. scan through all fragments and collect type conditions
			// 2. if no type conditions found, just pick one

			types := getTypeConditions(schema, set, fragments)
			unionTypes := parentType.Types

			log.Debugf("doCreateResponse %s %d UNION FOUND=%s", parentTypeName, depth, parentType.Name)

			for _, typeName := range types {
				found := false
				for _, unionType := range unionTypes {
					if typeName == unionType {
						found = true
					}
				}
				if !found {
					log.Errorf("Type condition not found in possible union types: %s (union: %s)", typeName, parentTypeName)
				}
			}

			if (len(types) == 0) || (len(types) == 1 && types[0] == parentTypeName) {
				// No types found, pick one

				if len(types) > 0 {
					parentTypeName = unionTypes[0]
					parentType = schema.Types[parentTypeName]
					log.Debugf("doCreateResponse Interface detected, changing type from %s to %s", parentTypeName, parentType.Name)
					parentTypeName = parentType.Name
				}
			} else if len(types) > 0 {
				parentTypeName = types[0]
				// TODO: check if type in uniontypes
				parentType = schema.Types[parentTypeName]
			} else {
				log.Fatalf("No suitable union type found.")
			}
		case ast.Interface:
			// We should pick a concrete type to use based on what's being queried.
			// 1. scan through all fragments and collect type conditions
			// 2. if no type conditions found, just pick one
			log.Debugf("doCreateResponse %s %d INTERFACE FOUND=%s", parentTypeName, depth, parentType.Name)

			types := getTypeConditions(schema, set, fragments)
			possibleTypes := schema.GetPossibleTypes(parentType)
			for _, typeName := range types {
				found := false
				for _, ifaceType := range possibleTypes {
					if typeName == ifaceType.Name {
						found = true
					}
				}
				if !found {
					log.Errorf("Type condition not found in possible iface types: %s (union: %s)", typeName, parentTypeName)
				}
			}

			if (len(types) == 0) || (len(types) == 1 && types[0] == parentTypeName) {
				// No types found, pick one

				if len(possibleTypes) > 0 {
					parentType = possibleTypes[0]
					log.Debugf("doCreateResponse Interface detected, changing type from %s to %s", parentTypeName, parentType.Name)
					parentTypeName = parentType.Name
				}
			} else if len(types) > 0 {
				parentTypeName = types[0]
				// TODO: check if type in possibleTypes
				parentType = schema.Types[parentTypeName]
			}
		}
	}

	for _, selection := range set {
		switch field := selection.(type) {
		case *ast.Field:
			log.Tracef("doCreateResponse %s.%s:%s", parentTypeName, field.Name, field.Alias)

			if field.Name == "_entities" {
				log.Debugf("Entities query...")

				reps := variables["representations"].([]interface{})
				entities := make([]map[string]interface{}, 0)

				for _, repIface := range reps {
					rep, ok := repIface.(map[string]interface{})
					if !ok {
						log.Errorf("Cannot cast rep type is: %T", repIface)
						continue
					}
					response, err := doCreateResponse(schema, opName, field.SelectionSet, nil,
						rep["__typename"].(string), fragments, depth+1)
					if err != nil {
						log.Errorf("Cannot create entity response: %s", err)
						continue
					}
					entities = append(entities, response)
				}

				results["_entities"] = entities
				continue
			}

			// typename is easy, set it
			if field.Name == "__typename" {
				results["__typename"] = parentTypeName
				log.Infof("%s%s%s=%s", strings.Repeat(" ", depth*2), tools.Op(&opName), field.Name, parentTypeName)
				continue
			}

			// determine real schema type of field
			fieldType, isArray, isNullable, err := tools.FindFieldType(field.Name, parentTypeName, schema)

			if err != nil {
				log.Errorf("Cannot find field type: %s %s", field.Name, parentTypeName)
				return nil, err
			}

			log.Infof("%s%s%s - %s      []:%t !:%t", strings.Repeat(" ", depth*2), tools.Op(&opName), field.Name, fieldType, isArray, isNullable)

			// create loop counter for arrays
			// TODO: this was disabled for testing, might be worth re-enabling random array lengths
			// loop := rand.IntN(10) + 3
			loop := 2
			if !isArray {
				loop = 1
			}

			data := make([]interface{}, loop)

			// fill in data
			for i := 0; i < loop; i++ {

				if len(field.SelectionSet) > 0 {
					fieldData, err := doCreateResponse(schema, opName, field.SelectionSet, nil, fieldType, fragments, depth+1)
					if err != nil {
						return nil, err
					}
					//data[field.Alias] = fieldData
					data[i] = fieldData
				} else {
					// If there is no selection set then this is a single value
					if tools.CheckScalar(fieldType) {
						log.Tracef("IS SCALAR %s %s", field.Name, fieldType)

						data[i] = tools.ScalarToFakeData(fieldType)
					} else {
						t := schema.Types[fieldType]
						switch t.Kind {
						case ast.Enum:
							log.Tracef("IS ENUM %s %s", field.Name, fieldType)
							data[i] = t.EnumValues[0].Name
						case ast.Scalar:
							log.Tracef("IS CUSTOM SCALAR %s %s", field.Name, fieldType)
							data[i] = tools.ScalarToFakeData("String")
						default:
							log.Errorf("Unhandled type in get value: %s %s %s", field.Name, fieldType, t.Kind)
							panic("How is this happen?")
						}
					}
				}
			}

			if isArray {
				if _, ok := results[field.Alias]; ok {
					log.Debugf("%s already exists on parent %s", field.Alias, parentTypeName)
					results[field.Alias] = mergeInterface(results[field.Alias], data)
				} else {
					results[field.Alias] = data
				}

			} else {
				if _, ok := results[field.Alias]; ok {
					log.Debugf("%s already exists on parent %s", field.Alias, parentTypeName)
					results[field.Alias] = mergeInterface(results[field.Alias], data[0])
				} else {
					results[field.Alias] = data[0]
				}

			}

		case *ast.InlineFragment:
			log.Infof("%s%sifrag on %s", strings.Repeat(" ", depth*2), tools.Op(&opName), field.TypeCondition)
			if field.TypeCondition != parentTypeName {
				log.Debugf("Type condition does not match for ifrag: %s != %s", field.TypeCondition, parentTypeName)
				continue
			}
			if len(field.SelectionSet) > 0 {
				fieldData, err := doCreateResponse(schema, opName, field.SelectionSet, nil, field.TypeCondition, fragments, depth+1)
				if err != nil {
					return nil, err
				}
				for k, v := range fieldData {
					if _, ok := results[k]; ok {
						log.Debugf("%s already exists on parent %s", k, parentTypeName)
						results[k] = mergeInterface(results[k], v)
					} else {
						results[k] = v
					}

				}
			}

		case *ast.FragmentSpread:
			frag := fragments[field.Name]
			if frag != nil {
				log.Infof("%s%sspread %s on %s", strings.Repeat(" ", depth*2), tools.Op(&opName), field.Name, frag.TypeCondition)
				fieldData, err := doCreateResponse(schema, opName, frag.SelectionSet, nil, frag.TypeCondition, fragments, depth+1)
				if err != nil {
					return nil, err
				}
				for k, v := range fieldData {
					if _, ok := results[k]; ok {
						log.Debugf("%s already exists on parent %s", k, parentTypeName)
						results[k] = mergeInterface(results[k], v)
					} else {
						results[k] = v
					}

				}
			} else {
				log.Errorf("Fragment spread cannot be found: %s", field.Name)
			}
		default:
			log.Errorf("AST type of %T not supported", field)
		}
	}
	return results, nil
}

func runSubscription(req GraphQLRequest, op *ast.OperationDefinition, fragments map[string]*ast.FragmentDefinition, done chan bool) {
	subData := req.Extensions["subscription"].(map[string]interface{})
	subId := subData["subscriptionId"].(string)
	verifier := subData["verifier"].(string)
	callback := subData["callbackUrl"].(string)
	heartbeatMs := subData["heartbeatIntervalMs"].(float64)

	heartbeat := time.Tick(time.Duration(heartbeatMs/2) * time.Millisecond)
	databeat := time.Tick(time.Duration(SUBSCRIPTION_FREQUENCY) * time.Second)

	for {
		select {
		case <-done:
			log.Debugf("Subscription asked to close.")
			return
		case <-heartbeat:
			log.Debugf("Sending check to router for sub %s", subId)
			err := sendSubscriptionCheck(subId, verifier, callback)
			if err != nil {
				log.Errorf("Subscription callback failed: %s", subId)
			}
		case <-databeat:
			schema := SchemaMutex.TryGet()
			if schema == nil {
				log.Warnf("Shutting down subscription due to schema update: %s", subId)
				done <- true
				return
			}
			result, err := doCreateResponse(schema, req.OperationName, op.SelectionSet, req.Variables, string(op.Operation), fragments, 1)
			SchemaMutex.Release()

			if err != nil {
				log.Errorf("error creating subscription payload: %s", err)
			} else {
				log.Debugf("Sending data to router for sub %s", subId)
				err = sendSubscriptionData(subId, verifier, callback, result)
				if err != nil {
					log.Errorf("error sending data to router: %s", err)
					done <- true
					return
				}
			}
		}
	}
}

func sendSubscriptionData(id, verifier, address string, payload map[string]interface{}) error {
	postData := map[string]interface{}{
		"kind":     "subscription",
		"action":   "next",
		"id":       id,
		"verifier": verifier,
		"payload":  map[string]interface{}{"data": payload},
	}
	marshalled, _ := json.Marshal(postData)

	log.Debug(string(marshalled))

	req, _ := http.NewRequest("POST", address, bytes.NewReader(marshalled))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("subscription-protocol", "callback/1.0")

	client := http.Client{Timeout: 3 * time.Second}

	res, err := client.Do(req)
	if err != nil {
		log.Errorf("Could not send check to router: %s", err)
		return err
	}
	if res.StatusCode > 204 {
		body, _ := io.ReadAll(res.Body)
		defer res.Body.Close()
		log.Warnf("Non 204 status code from router in next: %d: %s", res.StatusCode, string(body))
		return fmt.Errorf("non 204 status code from router in next: %d: %s", res.StatusCode, string(body))
	}

	return nil
}

func sendSubscriptionCheck(id, verifier, address string) error {

	postData := map[string]string{
		"kind":     "subscription",
		"action":   "check",
		"id":       id,
		"verifier": verifier,
	}
	marshalled, _ := json.Marshal(postData)
	req, _ := http.NewRequest("POST", address, bytes.NewReader(marshalled))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("subscription-protocol", "callback/1.0")

	client := http.Client{Timeout: 3 * time.Second}

	res, err := client.Do(req)
	if err != nil {
		log.Errorf("Could not send check to router: %s", err)
		return err
	}
	if res.StatusCode != 204 {
		body, _ := io.ReadAll(res.Body)
		defer res.Body.Close()
		log.Warnf("Non 204 status code from router in check: %d: %s", res.StatusCode, string(body))
		return fmt.Errorf("non 204 status code from router in check: %d: %s", res.StatusCode, string(body))
	}

	return nil
}

func runProcessor(w http.ResponseWriter, r *http.Request, _ httprouter.Params) {

	var req GraphQLRequest
	var result map[string]interface{}
	fragments := make(map[string]*ast.FragmentDefinition, 0)

	// Try to decode the request body into the struct. If there is an error,
	// respond to the client with the error message and a 400 status code.
	defer r.Body.Close()

	body, _ := io.ReadAll(r.Body)

	err := json.Unmarshal(body, &req)
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	log.Debugf("New query: %s", req.Query)
	log.Tracef("Variables: %s", to_string(req.Variables))

	doc, err := parser.ParseQueryWithTokenLimit(&ast.Source{Input: req.Query, Name: "spec"}, 150000)
	if err != nil {
		log.Errorf("cannot parse query: %s", err.Error())
		http.Error(w, fmt.Sprintf("Invalid GraphQL operation: %s", err.Error()), http.StatusBadRequest)
		return
	}
	// collect all fragments and pass to response generator
	for _, frag := range doc.Fragments {
		fragments[frag.Name] = frag
	}

	schema := SchemaMutex.Get()
	defer SchemaMutex.Release()

	opKind := ""
	for _, op := range doc.Operations {
		opKind = string(op.Operation)
		log.Debugf("Operation is %s (%d selections)", opKind, len(op.SelectionSet))

		if opKind == "subscription" {
			ch := make(chan bool)
			subData := req.Extensions["subscription"].(map[string]interface{})

			err := sendSubscriptionCheck(
				subData["subscriptionId"].(string),
				subData["verifier"].(string),
				subData["callbackUrl"].(string),
			)
			if err != nil {
				log.Errorf("Callback subscription cannot be establisted.")
				continue
			}

			SubscriptionDB.Store(subData["subscriptionId"].(string), ch)

			go runSubscription(req, op, fragments, ch)
		} else {
			log.Infof("Operation %s (%s)", op.Name, opKind)

			// Sanity check
			firstField, ok := op.SelectionSet[0].(*ast.Field)
			if ok && firstField.Name == "_entities" {
				if _, ok := req.Variables["representations"]; !ok {
					log.Error("NO REPS IN ENTITIES QUERY")
					log.Errorf("%v", req.Variables)
					log.Error(string(body))
				}
			}

			result, err = doCreateResponse(schema, op.Name, op.SelectionSet, req.Variables, string(op.Operation), fragments, 1)
			if err != nil {
				log.Errorf("Error creating response: %s", err)
				log.Errorf("Body: %s", req.Query)
				log.Errorf("Variables: %v", req.Variables)
			}
			if RunValidator {
				f, _ := os.OpenFile("./validation/"+op.Name, os.O_WRONLY|os.O_TRUNC|os.O_CREATE, 0755)
				stringOut := strings.Builder{}
				format := formatter.NewFormatter(&stringOut, formatter.WithIndent(" "))
				format.FormatQueryDocument(doc)
				f.WriteString(stringOut.String())
				validation := ValidateResponse(schema, string(op.Operation), op.SelectionSet, result, fragments, 0)
				f.WriteString(validation)
				f.Close()
			}
		}

	}

	latency := LatencyGenerator.GenerateLatency(time.Now())
	log.Debugf("Generating latency of: %s", latency.String())
	time.Sleep(latency)

	data, _ := json.Marshal(struct {
		Data map[string]interface{} `json:"data"`
	}{Data: result})

	if log.GetLevel() >= log.DebugLevel {
		log.Debugf("Result: %s", string(data))
	}

	for k, v := range ExtraHeaders {
		w.Header().Set(k, v)
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Content-Length", fmt.Sprintf("%d", len(data)))

	w.Write(data)
}

type SchemaData struct {
	schema        *ast.Schema
	mutex         sync.RWMutex
	updatePending bool
}

func (s *SchemaData) TryGet() *ast.Schema {
	if s.updatePending || !s.mutex.TryRLock() {
		log.Warn("Cannot get supergraph read lock.")
		return nil
	}
	s.mutex.RLock()
	return s.schema
}

func (s *SchemaData) Get() *ast.Schema {
	if s.updatePending {
		return nil
	}
	s.mutex.RLock()
	return s.schema
}

func (s *SchemaData) Release() {
	s.mutex.RUnlock()
}

func (s *SchemaData) Update(newSchema *ast.Schema) {
	log.Info("Supergraph update starting.")
	s.updatePending = true
	s.mutex.Lock()
	s.updatePending = false
	s.schema = newSchema
	s.mutex.Unlock()
	log.Info("Supergraph update complete.")
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

func loadSchema(filename string) error {
	log.Infof("Loading supergraph: %s", filename)

	fileContents, err := os.ReadFile(filename)
	if err != nil {
		return fmt.Errorf("cannot read local schema file: %s", err)
	}
	schemaSDL := string(fileContents)

	/*
		//commented out since only applies if we are just loading a subgraph
		// file, lately we've been mostly using supergraphs for ease.

		if Fed2Regex.Match(fileContents) {
			log.Infof("Fed 2 mode enabled.")
			schemaSDL += GetFederationSymbols(2)
		} else {
			log.Infof("Fed 1 mode enabled.")
			schemaSDL += GetFederationSymbols(1)
		}
	*/

	input := ast.Source{
		Name:    "schema.graphql",
		Input:   schemaSDL,
		BuiltIn: false,
	}

	ast, err := gql.LoadSchema(&input)
	if err != nil {
		return fmt.Errorf("cannot parse local schema file: %s", err)
	}
	SchemaMutex.Update(ast)

	return nil

}

// watchSupergraph - go routine to update supergraph when changes occur
func watchSupergraph(watcher *fsnotify.Watcher, filename string) {

	for {
		select {
		case event, ok := <-watcher.Events:
			if !ok {
				return
			}
			log.Debugf("Supergraph file activity: %s", event)
			if event.Has(fsnotify.Write) {
				log.Debugf("Supergraph file was written to: %s", event.Name)
				err := loadSchema(filename)
				if err != nil {
					log.Errorf("Cannot reload modified Supergraph schema: %s", err)
				}
			}
		case err, ok := <-watcher.Errors:
			if !ok {
				return
			}
			log.Error("Supergraph watcher error:", err)
		}
	}
}

func getDuration(duration string) time.Duration {
	d, err := time.ParseDuration(duration)
	if err != nil {
		log.Warnf("Error parsing duration: %s", duration)
	}
	return d
}

func main() {

	godotenv.Load()

	SubscriptionDB = new(sync.Map)
	ExtraHeaders = make(map[string]string)
	var headerFlags StringArrayFlag

	var logLevel = flag.String("log", "warn", "set the log level to 'trace', 'debug', 'info', 'warn', or 'error''")
	var schema = flag.String("schema", "supergraph.graphql", "read a local schema file")
	var help = flag.Bool("h", false, "show usage information")
	var logJSON = flag.Bool("log-json", false, "log output as JSON")
	var port = flag.Int("port", 8080, "port to listen on")
	var validate = flag.Bool("validate", false, "validate response to query")
	flag.Var(&headerFlags, "header", "HTTP header to add to response")

	var baseLatency = flag.String("latency", "0", "base latency to add (ie, 1ms, 2s, or 5m)")
	var sawAmplitude = flag.String("saw-amplitude", "0", "amplitude of the latency sawtooth wave (ie, 1ms, 2s, or 5m)")
	var sawPeriod = flag.String("saw-period", "0", "period of the latency saw tooth wave (ie, 1ms, 2s, or 5m)")
	var sineAmplitude = flag.String("sine-amplitude", "0", "amplitude of the latency sine wave (ie, 1ms, 2s, or 5m)")
	var sinePeriod = flag.String("sine-period", "0", "period of the latency sine wave (ie, 1ms, 2s, or 5m)")
	var squarePeriod = flag.String("square-period", "0", "period of the latency square wave (ie, 1ms, 2s, or 5m)")
	var squareAmplitude = flag.String("square-amplitude", "0", "amplitude of the latency square wave (ie, 1ms, 2s, or 5m)")
	var trianglePeriod = flag.String("triangle-period", "0", "period of the latency triangle wave (ie, 1ms, 2s, or 5m)")
	var triangleAmplitude = flag.String("triangle-amplitude", "0", "amplitude of the latency triangle wave (ie, 1ms, 2s, or 5m)")

	flag.Parse()

	if *help {
		fmt.Printf("Usage: %s <flags>\n", os.Args[0])
		flag.PrintDefaults()
		os.Exit(0)
	}

	for _, header := range headerFlags {
		parts := strings.SplitN(header, "=", 2)
		if len(parts) != 2 {
			log.Warnf("%s is not a valid header, use x=y syntax", header)
			continue
		}
		ExtraHeaders[parts[0]] = parts[1]
	}

	delayCfg := &latency.LatencyCfg{
		Base:              getDuration(*baseLatency),
		SineAmplitude:     getDuration(*sineAmplitude),
		SinePeriod:        getDuration(*sinePeriod),
		SawAmplitude:      getDuration(*sawAmplitude),
		SawPeriod:         getDuration(*sawPeriod),
		SquareAmplitude:   getDuration(*squareAmplitude),
		SquarePeriod:      getDuration(*squarePeriod),
		TriangleAmplitude: getDuration(*triangleAmplitude),
		TrianglePeriod:    getDuration(*trianglePeriod),
	}
	LatencyGenerator = latency.NewSimpleLatencyGenerator(time.Now(), delayCfg)

	RunValidator = *validate
	if RunValidator {
		_ = os.MkdirAll("./validation", 0755)
	}

	setLogLevel(*logLevel)
	if *logJSON {
		log.SetFormatter(&log.JSONFormatter{})
	}

	err := loadSchema(*schema)
	if err != nil {
		log.Fatalf("Cannot load schema: %s", err)
	}

	watcher, err := fsnotify.NewWatcher()
	if err != nil {
		log.Fatal(err)
	}
	defer watcher.Close()
	go watchSupergraph(watcher, *schema)

	// Add supergraph file to watcher
	err = watcher.Add(*schema)
	if err != nil {
		log.Fatal(err)
	}

	router := httprouter.New()
	router.POST("/", runProcessor)

	fmt.Printf("Subgraph Server running on port %d\n", *port)
	log.Fatal(http.ListenAndServe(fmt.Sprintf(":%d", *port), router))
}

func init() {
	log.SetFormatter(&log.TextFormatter{})
	log.SetOutput(os.Stderr)
}
