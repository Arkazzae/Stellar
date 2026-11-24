#!/bin/sh
# Builds the witness: the first server's own backup code (Backups/BackupArchive.cs,
# BackupEntryIntegrity.cs, BackupRestore.cs, Storage/ObjectStorage.cs,
# Storage/ObjectKeys.cs, Storage/FileSystemObjectStorage.cs at the pinned commit)
# copied unchanged next to Program.cs and CockpitStub.cs, built outside the
# repository with the .NET SDK, and run by backups-archives.sh.
#
#   build.sh <first server's source folder, server/Stellarr.Api> <output folder>
set -eu
here=$(cd "$(dirname "$0")" && pwd)
source=${1:?the folder server/Stellarr.Api}
out=${2:?an output folder outside the repository}
dotnet=${DOTNET:-$HOME/.dotnet/dotnet}
mkdir -p "$out/copied"
cp "$here/Witness.csproj" "$here/Program.cs" "$here/CockpitStub.cs" "$out/"
for f in Backups/BackupArchive.cs Backups/BackupEntryIntegrity.cs Backups/BackupRestore.cs \
  Storage/ObjectStorage.cs Storage/ObjectKeys.cs Storage/FileSystemObjectStorage.cs; do
  cp "$source/$f" "$out/copied/"
done
"$dotnet" build "$out/Witness.csproj" -c Release -o "$out/bin" -nologo -v q
