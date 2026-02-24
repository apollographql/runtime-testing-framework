<!-- diataxis-type: howto -->

# Generating JSON schemas for config files

The `rtf json-schemas <config>` command can be used to generate the JSON schema for the test plan,
environment and scenario config file formats.

To save the JSON schema files, navigate to the directory they should be stored in and run:

```bash
rtf json-schemas test-plan > test-plan-schema.json
rtf json-schemas environment > environment-schema.json
rtf json-schemas scenario > scenario-schema.json
```

To reference the JSON schemas in your config files, add the following annotation to the start of
your file

```yaml
# yaml-language-server: $schema=relative/path/to/test-plan-schema.json

name: Test Plan
description: A test plan config validated against its JSON schema
...
```
