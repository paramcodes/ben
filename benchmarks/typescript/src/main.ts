import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const root = join(process.cwd(), "..", "..");

interface Result {
  workload: string;
  samples: number[];
  quantiles: Record<string, number>;
  environment: Record<string, string>;
  fixture_hashes: Record<string, string>;
}

function quantile(sorted: number[], p: number): number {
  const idx = Math.round((p / 100) * (sorted.length - 1));
  return sorted[Math.min(idx, sorted.length - 1)];
}

function countFiles(dir: string): { count: number; size: number } {
  let count = 0;
  let size = 0;
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    const info = statSync(path);
    if (info.isDirectory()) {
      const sub = countFiles(path);
      count += sub.count;
      size += sub.size;
    } else {
      count++;
      size += info.size;
    }
  }
  return { count, size };
}

function main(): void {
  // Workload 1: transcript operation counts
  const transcriptPath = join(root, "benchmarks", "fixtures", "transcript.json");
  const transcriptData = JSON.parse(readFileSync(transcriptPath, "utf-8"));

  let userCount = 0;
  let assistantCount = 0;
  let toolCount = 0;
  for (const msg of transcriptData.messages) {
    switch (msg.role) {
      case "user":
        userCount++;
        break;
      case "assistant":
        assistantCount++;
        break;
      case "tool":
        toolCount++;
        break;
    }
  }

  // Workload 2: context assembly from repository fixture
  const repoPath = join(root, "benchmarks", "fixtures", "repository");
  const repoStats = countFiles(repoPath);

  const samples = [
    transcriptData.message_count,
    transcriptData.tool_row_count,
    repoStats.count,
    repoStats.size,
  ];
  samples.sort((a, b) => a - b);

  const result: Result = {
    workload: "typescript_reference",
    samples,
    quantiles: {
      p50: quantile(samples, 50),
      p95: quantile(samples, 95),
      p99: quantile(samples, 99),
    },
    environment: {
      os: process.platform,
      node_version: process.version,
      terminal_width: "80",
      terminal_height: "24",
    },
    fixture_hashes: {
      transcript: "8cfa2b9a74079effb9e87daf5a15d43b34abf58d10d2b3e4c14327b1d86522b1",
      repository: "ed15992aaf55dd6cdd18a7b9521138e3024527df81d7df9f48fb7d20040ae473",
    },
  };

  console.log(JSON.stringify(result, null, 2));
}

main();