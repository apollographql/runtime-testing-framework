package client

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"

	log "github.com/sirupsen/logrus"
)

const MeQuery = `query Me { me {id }}`

const GetOperationSignature = `query GetOpSignature($serviceId: ID!, $operationId: ID!) {
	service(id: $serviceId) {  
	  operation(id: $operationId) {
		signature
	  }
	}
  } `

const FetchQueryIDs = `query FetchQueryIDs($serviceId: ID!, $name: String!, $from: Timestamp!, $to: Timestamp!, $filter: OperationInsightsListFilterInput, $first: Int, $orderBy: OperationInsightsListOrderByInput) {
	service(id: $serviceId) {
	  variant(name: $name) {
		id
		byRequests: operationInsightsList(from: $from, to: $to, filter: $filter, first: $first, orderBy: $orderBy) {
		  nodes {
			id
			displayName
			type
			requestCountPerMin
		  }
		}
	  }
	}
  } 
`

const SubgraphQuery = `query Service($serviceId: ID!, $name: String!) {
	service(id: $serviceId) {
	  variant(name: $name) {
		id
		subgraphs {
		  name
		  activePartialSchema {
			sdl
		  }
		}
	  }
	}
  }`

const SupergraphQuery = `query SupergraphFetchQuery($graph_id: ID!, $variant: String!) {
  frontendUrlRoot
  service(id: $graph_id) {
    variant(name: $variant) {
		subgraphs {
		  name
		  activePartialSchema {
			sdl
		  }
		}
		latestApprovedLaunch {
        id
        createdAt
        build {
          result {
            ... on BuildSuccess {
              coreSchema {
                coreHash
                coreDocument
              }
            }
          }
        }
      }
	  sourceVariant {
        subgraphs {
          name
          activePartialSchema {
            sdl
          }
        }
      }
	}
    mostRecentCompositionPublish(graphVariant: $variant) {
      errors {
        message
        code
      }
    }
  }
}
`

type GQLQuery struct {
	Variables     map[string]interface{} `json:"variables,omitempty"`
	Query         string                 `json:"query"`
	OperationName string                 `json:"operationName,omitempty"`
}

type UplinkRouterConfig struct {
	TypeName      string `json:"__typename"`
	ID            string `json:"id"`
	SupergraphSDL string `json:"supergraphSdl"`
}

type UplinkRouterConfigWrapper struct {
	RouterConfig UplinkRouterConfig `json:"routerConfig"`
}

type UplinkResult struct {
	Data UplinkRouterConfigWrapper `json:"data"`
}

type BuildErrorLocation struct {
	Line   int64 `json:"line"`
	Column int64 `json:"column"`
}

type BuildStatusError struct {
	Message   string
	Locations []BuildErrorLocation
}

type BuildStatusWebhook struct {
	EventType           string             `json:"eventType"`
	EventID             string             `json:"eventID"`
	SupergraphSchemaURL string             `json:"supergraphSchemaURL"`
	BuildSucceeded      bool               `json:"buildSucceeded"`
	BuildErrors         []BuildStatusError `json:"buildErrors"`
	GraphID             string             `json:"graphID"`
	VariantID           string             `json:"variantID"`
	Timestamp           string             `json:"timestamp"`
}

type CompositionResult struct {
	TypeName           string `json:"__typename"`
	SupergraphSDL      string `json:"supergraphSdl"`
	GraphCompositionID string `json:"graphCompositionID"`
}

type SchemaResult struct {
	Document   string `json:"document"`
	FieldCount int64  `json:"fieldCount"`
	TypeCount  int64  `json:"typeCount"`
}

type CoreSchema struct {
	CoreDocument string `json:"coreDocument"`
	CoreHash     string `json:"coreHash"`
}

type BuildResult struct {
	CoreSchema CoreSchema `json:"coreSchema"`
}

type Build struct {
	Result BuildResult `json:"result"`
}

type Launch struct {
	ID        string `json:"id"`
	Build     Build  `json:"build"`
	CreatedAt string `json:"createdAt"`
}

type SchemaTag struct {
	CompositionResult CompositionResult `json:"compositionResult"`
	Schema            SchemaResult      `json:"schema"`
}

type PartialSchema struct {
	CreatedAt string `json:"createdAt"`
	IsLive    bool   `json:"isLive"`
	SDL       string `json:"sdl"`
}

type OperationInsightsListItem struct {
	CacheHitRate       float64 `json:"cacheHitRate"`
	CacheTTLP50Ms      float64 `json:"cacheTtlP50Ms"`
	DisplayName        string  `json:"displayName"`
	ErrorCount         int64   `json:"errorCount"`
	ErrorCountPerMin   float64 `json:"errorCountPerMin"`
	ErrorPercentage    float64 `json:"errorPercentage"`
	ID                 string  `json:"id"`
	Name               string  `json:"name"`
	RequestCount       int64   `json:"requestCount"`
	RequestCountPerMin float64 `json:"requestCountPerMin"`
	ServiceTimeP50Ms   float64 `json:"serviceTimeP50Ms"`
	Type               string  `json:"type"`
}

type OperationInsightsListPageInfo struct {
	EndCursor   string
	StartCursor string
}

type GraphVariantOperationInsightsListItemEdge struct {
	Node OperationInsightsListItem `json:"node"`

	Cursor string `json:"cursor"`
}

type GraphVariantOperationInsightsListItemConnection struct {
	Edges      []GraphVariantOperationInsightsListItemEdge
	Nodes      []OperationInsightsListItem
	PageInfo   OperationInsightsListPageInfo
	TotalCount int `json:"totalCount"`
}

type Subgraph struct {
	Name                string         `json:"name"`
	ActivePartialSchema *PartialSchema `json:"activePartialSchema"`
}

type GraphVariant struct {
	LatestApprovedLaunch Launch                                          `json:"latestApprovedLaunch"`
	Name                 string                                          `json:"name"`
	Subgraphs            []Subgraph                                      `json:"subgraphs"`
	ByRequests           GraphVariantOperationInsightsListItemConnection `json:"byRequests"`
	SourceVariant        *GraphVariant                                   `json:"sourceVariant"`
}

type Operation struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	Signature string `json:"signature"`
	Truncated bool   `json:"truncated"`
}

type ServiceResult struct {
	Variants  []GraphVariant `json:"variants"`
	Variant   GraphVariant   `json:"variant"`
	SchemaTag SchemaTag      `json:"schemaTag"`
	Operation Operation      `json:"operation"`
}

type SupergraphFetch struct {
	FrontendURLRoot string        `json:"frontendUrlRoot"`
	Service         ServiceResult `json:"service"`
}

type Error struct {
	Message   string
	Locations []map[string]int
	Path      []interface{}
}

type SupergraphResult struct {
	Data       SupergraphFetch        `json:"data"`
	Errors     []Error                `json:"errors,omitempty"`
	Extensions map[string]interface{} `json:"extensions,omitempty"`
}

func RunOp[T interface{}](name, query, key string, variables map[string]interface{}) (*T, error) {

	var q = GQLQuery{
		Variables: variables,
		Query:     query,
		//OperationName: name,
	}

	//log.Infof("GQL Op: %s %s %s", name, query, variables)

	body, _ := json.Marshal(q)

	//log.Infof("BODY: %s", body)

	tr := &http.Transport{
		DisableKeepAlives:  true,
		DisableCompression: false,
	}

	httpClient := http.Client{Transport: tr}
	postRequest, err := http.NewRequest(
		"POST",
		"https://graphql.api.apollographql.com/api/graphql",
		bytes.NewBuffer(body))

	if err != nil {
		log.Errorf("Could create request %s", err)
		return nil, fmt.Errorf("could create request %s", err)
	}

	postRequest.Close = true
	postRequest.Header.Set("Accept", "*/*")
	postRequest.Header.Set("Content-Type", "application/json")
	postRequest.Header.Set("x-api-key", key)
	postRequest.Header.Set("apollo-sudo", "true")
	postRequest.Header.Set("apollographql-client-name", "lovelace-benchmark")
	postRequest.Header.Set("apollographql-client-version", "0.1.0")

	resp, err := httpClient.Do(postRequest)

	if err != nil {
		log.Errorf("Could not retrieve supergraph SDL %s", err)
		return nil, fmt.Errorf("could not retrieve supergraph SDL %s", err)
	}
	defer resp.Body.Close()

	data, _ := io.ReadAll(resp.Body)
	result := new(T)

	// Decode response
	err = json.Unmarshal(data, result)
	if err != nil {
		log.Errorf("Could not decode supergraph result: %s", err)
		return nil, fmt.Errorf("could not decode supergraph result: %s", err)
	}

	if log.GetLevel() == log.DebugLevel {
		data, _ := json.MarshalIndent(result, "", "    ")
		log.Debug("API Response", string(data))
	}

	return result, nil
}
