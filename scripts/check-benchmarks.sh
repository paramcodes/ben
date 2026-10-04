#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

python3 -c "
import json, hashlib, os, sys

fixtures_dir = 'benchmarks/fixtures'

# Transcript fixture
transcript_path = os.path.join(fixtures_dir, 'transcript.json')
with open(transcript_path, 'rb') as f:
    transcript_hash = hashlib.sha256(f.read()).hexdigest()
with open(transcript_path) as f:
    transcript = json.load(f)

expected_messages = 500
expected_tool_rows = 100
actual_messages = transcript['message_count']
actual_tool_rows = transcript['tool_row_count']

if actual_messages != expected_messages:
    print(f'transcript.json: expected {expected_messages} messages, got {actual_messages}', file=sys.stderr)
    sys.exit(1)
if actual_tool_rows != expected_tool_rows:
    print(f'transcript.json: expected {expected_tool_rows} tool rows, got {actual_tool_rows}', file=sys.stderr)
    sys.exit(1)

# Repository fixture
repo_dir = os.path.join(fixtures_dir, 'repository')
files = [f for f in os.listdir(repo_dir) if f.endswith('.txt')]
file_count = len(files)
total_size = sum(os.path.getsize(os.path.join(repo_dir, f)) for f in files)
expected_files = 1000
expected_size = 100 * 1024 * 1024  # 100 MiB

if file_count != expected_files:
    print(f'repository: expected {expected_files} files, got {file_count}', file=sys.stderr)
    sys.exit(1)
if total_size != expected_size:
    print(f'repository: expected {expected_size} bytes, got {total_size}', file=sys.stderr)
    sys.exit(1)

print(f'OK: transcript.json ({actual_messages} messages, {actual_tool_rows} tool rows, sha256={transcript_hash})')
print(f'OK: repository ({file_count} files, {total_size} bytes, 100.0 MiB)')
"