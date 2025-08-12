module github.com/apollosolutions/loadtesting/cmd/rehydrate

go 1.22.0

require (
	github.com/agnivade/levenshtein v1.1.1 // indirect
	github.com/apollosolutions/loadtesting/pkg/client v0.0.0-unpublished // indirect
	github.com/apollosolutions/loadtesting/pkg/tools v0.0.0-unpublished // indirect
	github.com/joho/godotenv v1.5.1 // indirect
	github.com/sirupsen/logrus v1.9.3 // indirect
	github.com/vektah/gqlparser/v2 v2.5.11 // indirect
	golang.org/x/sys v0.20.0 // indirect
)

replace github.com/apollosolutions/loadtesting/pkg/client v0.0.0-unpublished => ../../pkg/client

replace github.com/apollosolutions/loadtesting/pkg/tools v0.0.0-unpublished => ../../pkg/tools
