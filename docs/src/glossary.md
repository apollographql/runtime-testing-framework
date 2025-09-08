# Glossary

## Environment

An RTF configuration file for specifying how to set up and tear down the services and infrastructure
under test using Command and File providers. As with all RTF configuration files, it is possible to
template an Environment configuration using Values provided in the Test Plan.

## Provider

A self contained piece of functionality within RTF that can provide file content for use in
executing commands found in a Scenario or Environment configuration. Providers are declared as part
of RTF configuration files by specifying their kind and the parameters needed to run them.

### Command Provider

A Provider that defines an executable command along with environment variables and a set of attached
File Providers whose content will be made available to the command when it is run. The purpose of
each of the Scenario and Environment configuration files is to parameterise, resolve and run one or
more Command Providers.

### File Provider

A Provider that will write out one or more files into a Test Plan's output directory when resolved.
File Providers range from being completely general purpose (e.g. pulling an arbitrary file from
GitHub) to generating data specific to the Test Plan being resolved (e.g. generating valid GraphQL
operations to run against the supergraph under test).

## RTF

May refer to either the Runtime Testing Framework as a whole or the `rtf` CLI which is used to
resolve and run Test Plans.

## Scenario

An RTF configuration file for specifying how to run a test against services spun up by an
Environment configuration using Command and File providers. As with all RTF configuration files, it
is possible to template a Scenario configuration using Values provided in the Test Plan.

## Templating

The use of template strings within RTF configuration files for declaring how users of that
configuration file may specify how to set Provider parameters and Command Provider environment
variables. Template strings are denoted with opening and closing double braces surrounding the name
of the Value to inject with a single space at either side: `"{{ my_value }}"`

## Test Plan

The main RTF configuration file that defines an runnable test by combining a Scenario configuration
with the Environment configuration it should be executed against. If either the Scenario or
Environment supports Templating Values then they can be specified statically as part of the Test
Plan itself or dynamically through command line arguments to the RTF CLI.

## Values

Templating Values are declared within the Scenario and Environment configuration files and have
their values defined within the Test Plan referencing those configuration files.
