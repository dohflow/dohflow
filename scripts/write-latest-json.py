#!/usr/bin/env python3
"""Write the updater's latest.json for a release build (called by scripts/release.sh).

Usage: write-latest-json.py <version> <notes> <updater-sig-path> <out-path> <tauri.conf.json>

Kept as its own file so scripts/tests/write-latest-json.test.sh can check the
manifest without a signed build (personal-cfo-md8h2.4).
"""
import json, sys
version, notes, sig_path, out, conf = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5]
# The bundle's macOS floor, read from tauri.conf.json (never hard-coded). An installed
# app compares it with the running macOS and declines an update it couldn't open
# (personal-cfo-md8h2.4, ADR 0065 addendum 2026-10-17).
min_os = json.load(open(conf)).get('bundle', {}).get('macOS', {}).get('minimumSystemVersion')
if not min_os:
    sys.exit('tauri.conf.json has no bundle.macOS.minimumSystemVersion; refusing to write latest.json without it')
# The 'url' below is a PLACEHOLDER — the real asset URL only exists once this build's
# artifacts are uploaded to a GitHub release. Fill it in (or pass --smoke-endpoint's
# matching upload target) before publishing; release-checklist.md (867.1.3) names the
# exact step.
manifest = {
    'version': version,
    'notes': notes,
    'pub_date': __import__('datetime').datetime.now(__import__('datetime').timezone.utc).isoformat(timespec='seconds').replace('+00:00', 'Z'),
    'minimumSystemVersion': min_os,
    'platforms': {
        'darwin-aarch64': {
            'url': 'REPLACE_WITH_THE_UPLOADED_APP_TAR_GZ_ASSET_URL',
            'signature': open(sig_path).read(),
        },
        'darwin-x86_64': {
            'url': 'REPLACE_WITH_THE_UPLOADED_APP_TAR_GZ_ASSET_URL',
            'signature': open(sig_path).read(),
        },
    },
}
json.dump(manifest, open(out, 'w'), indent=2)
