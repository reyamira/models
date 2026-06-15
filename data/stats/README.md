# CDN stats — app-launch proxy

The app has **no telemetry**. It fetches the v2 benchmark source files fresh from
jsDelivr on every launch, so **per-file daily CDN hits are the closest proxy we
have to daily app launches**.

`jsdelivr-history.json` is a growing archive of those daily hits, written by
`scripts/snapshot-stats.sh` (also `mise run snapshot-stats`, and the
`Snapshot CDN Stats` GitHub Action on a daily schedule).

## How to read it

```json
{
  "note": "...",
  "days": {
    "2026-06-13": {
      "/data/v2/aa.json": 37,        // cleanest per-launch proxy (post-v2)
      "/data/v2/arena.json": 36,
      "/data/v2/epoch.json": 35,
      "/data/v2/llmstats.json": 32,
      "/data/benchmarks.json": 413   // legacy lane — older released binaries
    }
  }
}
```

- Each launch fetches **all four** v2 sources, so any one of `aa/arena/epoch/llmstats`
  approximates launches. `aa.json` is the most stable reference.
- `data/benchmarks.json` is fetched only by **pre-v2 released binaries** (frozen
  legacy lane). It will decay as users upgrade.
- Spikes around a release date or a `data/v2/*` commit are mostly the
  **data-bot + jsDelivr purge**, not humans — discount them.

## Why a snapshot (vs. just querying the API)

jsDelivr only serves a **rolling ~30-day** daily window per file. The snapshot
**upserts** the trailing 30 days into this archive on each run, so:

- History is retained **indefinitely**.
- The job is **self-healing**: a missed scheduled run loses nothing as long as we
  snapshot at least every ~25 days (the daily cron leaves a wide margin).

**Stats lag ~2 days.** jsDelivr's daily numbers trail real time by roughly two
days, and the most recent day or two keep **revising upward** as the window
fills. So the latest day in this archive is always partial — read trends from
days that are 2+ days old, and don't conclude "the ping isn't working" by
checking minutes after a launch.

## Filtering maintainer (self) traffic

jsDelivr per-package stats are **aggregate-only** — no per-IP, country, or
user-agent breakdown — so these counts **include the maintainer's own launches**.
You cannot subtract yourself from the CDN data directly.

To get a clean "other users" signal **without leaving the fresh `@main` data
pool**, the snapshot also tracks any file committed under `data/stats/` (it
captures every `/data/` path automatically). The `data/stats/self-ping` sentinel
is fetched **only by the maintainer's own launches**, so it lands in its own
bucket:

```
other-user launches ≈ /data/v2/aa.json hits − data/stats/self-ping hits   (per day)
```

The ping is fired by a shell wrapper on the maintainer's machine
(`~/.config/fish/functions/models.fish`): each `models` launch background-curls
the sentinel before starting the app. No app code ships, and data freshness is
untouched — the app still fetches the v2 sources from `@main` exactly as before.

Notes:

- The sentinel only counts launches made through the wrapped `models` command on
  machines where the wrapper is installed (re-add it on each dev machine).
- It starts counting only **after `self-ping` is live on `@main`** (committed +
  jsDelivr cache warm). Pings before that 404 harmlessly and are not counted.
- **The subtraction assumes ~1 `aa.json` fetch per launch.** The in-app `r`
  refresh re-fetches the active source, so a session with refreshes adds
  `aa.json` hits without matching pings → slight *under*-subtraction. Fine for a
  proxy; just don't read it as exact.
- **Pending verification:** this relies on jsDelivr counting each sentinel curl
  as a distinct hit (not edge-cache-deduping repeats). If a maintainer's repeated
  pings collapse to ~1/day in `/files`, switch the wrapper to a cache-busting
  query (`?_=<random>` — stats still attribute to `self-ping`, but each request
  is forced distinct). Verify before trusting the subtraction.
- Crudest fallback if the wrapper is ever absent: subtract your own rough daily
  launch count from the trend by hand.
