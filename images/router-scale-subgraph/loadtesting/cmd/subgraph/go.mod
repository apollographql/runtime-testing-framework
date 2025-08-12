module github.com/apollosolutions/loadtesting/cmd/subgraph

go 1.22.0

require (
	github.com/julienschmidt/httprouter v1.3.0
	github.com/sirupsen/logrus v1.9.3
	github.com/vektah/gqlparser/v2 v2.5.11
)

require (
	github.com/agnivade/levenshtein v1.1.1 // indirect
	github.com/apollosolutions/loadtesting/pkg/tools v0.0.0-unpublished // indirect
	github.com/fsnotify/fsnotify v1.7.0 // indirect
	github.com/joho/godotenv v1.5.1 // indirect
	golang.org/x/sys v0.21.0 // indirect
)

replace github.com/apollosolutions/loadtesting/pkg/tools v0.0.0-unpublished => ../../pkg/tools
replace github.com/apollosolutions/loadtesting/pkg/latency v0.0.0-unpublished => ../../pkg/latency
