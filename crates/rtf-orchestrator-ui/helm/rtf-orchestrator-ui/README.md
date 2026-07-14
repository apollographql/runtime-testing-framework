# rtf-orchestrator-ui

Helm chart for the orchestrator status UI

## Values

| Key              | Type   | Default                                                                                                                                                   | Description                                                        |
| ---------------- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| config           | object | `{"host":"0.0.0.0","logLevel":"warn","port":"8080"}`                                                                                                      | Application configuration                                          |
| config.host      | string | `"0.0.0.0"`                                                                                                                                               | Host address the server listens on                                 |
| config.logLevel  | string | `"warn"`                                                                                                                                                  | Log level for the RUST_LOG environment variable                    |
| config.port      | string | `"8080"`                                                                                                                                                  | Port the server listens on                                         |
| fullnameOverride | string | `""`                                                                                                                                                      | Overrides the fully qualified app name                             |
| image            | object | `{"pullPolicy":"IfNotPresent","repository":"us-central1-docker.pkg.dev/platform-cross-environment/apollo-private-docker/rtf-orchestrator-ui","tag":null}` | Container image configuration                                      |
| image.pullPolicy | string | `"IfNotPresent"`                                                                                                                                          | Image pull policy                                                  |
| image.repository | string | `"us-central1-docker.pkg.dev/platform-cross-environment/apollo-private-docker/rtf-orchestrator-ui"`                                                       | Image repository                                                   |
| image.tag        | string | `nil`                                                                                                                                                     | Image tag. Defaults to the chart appVersion when not set           |
| imagePullSecrets | list   | `[]`                                                                                                                                                      | List of image pull secrets                                         |
| nameOverride     | string | `""`                                                                                                                                                      | Overrides the chart name                                           |
| podAnnotations   | object | `{}`                                                                                                                                                      | Annotations to add to the pod template                             |
| resources        | object | `{}`                                                                                                                                                      | Resource requests and limits for the rtf-orchestrator-ui container |
| service          | object | `{"create":true,"port":8080,"type":"ClusterIP"}`                                                                                                          | Kubernetes Service configuration                                   |
| service.create   | bool   | `true`                                                                                                                                                    | Whether to create a Service resource                               |
| service.port     | int    | `8080`                                                                                                                                                    | Service port                                                       |
| service.type     | string | `"ClusterIP"`                                                                                                                                             | Service type                                                       |
