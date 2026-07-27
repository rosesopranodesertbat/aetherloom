#!/usr/bin/env bash
set -euo pipefail

release_sha="${1:-}"
if [[ ! "${release_sha}" =~ ^[0-9a-f]{40}$ ]]; then
  echo "release SHA must be a full lowercase 40-character Git SHA" >&2
  exit 1
fi

git fetch --no-tags origin main
resolved_sha="$(git rev-parse "${release_sha}^{commit}")"
if [[ "${resolved_sha}" != "${release_sha}" ]]; then
  echo "release SHA does not resolve to the requested commit" >&2
  exit 1
fi
if ! git merge-base --is-ancestor "${release_sha}" origin/main; then
  echo "production releases must already be present on origin/main" >&2
  exit 1
fi

echo "verified release commit ${release_sha} on origin/main"
