module github.com/apollosolutions/loadtesting/cmd/fetchsup

go 1.22.0

require github.com/sirupsen/logrus v1.9.3

require (
	github.com/apollosolutions/loadtesting/pkg/client v0.0.0-unpublished // indirect
	github.com/joho/godotenv v1.5.1 // indirect
	golang.org/x/sys v0.21.0 // indirect
)

replace github.com/apollosolutions/loadtesting/pkg/client v0.0.0-unpublished => ../../pkg/client
