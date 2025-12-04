//go:build tools

// ABOUTME: Tool dependencies managed via go.mod.
// ABOUTME: Install with: go install github.com/golangci/golangci-lint/cmd/golangci-lint

package tools

import (
	_ "github.com/golangci/golangci-lint/cmd/golangci-lint"
)
