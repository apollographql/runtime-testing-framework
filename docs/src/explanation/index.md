<!-- diataxis-type: explanation -->

# Understanding RTF

This section explains the thinking behind RTF: what it is, how it fits into your testing strategy,
and why it's designed the way it is.

The short version is that RTF makes it easy to develop and debug tests that need more resources than
you have available on your local machine. You write a Test Plan once, iterate on it locally, and
then run the same Test Plan at scale in production-like environments without needing to rebuild your
tooling for each new setup.

- [Overview][0] covers the problem RTF solves and the design principles behind it.
- [Where RTF fits in your test suite][1] helps you decide which of your tests might benefit from
  being run under RTF and which wont.

[0]: ./overview.md
[1]: ./testing-with-rtf.md
