<!-- diataxis-type: explanation -->

# The Apollo Runtime Testing Framework

Welcome to [RTF][0]!

RTF makes it easy to develop and debug tests that need more resources than you have available on
your local machine. You describe your test once as a [Test Plan][1], iterate on it locally using the
`rtf` CLI, and then run the same Test Plan at scale using the [RTF Orchestrator][2].

This documentation site covers how to get set up with the tooling and work with RTF Test Plans.
While it is not necessary to read through every page in order, please be aware that the contents of
the tutorials and guides may require you to have already worked through previous sections in some
places. Where this is the case, links will be provided to the relevant sections of the
documentation.

### Who are these docs for?

The documentation is split into two main sections: "User Documentation" and "Developer
Documentation".

**User Documentation** covers both how to make use of the `rtf` CLI for running existing Test Plans
and how to write your own Test Plans.

**Developer Documentation** covers details on the internal design of RTF and how to work within the
[runtime-testing-framework][0] repo.

### Where do I ask for help if I am stuck?

RTF is written and maintained by the Runtime Readiness team. You can find our Confluence space
[here][5] and we can be reached in Slack in our team channel: `#team-runtime-readiness` where you
can find links to our intake process and other useful information in the channel bookmarks.

For questions, feedback and support with RTF specifically we also have the
`#proj-runtime-testing-framework` channel.

### Where can I find examples of RTF in use?

The Orchestrator's list of [registered test plans][6] is a good place to start to see what your
fellow Apollonauts are doing with RTF. If you have any questions about how one of those test plans
works you can reach out to one of the listed owners to chat to them about the specifics!

[0]: https://github.com/apollographql/runtime-testing-framework
[1]: ./reference/glossary.md#test-plan
[2]: ./reference/glossary.md#rtf-orchestrator-service
[3]: https://www.apollographql.com/blog/apollos-runtime-testing-framework
[4]: ./explanation/index.md
[5]: https://apollographql.atlassian.net/wiki/spaces/RUNTIMEREADINESS/pages/1470103558/Runtime+Readiness+Team+Charter
[6]: https://api.rtf.apollographql.com/ui/test-plans
