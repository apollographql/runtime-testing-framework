# Templating fields

The leaves of the config data structures are either concrete scalar values (such as numbers, strings
and booleans) or [Fields][0] which is how we support a limited form of type-checked templating
within config files.

While hard coded scalar values are parsed directly using [serde][1], `Fields` are allowed to be in
one of two states:

- `Pending`, where they hold the name of templating _value_ that the user must specify as part of
  their test plan.
- `Resolved`, where they hold a concrete [scalar][2] value, either because a value was provided
  directly within the config file or following successful templating.

Pending fields are indicated within a config file using `"{{ field_name }}"` syntax and are parsed
into the `Field::Pending` enum variant directly using serde. When writing new providers you should
make use of templating fields where it makes sense for users to be able to dynamically set values
when executing a test plan and avoid using them where such flexibility is not required.

> For example, we do not allow the `inline` file provider's content value to be templated as the
> intention is for this to always be provided within the test plan itself.

[0]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/templating.rs#L205
[1]: https://serde.rs/
[2]: https://github.com/apollographql/runtime-testing-framework/blob/37ed7a5f57c7761ee24acc1a0ece82fc5456740d/crates/rtf-config/src/templating.rs#L363
