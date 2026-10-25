#!/usr/bin/env bash
#
# Install the pinned gitleaks release on a Linux x64 CI runner, checksum-verified
# (personal-cfo-fkt5.7; shared by the Security scanning job and the daily secrets audit,
# personal-cfo-md8h2.22). gitleaks is a Go binary with no taiki-e/install-action manifest, so it
# comes from its own GitHub release; the checksum is the trust mechanism (there is no source to
# pin a commit of). To upgrade: change BOTH values from the release's published SHA256SUMS.
set -euo pipefail

version=8.30.1
checksum=551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb
asset="gitleaks_${version}_linux_x64.tar.gz"

work="$(mktemp -d)"
cd "$work"
curl -sSLO "https://github.com/gitleaks/gitleaks/releases/download/v${version}/${asset}"
echo "${checksum}  ${asset}" | sha256sum -c -
tar -xzf "$asset" gitleaks
sudo install -m 0755 gitleaks /usr/local/bin/gitleaks
gitleaks version
