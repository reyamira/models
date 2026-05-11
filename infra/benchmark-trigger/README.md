# Benchmark Trigger Worker

Cloudflare Worker that invokes `workflow_dispatch` on
`.github/workflows/update-benchmarks.yml` from a cron trigger.

**Status: active and sole driver.** Activated 2026-05-11. Fires every 30
minutes at `:17` and `:47`. The GitHub Actions `schedule:` was removed
on the same day after proving unreliable — see git history for context.

## Why this exists

GitHub Actions deprioritizes scheduled workflows under load. Public repos on
half-hour boundaries saw roughly half of scheduled slots dropped and overnight
gaps of 3–4 hours; an offset cron at `13,43 * * * *` did not recover them.
Cloudflare Workers cron triggers fire reliably (sub-minute precision), so we
use one to invoke `workflow_dispatch` from outside the GitHub Actions
scheduler entirely.

The workflow is idempotent — `git diff --quiet data/benchmarks.json && exit 0`
short-circuits if no data changed — so any duplicate invocation (e.g., a
re-armed schedule plus this worker firing close together) costs an API call
and nothing else.

## Operations

Worker name: `models-benchmark-trigger`
Account: `ari111097@gmail.com`
Secret: `GH_DISPATCH_TOKEN` — fine-grained GitHub PAT scoped to `reyamira/models` with Actions: Read+Write. Rotate annually.

```bash
cd infra/benchmark-trigger

# Deploy after code or wrangler.toml changes
wrangler deploy

# Live-tail logs while waiting for the next fire
wrangler tail

# Rotate the PAT
wrangler secret put GH_DISPATCH_TOKEN
```

Verify a recent fire from the repo root:

```bash
curl -s "https://api.github.com/repos/reyamira/models/actions/workflows/update-benchmarks.yml/runs?per_page=5" \
  | jq -r '.workflow_runs[] | "\(.created_at)  event=\(.event)  conclusion=\(.conclusion)"'
```

Successful runs created within ~30 seconds of `:17` or `:47` with `event=workflow_dispatch` mean the worker is firing as expected.

## Deactivate

```bash
cd infra/benchmark-trigger
wrangler delete
```

or comment out the `[triggers]` block in `wrangler.toml` and `wrangler deploy` (keeps the worker but unschedules it).

## Cost

Cloudflare Workers Free tier covers 100k requests/day. Two crons per hour = ~1,440/month — well under the free limit.
