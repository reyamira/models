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
    "2026-08-12": {
      "/data/v2/aa.json": 107,        // contaminated — do NOT use, see below
      "/data/v2/arena.json": 83,      // }
      "/data/v2/epoch.json": 85,      // }- read the MEDIAN of these three
      "/data/v2/llmstats.json": 84,   // }
      "/data/benchmarks.json": 47     // retired v1 lane — pre-v2 binaries
    }
  }
}
```

- Each launch fetches **all four** v2 sources, so any one of them approximates
  launches. Read the **median of `arena` / `epoch` / `llmstats`** — one file can
  pick up non-launch traffic on its own (see below), and a median of three
  survives that happening again.
- `data/benchmarks.json` is the retired v1 lane, fetched only by **pre-v2
  released binaries**. See the sunset note in the project `CLAUDE.md`.
- Spikes around a release date or a `data/v2/*` commit are mostly the
  **data-bot + jsDelivr purge**, not humans — discount them. A purge spike moves
  all four v2 files in lockstep; that lockstep is what distinguishes it from the
  single-file case below.

## `aa.json` is contaminated — don't use it as the proxy

Until 2026-08-14 this file recommended `/data/v2/aa.json` as "the most stable
reference." That is now backwards. Since early July, something external polls
aa.json in isolated bursts while the other three v2 files stay flat:

| day | aa | arena | ratio |
|---|---|---|---|
| 2026-07-30 | 3614 | 104 | 34.8× |
| 2026-08-02 | 1298 | 93 | 14.0× |
| 2026-07-31 | 826 | 81 | 10.2× |
| 2026-07-03 | 436 | 116 | 3.8× |

Median aa/arena ratio across 90 days is 1.05, but 13 days exceed 1.25× and four
exceed 3×; 87% of the ~6,600 excess hits fall on those four days. It is **not**
maintainer traffic — mean ratio on days with local commits (2.09) is
indistinguishable from quiet days (2.12), and the four biggest spikes landed on
days with zero commits. `benchmarks.json` stayed flat throughout, so it is not a
bot/purge artifact either (those move every file at once).

The likely explanation is a downstream consumer treating aa.json as a public
dataset — it is the richest of the four and refreshes every 30 minutes. jsDelivr
stats are aggregate-only, so this is not attributable and does not need to be:
just read a file that isn't being scraped.

Note the in-app `r` refresh re-fetches **only the active source**, which defaults
to AA. That is a second, first-party reason aa.json runs hotter than the rest.

## Why a snapshot (vs. just querying the API)

jsDelivr only serves a **rolling ~30-day** daily window per file. The snapshot
**upserts** the trailing 30 days into this archive on each run, so:

- History is retained **indefinitely**.
- The job is **self-healing**: a missed scheduled run loses nothing as long as we
  snapshot at least every ~25 days (the daily cron leaves a wide margin).

**Stats lag ~2 days.** jsDelivr's daily numbers trail real time by roughly two
days, and the most recent day or two keep **revising upward** as the window
fills. So the latest day in this archive is always partial — read trends from
days that are 2+ days old.

## What this archive cannot tell you

- **Launches, not people.** One user launching four times and four users
  launching once are identical here. No uniques, no retention, no new-vs-
  returning.
- **Not your own traffic, separately.** jsDelivr per-package stats are
  aggregate-only — no per-IP, country, or user-agent breakdown — so these counts
  include the maintainer's own launches with no way to subtract them.

### The self-ping sentinel (retired 2026-08-14)

A `data/stats/self-ping` file used to be fetched by a shell wrapper on the
maintainer's machine, so that `other users ≈ aa.json − self-ping` could estimate
external launches. It was retired because both halves were unsound: the minuend
was the contaminated aa.json, and the sentinel only fired for launches made
through the wrapped interactive `models` command — development launches via
`cargo run` or `./target/release/models` (including the release visuals pass)
fetch the data files without pinging, so the maintainer's own traffic counted as
users. A separate question — whether jsDelivr counts repeated identical sentinel
requests as distinct hits at all — was never resolved.

The sentinel file and the shell wrapper that fetched it are both gone. The
`self-ping` rows already in `jsdelivr-history.json` are kept as historical data —
they are not a metric, so don't build on them. Any future attempt at this needs
the sentinel fired from the same code path as the launch it marks, not from a
shell alias.
