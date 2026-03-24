# RTF Orchestrator - Local Stack

## Pre-reqs

You need to have the following tools installed for this local stack to run on your Mac:

```bash
brew install docker docker-compose kind tilt docker-buildx kubectl
```

You also need to be authenticated to GCP as an `apollographql` user in order to pull images from the
container registry:

```bash
gcloud auth login
gcloud auth configure-docker us-docker.pkg.dev
```

We also recommend that you install `k9s` for easier management of your local clusters

```bash
brew install derailed/k9s/k9s
```

## Creating and tearing down your local clusters

```bash
# Create the management and workload clusters along with required namespaces in the management cluster
./setup.sh

# Remove both local clusters
./teardown.sh
```

### Spinning up the stack

```bash
# In one terminal window
tilt up --context kind-rtf-mgmt

# To work with the management cluster from a second terminal
k9s --context kind-rtf-mgmt
```

### Running the current demo workflow

```bash
# Create the configmap containing the RTF environment definition
kubectl -n cluster-api --context kind-rtf-mgmt create configmap rtf-environment-config --from-file=<path-to-test-plan>/environment.yaml
# Create the workflow (monitor in k9s)
kubectl -n cluster-api --context kind-rtf-mgmt create -f demo/environment-workflow.yaml

# Wait for environment to finish provisioning

# Create the config map for the RTF scenario definition
kubectl -n demo-namespace --context kind-rtf-workload create configmap rtf-scenario-config --from-file=<path-to-test-plan>/scenario.yaml
# Run the scenario job
kubectl -n demo-namespace --context kind-rtf-workload apply -f demo/scenario-job.yaml

# Clean up the namespace and scenario config map (/ data in the volume once that's how we're handling this)
```
