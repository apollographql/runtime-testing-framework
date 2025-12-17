<!-- diataxis-type: explanation -->

# Logging Philosophy

RTF takes the "no news is good news" approach - quiet by default. Logging exists to help debug
issues and provide feedback when things go wrong.

## Design goals

When adding log statements to RTF, consider whether the information is helpful and avoid
overwhelming users with unnecessary data.

## Performance considerations

Logging performance is not a primary concern in RTF. The framework is expected to become I/O bound
(waiting for network requests, file operations, etc.) before logging becomes a bottleneck.

However, keep these guidelines in mind:

- Avoid expensive computations solely for log messages
- Use structured fields instead of string formatting when possible
- Don't worry about the overhead of log statements that won't be displayed

## Testing approach

Do not test logging at the low level using a crate like `tracing_test`. Instead, make sure to test
the output the user sees in the CLI tests. Tests should ensure the user sees the logging statement
in situations where it is expected and required to give helpful feedback. Tests should not cover
`debug` and `trace` level logs.
