# shellcheck shell=bash
# Sourced by install.test.sh, which supplies the isolated installer harness.
case_release_choices() {
  new_case release-series
  local release_list="$http/api.github.com/repos/Arkazzae/Stellarr/releases"
  mkdir -p "${release_list%/*}"
  cat >"$release_list" <<'JSON'
[
  {"tag_name":"v1.10.0","prerelease":false,"draft":false},
  {"tag_name":"v1.4.8","prerelease":false,"draft":true},
  {"tag_name":"v1.4.9-rc.1","prerelease":true,"draft":false},
  {"tag_name":"v1.4.0","prerelease":false,"draft":false},
  {"tag_name":"v1.4.3","prerelease":false,"draft":false}
]
JSON
  mkdir -p "$raw/v1.4.3"
  cp "$fixtures/compose-1.4.0" "$raw/v1.4.3/docker-compose.yml"
  cp "$fixtures/env.example" "$raw/v1.4.3/.env.example"
  release 1.5.0
  run --yes --version 1.4
  expect_success
  expect_line "$home/stellarr/.env" STELLARR_VERSION=1.4.3
  expect_line "$home/stellarr/.env" STELLARR_TRACK=1.4
  expect_called "/v1.4.3/docker-compose.yml"
  expect_called "image inspect ghcr.io/arkazzae/stellarr:1.4.3"
  expect_not_called "/v1.5.0/docker-compose.yml"

  # In the absence of a new choice, re-running follows the stored series.
  : >"$case_dir/calls"
  run --yes --no-start
  expect_success
  expect_line "$home/stellarr/.env" STELLARR_VERSION=1.4.3
  expect_called "api.github.com/repos/Arkazzae/Stellarr/releases"
  expect_not_called "github.com/Arkazzae/Stellarr/releases/latest"

  new_case release-latest
  release 1.5.0
  run --yes --no-start --version latest
  expect_success
  expect_line "$home/stellarr/.env" STELLARR_VERSION=1.5.0
  expect_line "$home/stellarr/.env" STELLARR_TRACK=latest

  new_case release-explicit-ref
  mkdir -p "$raw/feature/compose"
  cp "$fixtures/compose-1.5.0" "$raw/feature/compose/docker-compose.yml"
  cp "$fixtures/env.example" "$raw/feature/compose/.env.example"
  run --yes --version 1.4.0 --ref feature/compose
  expect_success
  expect_line "$home/stellarr/.env" STELLARR_VERSION=1.4.0
  expect_called "/feature/compose/docker-compose.yml"
  expect_called "image inspect ghcr.io/arkazzae/stellarr:1.4.0"
  expect_shown "Using Compose and settings files from feature/compose"

  new_case release-prerelease-downgrade
  installed "$home/stellarr" 1.4.0
  container "$home/stellarr" 1.5.0-rc.10
  run --yes --version 1.5.0-rc.2 --ref main
  expect_failure
  expect_shown "1.5.0-rc.2 is older"
  expect_not_called "/main/docker-compose.yml"

  new_case release-custom-tag
  local dir="$home/stellarr"
  installed "$dir" 1.4.0
  sed 's/^STELLARR_VERSION=.*/STELLARR_VERSION=experimental/' "$dir/.env" >"$case_dir/env-before"
  cp "$case_dir/env-before" "$dir/.env"
  run --yes --no-start
  expect_failure
  expect_shown "experimental is not an exact release"
  expect_not_called "/releases/latest"
  cmp -s "$case_dir/env-before" "$dir/.env" || fail "a custom image tag was changed."

  new_case release-custom-image
  dir="$home/stellarr"
  installed "$dir" 1.4.0
  printf '%s\n' 'STELLARR_IMAGE=localhost:5000/testing/stellarr:edge' >>"$dir/.env"
  cp "$dir/.env" "$case_dir/env-before"
  run --yes --no-start
  expect_failure
  expect_shown "The installation uses a custom image"
  expect_not_called "/releases/latest"
  cmp -s "$case_dir/env-before" "$dir/.env" || fail "a custom image was changed."
}
