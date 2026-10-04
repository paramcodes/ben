import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const root = join(process.cwd(), "..", "..");

function readJSON(path: string): unknown {
  return JSON.parse(readFileSync(path, "utf-8"));
}

function assert(condition: boolean, message: string): void {
  if (!condition) {
    console.error(`FAIL: ${message}`);
    process.exit(1);
  }
}

// Test: transcript has 500 messages and 100 tool rows
const transcript = readJSON(join(root, "benchmarks", "fixtures", "transcript.json")) as {
  message_count: number;
  tool_row_count: number;
};
assert(transcript.message_count === 500, `expected 500 messages, got ${transcript.message_count}`);
assert(transcript.tool_row_count === 100, `expected 100 tool rows, got ${transcript.tool_row_count}`);

// Test: repository fixture has 1000 files and 100 MiB
const repoPath = join(root, "benchmarks", "fixtures", "repository");
let count = 0;
let size = 0;
function walk(dir: string): void {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    const info = statSync(path);
    if (info.isDirectory()) {
      walk(path);
    } else {
      count++;
      size += info.size;
    }
  }
}
walk(repoPath);
assert(count === 1000, `expected 1000 files, got ${count}`);
assert(size === 100 * 1024 * 1024, `expected 100 MiB, got ${size} bytes`);

console.log("All fixture tests passed.");