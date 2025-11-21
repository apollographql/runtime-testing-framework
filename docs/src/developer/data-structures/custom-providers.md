# Custom provider definitions

[Custom provider definitions][0] are YAML files that define reusable custom providers which can be
executed as part of a test plan. They share structural similarities with [scenario configs][1] but
differ in important ways.

## Differences from config files

While custom provider definitions use the same `CommandSection` structure as config files, they
serve a different purpose:

- **Purpose**: Custom provider definitions execute commands in order to produce file(s) that test
  plans can make use of, whereas scenario configs execute commands to perform test actions.
- **Restrictions**: Custom provider definitions cannot reference other custom providers within their
  command sections. Attempting to do so will result in a hard error during validation.
- **Templating**: Custom provider definitions are templated using arguments provided when the custom
  provider is referenced in a config file, rather than using variables from the test plan directly.

## Custom provider declarations

Custom provider definitions are loaded into config files through [custom provider declarations][3].
These declarations specify a source directory (either a local relative path or a GitHub repository)
and a mapping of provider names to definition files within that directory.

Once declared, custom providers can be referenced in file provider sections using
`kind:
custom_provider` along with a `type` field that matches the name from the declaration, and any
arguments required by the custom provider's variable definitions.

[0]: https://github.com/apollographql/runtime-testing-framework/crates/rtf-config/src/formats/custom_provider.rs#L22
[1]: ./config-files.md
[2]: ./command-providers.md
[3]: https://github.com/apollographql/runtime-testing-framework/crates/rtf-config/src/formats/custom_provider.rs#L123
