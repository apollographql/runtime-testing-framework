# Plumbing vs porcelain

The CLI subcommands are split into two different concepts; porcelain and plumbing.

The porcelain commands expose full end-to-end functionality to the users. They can be thought of as pipelines running the functionality in the plumbing commands sequentially to achieve the user's desired outcome.

The plumbing commands expose ways for a user to run a subset of the porcelain commands. These commands are primarily used for debugging when the porcelain command has errored or so the user can test a portion of their configuration ahead of running a porcelain command.
