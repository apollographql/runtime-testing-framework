# The Apollo Runtime Testing Framework

Welcome to [RTF][0]!

RTF is a tool for writing, running and debugging tests of the Apollo Runtime under a variety of
deployment setups. This documentation site covers how to get set up with the tooling and work with
RTF Test Plans. While it is not necessary to read through every page in order, please be aware that
the contents of the tutorials and guides may require you to have already worked through previous
sections in some places. Where this is the case, links will be provided to the relevant sections of
the documentation.

### Who are these docs for?

The documentation is split into two main sections: "User Documentation" and "Developer
Documentation".

**User Documentation** covers both how to make use of the `rtf` CLI for running existing Test Plans
(such as those found in the [rtf-morgue][1] repo) and how to write your own Test Plans.

**Develop Documentation** covers details on the internal design of RTF and how to work within the
[runtime-testing-framework][0] repo.

### Where do I ask for help if I am stuck?

RTF is written and maintained by the Runtime Readiness team. You can find our Confluence space
[here][2] and we can be reached in Slack in our team channel: `#team-runtime-readiness` where you
can find links to our intake process and other useful information in the channel bookmarks.

For questions, feedback and support with RTF specifically we also have the
`#proj-runtime-testing-framework` channel.

### Where can I find examples of RTF in use?

The [rtf-morgue][1] repo contains a number of test plans, helper scripts and GitHub Actions
workflows for running tests of the Apollo Router. This repo is maintained by the Runtime Readiness
team in order to support Router Core with release validation, regression testing and running
investigations into customer issues.

[0]: https://github.com/apollographql/runtime-testing-framework
[1]: https://github.com/apollographql/rtf-morgue
[2]: https://apollographql.atlassian.net/wiki/spaces/RUNTIMEREADINESS/pages/1470103558/Runtime+Readiness+Team+Charter
