import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// Repo root is one level up from website/ (process.cwd() = website/ during build)
const REPO_ROOT = resolve(process.cwd(), "..");

function readRepoFile(path: string): string {
  return readFileSync(resolve(REPO_ROOT, path), "utf-8");
}

// --- Cargo.toml ---

const cargoToml = readRepoFile("Cargo.toml");

function parseCargoField(field: string): string {
  const match = cargoToml.match(new RegExp(`^${field}\\s*=\\s*"(.+)"`, "m"));
  return match?.[1] ?? "";
}

export const VERSION = parseCargoField("version");
export const CRATE_NAME = parseCargoField("name");
export const REPO_URL = parseCargoField("repository");
export const DESCRIPTION = parseCargoField("description");

// Derived URLs
export const WIKI_URL = `${REPO_URL}/wiki`;
export const RELEASES_URL = `${REPO_URL}/releases`;
export const LICENSE_URL = `${REPO_URL}/blob/main/LICENSE`;
export const CRATES_URL = `https://crates.io/crates/${CRATE_NAME}`;

// --- Local data files ---

// Sum of model rows across the four v2 benchmark sources (aa, epoch, arena,
// llmstats). The legacy data/benchmarks.json is the frozen single-source lane
// for released binaries — not representative of the tab anymore.
const BENCHMARK_SOURCE_IDS = ["aa", "epoch", "arena", "llmstats"] as const;
export const BENCHMARK_SOURCE_COUNT = BENCHMARK_SOURCE_IDS.length;
export const BENCHMARK_COUNT = BENCHMARK_SOURCE_IDS.reduce((total, id) => {
  const sourceFile = JSON.parse(readRepoFile(`data/v2/${id}.json`));
  return total + (sourceFile.models?.length ?? 0);
}, 0);

const agentsJson = JSON.parse(readRepoFile("data/agents.json"));
const agents = agentsJson.agents ?? {};
export const AGENT_COUNT = Array.isArray(agents)
  ? agents.length
  : Object.keys(agents).length;

const registryRs = readRepoFile("src/status/registry.rs");
export const STATUS_PROVIDER_COUNT = (
  registryRs.match(/RegistryEntry\s*\{/g) || []
).length;

// --- models.dev API (fetched at build time) ---

interface ModelsDevProvider {
  models?: unknown[];
  [key: string]: unknown;
}

let modelCount = 0;
let providerCount = 0;
let providerModelCounts: number[] = [];

try {
  const res = await fetch("https://models.dev/api.json");
  const data = (await res.json()) as Record<string, ModelsDevProvider>;
  providerCount = Object.keys(data).length;
  const counts: number[] = [];
  modelCount = Object.values(data).reduce((sum, provider) => {
    const models = provider.models;
    if (!models || typeof models !== "object") return sum;
    const n = Object.keys(models).length;
    if (n > 0) counts.push(n);
    return sum + n;
  }, 0);
  providerModelCounts = counts.sort((a, b) => b - a).slice(0, 24);
} catch {
  // Fallback if API is unreachable during build
  modelCount = 3800;
  providerCount = 100;
  providerModelCounts = [];
}

export const MODEL_COUNT = modelCount;
export const PROVIDER_COUNT = providerCount;
export const PROVIDER_MODEL_COUNTS: number[] = providerModelCounts;

// --- GitHub stars (fetched at build time via ungh.cc) ---

let starCount = 0;

try {
  const repoPath = REPO_URL.replace("https://github.com/", "");
  const res = await fetch(`https://ungh.cc/repos/${repoPath}`);
  const data = (await res.json()) as { repo?: { stars?: number } };
  starCount = data.repo?.stars ?? 0;
} catch {
  // Fallback if ungh.cc is unreachable during build
  starCount = 0;
}

export const STARS = starCount;

// --- Formatted display values ---

function formatCount(n: number, suffix = "+"): string {
  if (n >= 1000) {
    const thousands = Math.floor(n / 100) * 100;
    return `${thousands.toLocaleString("en-US")}${suffix}`;
  }
  return `${n}${suffix}`;
}

export const DISPLAY = {
  models: formatCount(MODEL_COUNT),
  benchmarks: formatCount(BENCHMARK_COUNT),
  providers: `${PROVIDER_COUNT}+`,
  agents: `${AGENT_COUNT}+`,
  statusProviders: `${STATUS_PROVIDER_COUNT}+`,
} as const;

// --- Site metadata ---

export const SITE = {
  title: "models — browse the AI ecosystem from your terminal",
  description: `High-density terminal navigator for the AI landscape. Browse ${DISPLAY.models} models, benchmarks, coding agents, and provider status from your terminal.`,
} as const;
