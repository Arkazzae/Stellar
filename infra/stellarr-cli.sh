#!/bin/sh
# The operator CLI, published into the API's folder: it runs on the API's own
# assemblies, which the image build checks are the ones it was built with. The
# publish is architecture-neutral, so the image's own .NET runtime executes it;
# no apphost for the build machine's CPU is copied into a multi-architecture image.
exec /usr/bin/dotnet /opt/stellarr/api/Stellarr.Cli.dll "$@"
