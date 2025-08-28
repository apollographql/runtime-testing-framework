## `router-scale-subgraph` Docker Image

This contains the instructions for creating the `router-scale-subgraph` docker image. The
`router-scale-subgraph` docker image is built from the code in the `loadtesting` directory. This is
copied from `loadtesting` directory in the [router-scale][0] repo. All files not required to build
the image have been deleted from the copy in this repo.

## Build

To build the container locally, make sure you are in the same directory as this README, use the
following command:

```bash
docker build -t router-scale-subgraph .
```

## Run

To run the container, mount your local GraphQL schema file into the container and provide its path
to the subgraph. Replace `/absolute/path/to/schema.graphql` with the actual path to your schema
file.

```bash
docker run -d \
    -v /absolute/path/to/schema.graphql:/schema.graphql \
    -t router-scale-subgraph -schema "/schema.graphql"
```

To see all options for the subgraph container run

```bash
docker run -t router-scale-subgraph -h
```

[0]: https://github.com/apollographql/router-scale
