#!/usr/bin/env bash
set -euo pipefail

# Snapshot daily jsDelivr CDN hits per data file into a growing local archive.
#
# The app has no telemetry. It fetches the v2 benchmark source files fresh from
# jsDelivr on every launch, so per-file daily CDN hits are the closest proxy we
# have to daily app launches (`/data/v2/aa.json` is the cleanest per-launch
# proxy now that v2 is the live path; `/data/benchmarks.json` is still fetched
# by older released binaries).
#
# jsDelivr's API only serves a rolling ~30-day daily window per file, so we
# upsert each run into data/stats/jsdelivr-history.json to retain history
# indefinitely. The merge is self-healing: every run re-pulls the trailing
# ~30 days, so a dropped scheduled run loses nothing as long as we snapshot at
# least every ~25 days (the daily cron leaves a wide margin).
#
# Caveat: jsDelivr per-package stats are aggregate-only (no per-IP / country /
# user-agent breakdown), so these counts include the maintainer's own launches.
# See data/stats/README.md for how to subtract self-traffic.

OUT="data/stats/jsdelivr-history.json"
# The /files sub-resource requires a version ref; @main is the branch the app
# fetches, so its per-file hits are exactly the app-launch traffic we want.
API="https://data.jsdelivr.com/v1/stats/packages/gh/reyamira/models@main/files?period=month"

mkdir -p data/stats

raw=$(mktemp)
trap 'rm -f "$raw"' EXIT
curl -sf -H "User-Agent: models-stats-snapshot" "$API" > "$raw"

# Pivot the per-file daily series into { "date": { "/path": hits } }, keeping
# only files under /data/ (the app's data lane; this auto-includes any future
# self-traffic sentinel committed under data/stats/).
new_days=$(jq '
  [ .[] | select(.name | startswith("/data/")) ] as $files
  | reduce $files[] as $f ({};
      reduce ($f.hits.dates | to_entries[]) as $d (.;
        .[$d.key][$f.name] = $d.value ) )
' "$raw")

# Existing archive, or an empty scaffold on first run.
if [ -f "$OUT" ]; then
  existing=$(cat "$OUT")
else
  existing='{"days":{}}'
fi

note="Daily jsDelivr CDN hits per data file = proxy for app launches (the app fetches v2 sources fresh from jsDelivr on every run; it has no telemetry). Recent days keep revising as the rolling window updates. Counts include the maintainer's own launches — see data/stats/README.md to subtract self-traffic."

# Merge: API days override stored days (recent counts revise upward through the
# day); days older than the API window are preserved. Sort keys for a stable,
# reviewable diff so re-runs with no new data produce no commit.
echo "$existing" | jq --argjson new "$new_days" --arg note "$note" '
  { note: $note,
    days: ( (.days // {}) + $new
            | to_entries | sort_by(.key) | from_entries ) }
' > "$OUT"

days=$(jq '.days | length' "$OUT")
latest=$(jq -r '.days | keys | last' "$OUT")
echo "Snapshotted jsDelivr stats -> $OUT ($days days, latest $latest)"
