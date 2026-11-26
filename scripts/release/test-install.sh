#!/usr/bin/env bash
# Run on a fresh release runner, after its versioned installer is public.
set -euo pipefail
version=${1:?release version}
root=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
instance="$work/instance"
origin=https://localhost:18743
mkdir "$instance"
if docker container inspect stellarr >/dev/null 2>&1 || docker volume inspect stellarr_stellarr_data >/dev/null 2>&1; then
  echo 'The release check requires a fresh Docker host.' >&2
  exit 1
fi
cd "$work"
STELLARR_INSTALLER_VERSION="$version" bash "$root/install.sh" --version "$version" --dir "$instance" \
  --yes --domain localhost --https-port 18743 --public-ip 127.0.0.1 --language en --disable-turn
test "$(docker inspect --format '{{.Config.Image}}' stellarr)" = "ghcr.io/arkazzae/stellarr:$version"
printf '\nSTELLARR_ACTIVITY_DOMAIN=activities.localhost\nSTELLARR_ACTIVITY_ORIGIN=%s\n' "$origin" >> "$instance/.env"
docker compose --project-directory "$instance" up -d --wait
curl --fail --silent --show-error --insecure "$origin/health/ready" >/dev/null
cd "$root"
while read -r path digest; do
  curl -kfsS "$origin/$path" -o "$work/body"
  echo "$digest  $work/body" | sha256sum -c --status
done < <(jq -r '.artifacts[] | [.path,.sha256] | @tsv' artifacts/manifest.json)
docker cp stellarr:/opt/stellarr/web "$work/served"
export STELLARR_TRANSLATION_ROOT="$work/served"
node web/scripts/verify-bergamot-runtime.mjs
for filter in rnnoise speex gtcrn; do
  STELLARR_CALL_FILTER=$filter node web/bench/voice/call/call.mjs --origin "$origin" --container stellarr
done

sdk_version=$(node -p 'require("./sdk/package.json").version')
npm install --prefix "$work/author" --ignore-scripts --no-audit --no-fund "@stellarr/sdk@$sdk_version"
mkdir "$work/activity"
printf '%s\n' '{"id":"release-check","protocol":1,"version":"1.0.0","name":"Release check","description":"SDK release smoke test","launchModes":["personal"],"icon":"icon.png"}' > "$work/activity/stellarr.json"
printf '%s\n' '<!doctype html><script type="module" src="./main.js"></script>' > "$work/activity/index.html"
printf '%s' 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aKioAAAAASUVORK5CYII=' | base64 -d > "$work/activity/icon.png"
cat > "$work/activity/main.js" <<'JS'
import { connect } from "/_stellarr/sdk/v1.js";
const session = await connect({ initialState: { ready: true } });
const releaseCheck = { mode: session.context.launchMode };
for (const [name, endpoint] of [["client.js", "v1.js"], ["media.js", "media.js"]]) {
  const response = await fetch("/_stellarr/sdk/" + endpoint);
  if (!response.ok) throw new Error(endpoint);
  const bytes = await crypto.subtle.digest("SHA-256", await response.arrayBuffer());
  releaseCheck[name] = Array.from(new Uint8Array(bytes), byte => byte.toString(16).padStart(2, "0")).join("");
}
parent.postMessage({ releaseCheck }, "https://localhost:18743");
JS
"$work/author/node_modules/.bin/stellarr" pack --dir "$work/activity" --out "$work/release"
data=$(docker volume inspect stellarr_stellarr_data --format '{{.Mountpoint}}')
sudo python3 scripts/release/seed-activity.py "$work/release" "$data/db/stellarr.db"
node scripts/release/test-activity.mjs "$origin" "$work/author/node_modules/@stellarr/sdk"
