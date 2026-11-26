#!/usr/bin/env bash
# Test the same tarball that will be published, outside the monorepo.
set -euo pipefail
archive=$(realpath "${1:?SDK tarball}")
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
cd "$tmp"
printf '%s\n' '{"name":"sdk-consumer","private":true}' > package.json
npm install --offline --ignore-scripts --no-audit --no-fund "$archive"
node --input-type=module -e 'import {connect} from "@stellarr/sdk"; if (typeof connect !== "function") process.exit(1)'
mkdir activity
printf '%s\n' '{"id":"poll","protocol":1,"version":"1.0.0","name":"Poll","description":"SDK smoke test","icon":"icon.png"}' > activity/stellarr.json
printf '%s\n' '<title>Poll</title>' > activity/index.html
node -e 'require("node:fs").writeFileSync("activity/icon.png",Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aKioAAAAASUVORK5CYII=","base64"))'
node_modules/.bin/stellarr validate --dir activity
node_modules/.bin/stellarr pack --dir activity --out release
test -s release/stellarr-release.json
