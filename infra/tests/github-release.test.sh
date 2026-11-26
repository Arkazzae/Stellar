#!/usr/bin/env bash
# Runs scripts/release/github-release.sh against stub curl and gh, so no
# network or GitHub account is needed. The stubs record every call and answer
# from files each case writes. Each case checks the release commands the
# script runs, or that it stops before any of them.
#
#   bash infra/tests/github-release.test.sh
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script="$repo_root/scripts/release/github-release.sh"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/stellarr-github-release-test.XXXXXX")"
trap 'rm -rf -- "$scratch"' EXIT
targets=(
  stellarr-installer-aarch64-apple-darwin
  stellarr-installer-aarch64-unknown-linux-musl
  stellarr-installer-x86_64-apple-darwin
  stellarr-installer-x86_64-unknown-linux-musl
)

# curl answers https://stellarr.online/server-downloads/PATH from $SERVED/PATH.
bin="$scratch/bin"
mkdir "$bin"
cat >"$bin/curl" <<'EOF'
#!/usr/bin/env bash
printf 'curl %s\n' "$*" >>"$STUB_LOG"
out='' url=''
while [[ $# -gt 0 ]]; do
  case $1 in
    -o) out=$2; shift ;;
    https://*) url=$1 ;;
  esac
  shift
done
file=$SERVED/${url#https://stellarr.online/server-downloads/}
[[ $url == https://stellarr.online/server-downloads/* && -f $file ]] || exit 22
if [[ -n $out ]]; then cp "$file" "$out"; else cat "$file"; fi
EOF
# gh knows the tags in $STATE/tags, the installer sums in $STATE/attested, and
# that the release exists when $STATE/release does.
cat >"$bin/gh" <<'EOF'
#!/usr/bin/env bash
printf 'gh %s\n' "$*" >>"$STUB_LOG"
case "$1 $2" in
  "api repos/"*/git/ref/tags/*)
    tag=${2##*/}
    grep -qx "$tag" "$STATE/tags" || { echo 'gh: Not Found (HTTP 404)' >&2; exit 1; }
    echo "refs/tags/$tag"
    ;;
  "api --paginate") sed 's|^|refs/tags/|' "$STATE/tags" ;;
  "api -X")
    previous=none
    for arg; do case $arg in previous_tag_name=*) previous=${arg#*=} ;; esac; done
    echo "Generated notes since $previous"
    ;;
  "attestation verify")
    grep -qx "$(sha256sum <"$3" | cut -d ' ' -f 1)" "$STATE/attested" ||
      { echo 'Error: no matching attestations found' >&2; exit 1; }
    echo 'https://github.com/Arkazzae/Stellarr/actions/runs/1/attempts/1'
    ;;
  "release view")
    if [[ -f $STATE/view-error ]]; then echo 'HTTP 502: Bad Gateway' >&2; exit 1; fi
    [[ -f $STATE/release ]] || { echo 'release not found' >&2; exit 1; }
    ;;
  "release create" | "release edit" | "release upload")
    while [[ $# -gt 0 ]]; do
      if [[ $1 == --notes-file ]]; then cp "$2" "$STATE/notes"; fi
      shift
    done
    ;;
  *) echo "unexpected gh $*" >&2; exit 1 ;;
esac
EOF
chmod +x "$bin/curl" "$bin/gh"

fail() {
  local file
  echo "FAIL: $case_name: $*" >&2
  for file in out calls; do
    if [[ -s $case_dir/$file ]]; then
      echo "--- $file" >&2
      cat "$case_dir/$file" >&2
    fi
  done
  exit 1
}

# Three server tags are out, and the stable channel serves 0.2.1.
new_case() {
  case_name=$1
  case_dir=$scratch/cases/$1
  destination=''
  SERVED=$case_dir/served STATE=$case_dir/state STUB_LOG=$case_dir/calls
  export SERVED STATE STUB_LOG
  mkdir -p "$SERVED" "$STATE"
  : >"$STUB_LOG"
  : >"$STATE/attested"
  printf '%s\n' server-v0.1.0 server-v0.2.0 server-v0.2.1 >"$STATE/tags"
  echo 0.2.1 >"$SERVED/latest"
}

# Serves the installers of $1 and their sums, and attests the installers.
publish() {
  local dir=$SERVED/$1 name
  mkdir -p "$dir"
  for name in "${targets[@]}"; do
    printf 'installer %s for %s\n' "$1" "$name" >"$dir/$name"
    sha256sum <"$dir/$name" | cut -d ' ' -f 1 >>"$STATE/attested"
  done
  (cd "$dir" && sha256sum "${targets[@]}" >stellarr-installer.sha256)
}

run() {
  status=0
  GITHUB_REPOSITORY=${destination:-} RUNNER_TEMP='' TMPDIR=$case_dir PATH="$bin:$PATH" \
    bash "$script" "$@" >"$case_dir/out" 2>&1 || status=$?
}

expect_success() { [[ $status -eq 0 ]] || fail "expected success, got status $status."; }
expect_failure() { [[ $status -ne 0 ]] || fail "expected a failure."; }
expect_called() { grep -Fq -- "$1" "$STUB_LOG" || fail "expected a call with: $1"; }
expect_not_called() { ! grep -Fq -- "$1" "$STUB_LOG" || fail "unexpected call with: $1"; }
expect_shown() { grep -Fq -- "$1" "$case_dir/out" || fail "expected on screen: $1"; }
expect_notes() { grep -Fq -- "$1" "$STATE/notes" || fail "expected in the notes: $1"; }
# The call that starts with $1 names every file the release carries.
expect_assets() {
  local call name
  call=$(grep -F -- "$1" "$STUB_LOG") || fail "expected a call with: $1"
  for name in "${targets[@]}" stellarr-installer.sha256; do
    [[ $call == *"/$name "* ]] || fail "$1 does not carry $name."
  done
}
expect_unchanged() {
  expect_not_called 'gh release create'
  expect_not_called 'gh release edit'
  expect_not_called 'gh release upload'
  [[ ! -e $STATE/notes ]] || fail "release notes were written."
}
expect_workdir_removed() {
  [[ -z $(find "$case_dir" -maxdepth 1 -name 'stellarr-github-release.*') ]] || fail "the download directory was left behind."
}

new_case create-latest
publish 0.2.1
run 0.2.1
expect_success
expect_assets 'gh release create server-v0.2.1 '
expect_called '--repo Arkazzae/Stellarr --verify-tag --title Stellarr 0.2.1 '
expect_called '--latest=true --prerelease=false'
[[ $(grep -c 'gh attestation verify .* --repo Arkazzae/Stellarr --signer-workflow Arkazzae/Stellarr/.github/workflows/release.yml --source-ref refs/tags/server-v0.2.1 ' "$STUB_LOG") -eq 4 ]] ||
  fail "expected four attestation checks against the release workflow for server-v0.2.1."
expect_shown 'Attestation verified: stellarr-installer-x86_64-unknown-linux-musl, built by https://github.com/Arkazzae/Stellarr/actions/runs/1/attempts/1'
expect_called 'previous_tag_name=server-v0.2.0'
expect_notes 'curl -fsSL https://raw.githubusercontent.com/Arkazzae/Stellarr/main/install.sh | bash'
expect_notes "Server image: \`ghcr.io/arkazzae/stellarr:0.2.1\`"
expect_notes '--source-ref refs/tags/server-v0.2.1'
expect_notes 'Generated notes since server-v0.2.0'
expect_workdir_removed

new_case older-version-is-not-latest
publish 0.2.0
run 0.2.0
expect_success
expect_called 'gh release create server-v0.2.0 '
expect_called '--latest=false --prerelease=false'
expect_called 'previous_tag_name=server-v0.1.0'

new_case prerelease
echo server-v0.3.0-rc.1 >>"$STATE/tags"
publish 0.3.0-rc.1
run 0.3.0-rc.1
expect_success
expect_called '--title Stellarr 0.3.0-rc.1 '
expect_called '--latest=false --prerelease=true'
expect_called 'previous_tag_name=server-v0.2.1'

new_case first-release
echo server-v0.1.0 >"$STATE/tags"
echo 0.1.0 >"$SERVED/latest"
publish 0.1.0
run 0.1.0
expect_success
expect_called '--latest=true --prerelease=false'
expect_not_called 'previous_tag_name'
expect_notes 'Generated notes since none'

new_case rerun-replaces-assets
publish 0.2.1
touch "$STATE/release"
run 0.2.1
expect_success
expect_not_called 'gh release create'
expect_assets 'gh release upload server-v0.2.1 '
expect_called ' --repo Arkazzae/Stellarr --clobber'
expect_called 'gh release edit server-v0.2.1 --repo Arkazzae/Stellarr --verify-tag --draft=false --title Stellarr 0.2.1 '
[[ $(grep -n 'gh release upload' "$STUB_LOG" | cut -d : -f 1) -lt $(grep -n 'gh release edit' "$STUB_LOG" | cut -d : -f 1) ]] ||
  fail "the assets must be in place before the release is edited."

# In a workflow the release goes to the repository it runs in, by its current
# name; the attestations are still checked against the name they were made under.
new_case renamed-repository
publish 0.2.1
destination=Arkazzae/Renamed
run 0.2.1
expect_success
expect_called 'gh api repos/Arkazzae/Renamed/git/ref/tags/server-v0.2.1 '
expect_called 'repos/Arkazzae/Renamed/releases/generate-notes'
expect_called 'gh release view server-v0.2.1 --repo Arkazzae/Renamed '
expect_called '--repo Arkazzae/Renamed --verify-tag --title Stellarr 0.2.1 '
[[ $(grep -c 'gh attestation verify .* --repo Arkazzae/Stellarr --signer-workflow Arkazzae/Stellarr/.github/workflows/release.yml ' "$STUB_LOG") -eq 4 ]] ||
  fail "the attestations were not checked against Arkazzae/Stellarr."
expect_notes 'gh attestation verify stellarr-installer-x86_64-unknown-linux-musl --repo Arkazzae/Stellarr '
expect_notes '[release workflow](https://github.com/Arkazzae/Renamed/blob/server-v0.2.1/.github/workflows/release.yml)'

new_case dry-run
publish 0.2.1
run --dry-run 0.2.1
expect_success
expect_called 'gh attestation verify'
expect_unchanged
expect_shown 'Dry run, would run: gh release create server-v0.2.1 '
expect_shown '--title Stellarr\ 0.2.1 '
expect_shown '--latest=true --prerelease=false'
expect_shown '  | curl -fsSL https://raw.githubusercontent.com/Arkazzae/Stellarr/main/install.sh | bash'
expect_workdir_removed

new_case corrupted-installer
publish 0.2.1
printf 'x' >>"$SERVED/0.2.1/stellarr-installer-x86_64-unknown-linux-musl"
run 0.2.1
expect_failure
expect_shown 'stellarr-installer-x86_64-unknown-linux-musl for 0.2.1 does not match its published SHA-256 sum'
expect_not_called 'gh attestation verify'
expect_unchanged
expect_workdir_removed

new_case corrupted-sums
publish 0.2.1
sed 's/^[0-9a-f]\{8\}/00000000/' "$SERVED/0.2.1/stellarr-installer.sha256" >"$case_dir/sums"
mv "$case_dir/sums" "$SERVED/0.2.1/stellarr-installer.sha256"
run 0.2.1
expect_failure
expect_shown 'does not match its published SHA-256 sum'
expect_unchanged

for change in truncated extra-line windows-lines; do
  new_case "sums-$change"
  publish 0.2.1
  sums=$SERVED/0.2.1/stellarr-installer.sha256
  case $change in
    truncated) sed '$d' "$sums" >"$case_dir/sums" ;;
    extra-line) { cat "$sums"; sed -n '1s/  .*/  stellarr-installer-riscv64-unknown-linux-musl/p' "$sums"; } >"$case_dir/sums" ;;
    windows-lines) sed 's/$/\r/' "$sums" >"$case_dir/sums" ;;
  esac
  mv "$case_dir/sums" "$sums"
  run 0.2.1
  expect_failure
  expect_shown 'stellarr-installer.sha256 for 0.2.1 does not list exactly the four installers'
  expect_unchanged
done

# A channel that serves other installers with matching sums still fails:
# they carry no attestation from the release workflow.
new_case unattested-installer
publish 0.2.1
printf 'another build\n' >"$SERVED/0.2.1/stellarr-installer-aarch64-apple-darwin"
(cd "$SERVED/0.2.1" && sha256sum "${targets[@]}" >stellarr-installer.sha256)
run 0.2.1
expect_failure
expect_shown 'stellarr-installer-aarch64-apple-darwin for 0.2.1 has no attestation from the release workflow for server-v0.2.1'
expect_unchanged

new_case missing-tag
publish 0.2.2
run 0.2.2
expect_failure
expect_shown 'The tag server-v0.2.2 does not exist'
expect_not_called 'curl'
expect_unchanged

for version in 01.2.3 0.2 0.2.1+build 0.2.1-rc.01 '0.2.1;id' server-v0.2.1; do
  new_case "invalid-version-$version"
  run "$version"
  expect_failure
  expect_not_called 'gh '
  expect_not_called 'curl'
done

new_case not-published
echo server-v0.2.2 >>"$STATE/tags"
run 0.2.2
expect_failure
expect_shown 'Could not download stellarr-installer-aarch64-apple-darwin for 0.2.2'
expect_unchanged

new_case stable-channel-unreachable
publish 0.2.1
rm "$SERVED/latest"
run 0.2.1
expect_failure
expect_shown 'Could not read the stable channel'
expect_unchanged

new_case release-lookup-error
publish 0.2.1
touch "$STATE/view-error"
run 0.2.1
expect_failure
expect_shown 'Could not look up the release for server-v0.2.1'
expect_unchanged

echo "github-release: all cases passed."
