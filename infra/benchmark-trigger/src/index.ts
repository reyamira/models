export interface Env {
  GH_DISPATCH_TOKEN: string;
}

const REPO_OWNER = "reyamira";
const REPO_NAME = "models";
const WORKFLOW_FILE = "update-benchmarks.yml";
const REF = "main";

async function triggerWorkflow(token: string): Promise<{ ok: boolean; status: number; body: string }> {
  const url = `https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}/actions/workflows/${WORKFLOW_FILE}/dispatches`;
  const response = await fetch(url, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
      "User-Agent": "models-benchmark-trigger",
    },
    body: JSON.stringify({ ref: REF }),
  });
  const body = response.ok ? "" : await response.text();
  return { ok: response.ok, status: response.status, body };
}

export default {
  async scheduled(event: ScheduledController, env: Env, _ctx: ExecutionContext): Promise<void> {
    const result = await triggerWorkflow(env.GH_DISPATCH_TOKEN);
    if (!result.ok) {
      console.error(`Workflow dispatch failed: ${result.status} ${result.body}`);
      throw new Error(`Workflow dispatch failed with status ${result.status}`);
    }
    console.log(JSON.stringify({ msg: "workflow dispatched", cron: event.cron, scheduledTime: event.scheduledTime }));
  },
} satisfies ExportedHandler<Env>;
