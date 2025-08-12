# FetchSup - Fetch Supergraph & Subgraph SDLS

## Build

`go build`

## Usage:

`fetchsup -h`

## Use External Router Config Template

`fetchsup --template your_template.txt`

## Example Template Syntax

Template syntax is Go [text/templates](https://pkg.go.dev/text/template) which is terrible. Might
switch to Jinja in the future.

```
supergraph:
  listen: 0.0.0.0:${env.PORT}
include_subgraph_errors:
  all: true
override_subgraph_url:
{{ range . }}  {{.SubgraphName}}: {{.SubgraphRoute}}
{{ end -}}
```
