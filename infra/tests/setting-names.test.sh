#!/usr/bin/env bash
# Fails when the STELLARR_* settings an operator can set are not the same set
# everywhere: what the image reads (the API's table, and what the entrypoint,
# its shared settings script and the TLS script read), what docker-compose.yml
# interpolates, the commented-out lines of .env.example and the tables of
# docs/install/configuration.md. Needs bash, grep, sed, sort and comm.
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

# Names the image reads that are no operator's setting:
#   STELLARR_DATA_ROOT  the image's own data path, set by the Dockerfile
#   STELLARR_API_URL    the operator CLI's own, described in docs/admin/cli.md
exceptions=(STELLARR_DATA_ROOT STELLARR_API_URL)

failed=0
problem() {
  echo "FAIL: $*" >&2
  failed=1
}

only() { grep -oE 'STELLARR_[A-Z0-9_]+' | sort -u; }

# The table: one row per line, the name first.
table=$(grep -oE 'new\("STELLARR_[A-Z0-9_]+"' server/Stellarr.Api/Operations/StellarrVariables.cs | only)
# A read is ${NAME...}; the names the entrypoint only exports to the media
# server never appear so.
scripts=$(grep -ohE '\$\{STELLARR_[A-Z0-9_]+' infra/entrypoint.sh infra/settings.sh infra/tls-maintenance.sh | only)
compose=$(grep -oE '\$\{STELLARR_[A-Z0-9_]+' docker-compose.yml | only)
example_lines=$(grep -oE '^# STELLARR_[A-Z0-9_]+=' .env.example | sed 's/^# //; s/=$//')
example=$(only <<<"$example_lines")
# A table row starts with the name in backticks. The development stack's
# section lists that stack's own variables.
docs=$(sed '/^## Development stack/,$d' docs/install/configuration.md | grep -oE '^\| .STELLARR_[A-Z0-9_]+. ' | only)

for name in "${exceptions[@]}"; do
  grep -qxF "$name" <<<"$table"$'\n'"$scripts" || problem "the exception $name is read nowhere; drop it."
done
read_by_image=$(printf '%s\n%s\n' "$table" "$scripts" | grep -vxF "$(printf '%s\n' "${exceptions[@]}")" | sort -u)
expected=$(printf '%s\n%s\n' "$read_by_image" "$compose" | sort -u)

# Prints the names in $2 but not in $3 under the heading $1.
differ() {
  local missing
  missing=$(comm -23 <(printf '%s\n' "$2") <(printf '%s\n' "$3") | grep . || true)
  [[ -z $missing ]] || problem "$1: $(tr '\n' ' ' <<<"$missing")"
}

differ "read by the image or Compose, missing from .env.example" "$expected" "$example"
differ "in .env.example, read by neither the image nor Compose" "$example" "$expected"
differ "read by the image or Compose, missing from the tables of docs/install/configuration.md" "$expected" "$docs"
differ "in docs/install/configuration.md, read by neither the image nor Compose" "$docs" "$expected"

duplicates=$(sort <<<"$example_lines" | uniq -d)
[[ -z $duplicates ]] || problem ".env.example sets these more than once: $(tr '\n' ' ' <<<"$duplicates")"
# The installer appends .env.example to the .env it writes, so a line there
# that is not a comment would be a setting.
if grep -nE '^[[:space:]]*[A-Za-z_][A-Za-z0-9_]*=' .env.example >&2; then
  problem ".env.example assigns the lines above; every line must be a comment."
fi
if grep -nE '^#[[:space:]]*[A-Z][A-Za-z]*__[A-Za-z]' .env.example >&2; then
  problem ".env.example names a raw .NET key on the lines above."
fi

if (( failed )); then
  exit 1
fi
echo "PASS: $(wc -l <<<"$expected" | tr -d ' ') settings, the same in the image, Compose, .env.example and the docs."
