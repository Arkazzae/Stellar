#!/usr/bin/env bash
# Mirrors a published server release onto the repository's Releases page: the
# four native installers and their SHA-256 sums, downloaded from the address
# install.sh fetches them from, so the release carries the bytes people
# already install. Every installer is checked against the sums and against
# its build attestation before anything on GitHub changes; a release that
# exists already gets its assets replaced and its notes refreshed.
#
#   bash scripts/release/github-release.sh [--dry-run] VERSION
#
# VERSION is a server version such as 0.2.1, whose server-v tag must exist.
# --dry-run downloads and verifies, then prints the notes and the commands
# that would create or update the release. It needs curl, python3 and gh,
# signed in or given GH_TOKEN.
set -euo pipefail

# The repository whose release workflow attests the installers, by the name
# their attestations carry.
repository=Arkazzae/Stellarr
# The repository the release is written to: in a workflow, the one it runs in.
destination=${GITHUB_REPOSITORY:-$repository}
server_downloads=https://stellarr.online/server-downloads
image=ghcr.io/arkazzae/stellarr
targets=(
  stellarr-installer-aarch64-apple-darwin
  stellarr-installer-aarch64-unknown-linux-musl
  stellarr-installer-x86_64-apple-darwin
  stellarr-installer-x86_64-unknown-linux-musl
)
sums=stellarr-installer.sha256

fail() {
  echo "::error::$*" >&2
  exit 1
}

dry_run=false
if [[ ${1:-} == --dry-run ]]; then
  dry_run=true
  shift
fi
[[ $# -eq 1 && -n $1 ]] || fail "Usage: github-release.sh [--dry-run] VERSION"
version=$1
tag=server-v$version

# The rule release.yml applies to server-v* tags.
python3 -I - "$version" <<'PYTHON'
import re
import sys

tag = "v" + sys.argv[1]
match = re.fullmatch(
    r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?", tag
)
if match is None or len(tag) > 129 or any(int(value) >= 2**64 for value in match.groups()[:3]):
    raise SystemExit(f"Invalid release version: {sys.argv[1]}")
for identifier in tag.partition("-")[2].split("."):
    if identifier.isdecimal() and len(identifier) > 1 and identifier.startswith("0"):
        raise SystemExit(f"Invalid numeric prerelease identifier: {identifier}")
PYTHON

[[ $(gh api "repos/$destination/git/ref/tags/$tag" --jq .ref) == "refs/tags/$tag" ]] ||
  fail "The tag $tag does not exist in $destination"

work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/stellarr-github-release.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

download() {
  curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL \
    --connect-timeout 15 --max-time 300 --retry 3 --retry-delay 2 -o "$2" "$1"
}

sha256() {
  if command -v sha256sum >/dev/null; then
    sha256sum <"$1" | cut -d ' ' -f 1
  else
    shasum -a 256 <"$1" | cut -d ' ' -f 1
  fi
}

echo "Downloading server $version from $server_downloads/$version/"
files=()
for name in "${targets[@]}" "$sums"; do
  download "$server_downloads/$version/$name" "$work/$name" || fail "Could not download $name for $version"
  echo "  $name: $(wc -c <"$work/$name" | tr -d ' ') bytes"
  files+=("$work/$name")
done

# The sums file holds exactly one line per installer, as sha256sum writes it.
if grep -Evq '^[0-9a-f]{64}  stellarr-installer-[0-9a-z_-]+$' "$work/$sums" ||
  [[ $(awk '{ print $2 }' "$work/$sums" | sort) != "$(printf '%s\n' "${targets[@]}" | sort)" ]]; then
  fail "$sums for $version does not list exactly the four installers"
fi
for name in "${targets[@]}"; do
  actual=$(sha256 "$work/$name")
  [[ $(awk -v name="$name" '$2 == name { print $1 }' "$work/$sums") == "$actual" ]] ||
    fail "$name for $version does not match its published SHA-256 sum"
  echo "SHA-256 matches $sums: $name $actual"
done

# The installers' attestation is made by the release workflow run for this tag.
for name in "${targets[@]}"; do
  runs=$(gh attestation verify "$work/$name" --repo "$repository" \
    --signer-workflow "$repository/.github/workflows/release.yml" --source-ref "refs/tags/$tag" \
    --format json --jq '[.[].verificationResult.signature.certificate.runInvocationURI] | join(" ")') ||
    fail "$name for $version has no attestation from the release workflow for $tag"
  echo "Attestation verified: $name, built by $runs"
done

stable=$(curl --proto '=https' --tlsv1.2 -fsSL --retry 3 "$server_downloads/latest") ||
  fail "Could not read the stable channel at $server_downloads/latest"
[[ $stable =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "The stable channel serves an invalid version: $stable"
prerelease=false
[[ $version == *-* ]] && prerelease=true
latest=false
[[ $version == "$stable" ]] && latest=true
echo "The stable channel serves $stable; latest=$latest, prerelease=$prerelease"

# GitHub's notes cover the changes since the closest earlier stable server tag.
core=${version%%-*}
previous=$(
  {
    echo "$core"
    gh api --paginate "repos/$destination/git/matching-refs/tags/server-v" --jq '.[].ref' |
      sed -n 's|^refs/tags/server-v\([0-9]*\.[0-9]*\.[0-9]*\)$|\1|p'
  } | sort -u -t . -k 1,1n -k 2,2n -k 3,3n | awk -v core="$core" '$0 == core { print last } { last = $0 }'
)
generate=(-X POST "repos/$destination/releases/generate-notes" -f "tag_name=$tag")
[[ -n $previous ]] && generate+=(-f "previous_tag_name=server-v$previous")
generated=$(gh api "${generate[@]}" --jq .body)

notes=$work/notes.md
cat >"$notes" <<EOF
Install the latest release with:

\`\`\`bash
curl -fsSL https://raw.githubusercontent.com/Arkazzae/Stellarr/main/install.sh | bash
\`\`\`

Server image: \`$image:$version\`

The installers below are the files \`install.sh\` downloads from \`stellarr.online/server-downloads/$version/\`, built and attested by the [release workflow](https://github.com/$destination/blob/$tag/.github/workflows/release.yml). To verify one, check its SHA-256 sum (on macOS, \`shasum -a 256\` in place of \`sha256sum\`) and its build attestation:

\`\`\`bash
sha256sum --check --ignore-missing $sums
gh attestation verify stellarr-installer-x86_64-unknown-linux-musl --repo $repository \\
  --signer-workflow $repository/.github/workflows/release.yml --source-ref refs/tags/$tag
\`\`\`

$generated
EOF

# GitHub answers "release not found" for a tag without a release; any other
# failure to look one up ends the run.
if gh release view "$tag" --repo "$destination" --json tagName >/dev/null 2>"$work/view"; then
  exists=true
elif grep -q 'release not found' "$work/view"; then
  exists=false
else
  cat "$work/view" >&2
  fail "Could not look up the release for $tag"
fi

# Everything above only reads (generating the notes stores nothing); this is
# the one place the release changes.
run() {
  if [[ $dry_run == true ]]; then
    printf 'Dry run, would run:'
    printf ' %q' "$@"
    printf '\n'
  else
    "$@"
  fi
}
if [[ $dry_run == true ]]; then
  echo "Dry run, release notes:"
  sed 's/^/  | /' "$notes"
fi
release=(--title "Stellarr $version" --notes-file "$notes" --latest="$latest" --prerelease="$prerelease")
if [[ $exists == true ]]; then
  # Assets first, so a draft left by an interrupted run is complete when published.
  run gh release upload "$tag" "${files[@]}" --repo "$destination" --clobber
  run gh release edit "$tag" --repo "$destination" --verify-tag --draft=false "${release[@]}"
else
  run gh release create "$tag" "${files[@]}" --repo "$destination" --verify-tag "${release[@]}"
fi
