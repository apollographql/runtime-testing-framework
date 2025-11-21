# Creating a custom provider

This guide assumes you've completed the ["Using file providers"](using-file-providers.md) guide. You
should already have the files in a directory named `rtf-hello-world`. Your directory should be in
the state it was at the end of that guide.

> **Note** This guide is a bit of a detour from the main flow. We're going to create a reusable
> custom provider that we'll use in the next guide. Custom providers allow you to encapsulate
> complex file generation logic into reusable components that can be shared across multiple test
> plans.

## What are custom providers?

Custom providers are a type of file provider that execute custom commands to produce files for use
in your test plans. Unlike the file providers we've seen so far (`inline`, `relative_path`,
`required`), custom providers can generate files dynamically by running scripts or commands. This
makes them powerful for creating reusable file generation logic.

In this guide, we'll create a custom provider that generates service configuration files. The
provider will create two interdependent files: a `service.config` file and a startup script that
uses it. Both files will reference the same variables, demonstrating how custom providers can
generate multiple related files with shared context.

## Creating the provider directory structure

First, let's create a directory structure for our custom provider. We'll create a `providers`
directory to hold our custom provider definition and any scripts it needs:

```bash
mkdir -p providers/scripts
```

## Defining the custom provider

A custom provider is defined in a YAML file that specifies the name, description, required
variables, and the command to execute. Let's create our custom provider definition:

```bash
touch providers/service-config.yaml
```

Add the following content to `providers/service-config.yaml`:

```yaml
name: service-config
description: Generates a service.config file and startup script for running a service

variable_definitions:
  - name: service_name
    description: The name of the service
  - name: port
    description: The port the service will run on
    default: "8080"
  - name: host
    description: The host address for the service
    default: "localhost"

command:
  name: generate-service-config.sh
  kind: relative_path
  path: scripts/generate-service-config.sh

env_vars:
  SERVICE_NAME: "{{ service_name }}"
  PORT: "{{ port }}"
  HOST: "{{ host }}"
```

This custom provider definition:

- **`name`**: Identifies the custom provider
- **`description`**: Explains what the provider does
- **`variable_definitions`**: Declares the variables that must be provided when using this provider
  - `service_name` is required (no default)
  - `port` has a default value of `"8080"`, so it's optional
  - `host` has a default value of `"localhost"`, so it's optional
- **`command`**: Specifies the script to execute (we'll create this next)
- **`env_vars`**: Passes the variables to the script as environment variables using templating

> **Note** The path in the `command` section is relative to the custom provider definition file
> location, not the test plan file.

## Writing the generation script

Now let's create the script that will generate our files. This script will create both the
`service.config` file and a `start-service.sh` script that uses it:

```bash
touch providers/scripts/generate-service-config.sh
```

Add the following content to `providers/scripts/generate-service-config.sh`:

```sh
#!/usr/bin/env sh

# The RTF_OUTPUT environment variable is set by RTF and points to the directory
# where we should write our output files
OUTPUT_DIR="$RTF_OUTPUT"

# Generate service.config
cat > "$OUTPUT_DIR/service.config" << EOF
SERVICE_NAME=${SERVICE_NAME}
PORT=${PORT}
HOST=${HOST}
EOF

# Generate start-service.sh script that uses the service.config
cat > "$OUTPUT_DIR/start-service.sh" << EOF
#!/usr/bin/env sh

# This script depends on service.config being in the same directory
CONFIG_FILE="\$(dirname "\$0")/service.config"

if [ ! -f "\$CONFIG_FILE" ]; then
  echo "Error: service.config not found at \$CONFIG_FILE"
  exit 1
fi

# Read the configuration file
. "\$CONFIG_FILE"

# Dummy script that just echoes the service configuration
echo "Service: \$SERVICE_NAME"
echo "Host: \$HOST"
echo "Port: \$PORT"
echo "Starting service \$SERVICE_NAME on \$HOST:\$PORT"
EOF

# Make the startup script executable
chmod +x "$OUTPUT_DIR/start-service.sh"

echo "Generated service.config and start-service.sh in $OUTPUT_DIR"
```

This script:

1. Uses the `$RTF_OUTPUT` environment variable (set automatically by RTF) to know where to write
   files
2. Generates `service.config` with the service name, port, and host templated in
3. Generates `start-service.sh` that references the config file and uses the same variables
4. Makes the startup script executable

Notice how both files reference the same variables (`SERVICE_NAME`, `PORT`, and `HOST`), and the
startup script depends on the config file existing. This demonstrates the interdependency between
the generated files.

Make sure the script is executable:

```bash
chmod +x providers/scripts/generate-service-config.sh
```

## Verifying the custom provider definition

Let's verify that our custom provider definition is valid. We can check this by looking at the
structure we've created:

```bash
$ tree providers
providers
├── service-config.yaml
└── scripts
    └── generate-service-config.sh
```

The custom provider definition file references the script using a relative path. Since the
definition file is in `providers/` and the script is in `providers/scripts/`, we use
`scripts/generate-service-config.sh` as the path.

---

In this guide, we've created a custom provider definition that can generate service configuration
files. In the next guide, we'll learn how to declare and use this custom provider in our test plan.

**Next:** [Using a custom provider](using-a-custom-provider.md)
