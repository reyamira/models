# Benchmark Trigger Worker

Cloudflare Worker that invokes `workflow_dispatch` on
`.github/workflows/update-benchmarks.yml` from a cron trigger, as a fallback
for GitHub Actions cron throttling.

**Status: scaffolded, not deployed.** Activate only if the offset cron in
`update-benchmarks.yml` (`13,43 * * * *`) does not recover enough slots.

## Why this exists

GitHub Actions deprioritizes scheduled workflows under load. Public repos on
half-hour boundaries see roughly half of scheduled slots dropped and overnight
gaps of 3–4 hours. Cloudflare Workers cron triggers fire reliably
(sub-minute precision), so we can use one to invoke `workflow_dispatch` from
outside the GitHub Actions scheduler.

The workflow is idempotent — `git diff --quiet data/benchmarks.json && exit 0`
short-circuits if no data changed — so duplicate invocations (e.g., GH cron
plus this worker firing in the same hour) cost an API call and nothing else.

## When to activate

After the offset-cron change (`13,43 * * * *`) has been on `main` for ~1 week,
inspect run history:

```bash
gh run list --workflow=update-benchmarks.yml --limit 50 \
  --json createdAt,conclusion,event
```

If gaps over 2 hours still appear regularly (especially overnight), activate
this worker. If runs land within ~1 hour of every cron slot, leave dormant.

## Activate

1. **Generate a fine-grained GitHub PAT** scoped to `reyamira/models`:
   - Repository access: only `reyamira/models`
   - Permission: **Actions → Read and write**
   - Expiration: 1 year (set a calendar reminder to rotate)

2. **Install deps and set the secret:**
   ```bash
   cd infra/benchmark-trigger
   bun install
   bun wrangler login
   bun wrangler secret put GH_DISPATCH_TOKEN
   # paste the PAT when prompted
   ```

3. **Uncomment** the `[triggers]` block in `wrangler.toml`. The default cron
   fires hourly at `:17`, complementary to the GitHub Actions slots at
   `:13` / `:43`.

4. **Deploy:**
   ```bash
   bun wrangler deploy
   ```

5. **Verify:** within ~1 hour, `gh run list --workflow=update-benchmarks.yml`
   should show entries with `event: workflow_dispatch` alongside the
   `event: schedule` entries.

## Deactivate

Either:

```bash
cd infra/benchmark-trigger
bun wrangler delete
```

or comment out the `[triggers]` block and redeploy (keeps the worker but
unschedules it).

## Cost

Cloudflare Workers Free tier covers 100k requests/day. Hourly invocations are
~720/month — well under the free limit.
