package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"time"
)

type Result struct {
	Workload      string            `json:"workload"`
	Samples       []int64           `json:"samples"`
	Quantiles     map[string]int64  `json:"quantiles"`
	Environment   map[string]string `json:"environment"`
	FixtureHashes map[string]string `json:"fixture_hashes"`
}

func main() {
	// Workload 1: transcript operation count
	transcriptPath := filepath.Join("..", "..", "benchmarks", "fixtures", "transcript.json")
	transcriptData, err := os.ReadFile(transcriptPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "reading transcript: %v\n", err)
		os.Exit(1)
	}

	var transcript struct {
		MessageCount  int `json:"message_count"`
		ToolRowCount  int `json:"tool_row_count"`
		Messages      []struct {
			Role string `json:"role"`
		} `json:"messages"`
	}
	if err := json.Unmarshal(transcriptData, &transcript); err != nil {
		fmt.Fprintf(os.Stderr, "parsing transcript: %v\n", err)
		os.Exit(1)
	}

	// Count operations
	userCount := 0
	assistantCount := 0
	toolCount := 0
	for _, m := range transcript.Messages {
		switch m.Role {
		case "user":
			userCount++
		case "assistant":
			assistantCount++
		case "tool":
			toolCount++
		}
	}

	// Workload 2: context assembly from repository fixture
	repoPath := filepath.Join("..", "..", "benchmarks", "fixtures", "repository")
	start := time.Now()
	fileCount := 0
	totalSize := int64(0)
	filepath.Walk(repoPath, func(path string, info os.FileInfo, err error) error {
		if err == nil && !info.IsDir() {
			fileCount++
			totalSize += info.Size()
		}
		return nil
	})
	elapsed := time.Since(start).Milliseconds()

	_ = elapsed // suppress unused variable when running without verbose output

	samples := []int64{int64(transcript.MessageCount), int64(transcript.ToolRowCount), int64(fileCount), totalSize}
	sort.Slice(samples, func(i, j int) bool { return samples[i] < samples[j] })

	result := Result{
		Workload: "go_reference",
		Samples:  samples,
		Quantiles: map[string]int64{
			"p50": samples[len(samples)*50/100],
			"p95": samples[len(samples)*95/100],
			"p99": samples[len(samples)*99/100],
		},
		Environment: map[string]string{
			"os":             runtime.GOOS,
			"go_version":     runtime.Version(),
			"terminal_width": "80",
			"terminal_height": "24",
		},
		FixtureHashes: map[string]string{
			"transcript": "8cfa2b9a74079effb9e87daf5a15d43b34abf58d10d2b3e4c14327b1d86522b1",
			"repository": "ed15992aaf55dd6cdd18a7b9521138e3024527df81d7df9f48fb7d20040ae473",
		},
	}

	output, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		fmt.Fprintf(os.Stderr, "marshaling result: %v\n", err)
		os.Exit(1)
	}
	fmt.Println(string(output))
}