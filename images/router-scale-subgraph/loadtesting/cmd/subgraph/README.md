# Subgraph Server

The Subgraph Server is a GraphQL service that can return realistic data for any GraphQL request by
utilizing the Supergraph to generate responses based on schema type information.

The subgraph server supports callback based Federated subscriptions as well.

## Building

`go build`

## Running

`./subgraph -schema my_schema.graphql`

## Usage Info

`./subgraph -h`
