# RTF Toolbox

This directory contains the RTF toolbox Docker image.

## Adding new binaries

To add further binaries to the image in addition to `rtf` itself, update the Dockerfile to run a
release build of the binary in question as part of the builder image, and then copy the binary into
the runtime image.

## Adding new scripts

Scripts placed in the `/scripts` directory will be:

- Copied to `/toolbox/scripts/` in the container
- Made executable automatically
- Available in the PATH for easy execution

## Image tags

- On every merge to `main` the `rtf-toolbox` will be published with the image tag `edge`. This
  should not be considered the latest stable version of the toolbox and is published to allow
  testing of the latest updates to the toolbox before releasing a stable version.
