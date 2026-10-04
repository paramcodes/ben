package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestTranscriptHasExpectedCounts(t *testing.T) {
	transcriptPath := filepath.Join("..", "..", "benchmarks", "fixtures", "transcript.json")
	data, err := os.ReadFile(transcriptPath)
	if err != nil {
		t.Fatalf("reading transcript: %v", err)
	}

	var transcript struct {
		MessageCount int `json:"message_count"`
		ToolRowCount int `json:"tool_row_count"`
	}
	if err := json.Unmarshal(data, &transcript); err != nil {
		t.Fatalf("parsing transcript: %v", err)
	}

	if transcript.MessageCount != 500 {
		t.Errorf("expected 500 messages, got %d", transcript.MessageCount)
	}
	if transcript.ToolRowCount != 100 {
		t.Errorf("expected 100 tool rows, got %d", transcript.ToolRowCount)
	}
}

func TestRepositoryFixtureDeterministic(t *testing.T) {
	repoPath := filepath.Join("..", "..", "benchmarks", "fixtures", "repository")
	fileCount := 0
	totalSize := int64(0)

	filepath.Walk(repoPath, func(path string, info os.FileInfo, err error) error {
		if err == nil && !info.IsDir() {
			fileCount++
			totalSize += info.Size()
		}
		return nil
	})

	if fileCount != 1000 {
		t.Errorf("expected 1000 files, got %d", fileCount)
	}
	if totalSize != 100*1024*1024 {
		t.Errorf("expected 100 MiB, got %d bytes", totalSize)
	}
}