// ABOUTME: Tests for icon loading from various sources.
// ABOUTME: Covers base64, URL, and file path icon loading.

package main

import (
	"encoding/base64"
	"image"
	"image/png"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
)

// createTestPNG creates a minimal valid PNG image
func createTestPNG() []byte {
	img := image.NewRGBA(image.Rect(0, 0, 10, 10))
	// Fill with red
	for y := 0; y < 10; y++ {
		for x := 0; x < 10; x++ {
			img.Set(x, y, image.White)
		}
	}

	f, _ := os.CreateTemp("", "test*.png")
	defer f.Close()
	png.Encode(f, img)
	f.Seek(0, 0)
	data, _ := os.ReadFile(f.Name())
	os.Remove(f.Name())
	return data
}

func TestLoadIconFromBase64(t *testing.T) {
	pngData := createTestPNG()
	encoded := base64.StdEncoding.EncodeToString(pngData)

	img, err := loadIconFromBase64(encoded)
	if err != nil {
		t.Fatalf("loadIconFromBase64 failed: %v", err)
	}
	if img == nil {
		t.Fatal("loadIconFromBase64 returned nil image")
	}

	// Check dimensions (should be scaled to iconSize or smaller)
	bounds := img.Bounds()
	if bounds.Dx() > iconSize || bounds.Dy() > iconSize {
		t.Errorf("image not scaled: got %dx%d, want <= %dx%d",
			bounds.Dx(), bounds.Dy(), iconSize, iconSize)
	}
}

func TestLoadIconFromBase64_InvalidData(t *testing.T) {
	_, err := loadIconFromBase64("not-valid-base64!!!")
	if err == nil {
		t.Error("expected error for invalid base64")
	}
}

func TestLoadIconFromBase64_InvalidImage(t *testing.T) {
	// Valid base64, but not an image
	encoded := base64.StdEncoding.EncodeToString([]byte("hello world"))
	_, err := loadIconFromBase64(encoded)
	if err == nil {
		t.Error("expected error for non-image data")
	}
}

func TestLoadIconFromURL(t *testing.T) {
	pngData := createTestPNG()

	// Start test server
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "image/png")
		w.Write(pngData)
	}))
	defer ts.Close()

	img, err := loadIconFromURL(ts.URL)
	if err != nil {
		t.Fatalf("loadIconFromURL failed: %v", err)
	}
	if img == nil {
		t.Fatal("loadIconFromURL returned nil image")
	}
}

func TestLoadIconFromURL_NotFound(t *testing.T) {
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.NotFound(w, r)
	}))
	defer ts.Close()

	_, err := loadIconFromURL(ts.URL)
	if err == nil {
		t.Error("expected error for 404 response")
	}
}

func TestLoadIconFromURL_InvalidURL(t *testing.T) {
	_, err := loadIconFromURL("not-a-valid-url")
	if err == nil {
		t.Error("expected error for invalid URL")
	}
}

func TestLoadIconFromPath(t *testing.T) {
	pngData := createTestPNG()

	// Write to temp file
	tmpFile := filepath.Join(t.TempDir(), "test.png")
	if err := os.WriteFile(tmpFile, pngData, 0644); err != nil {
		t.Fatalf("failed to write test file: %v", err)
	}

	img, err := loadIconFromPath(tmpFile)
	if err != nil {
		t.Fatalf("loadIconFromPath failed: %v", err)
	}
	if img == nil {
		t.Fatal("loadIconFromPath returned nil image")
	}
}

func TestLoadIconFromPath_NotFound(t *testing.T) {
	_, err := loadIconFromPath("/nonexistent/path/to/file.png")
	if err == nil {
		t.Error("expected error for nonexistent file")
	}
}

func TestResolveIcon_Priority(t *testing.T) {
	pngData := createTestPNG()

	// Create test file
	tmpFile := filepath.Join(t.TempDir(), "test.png")
	if err := os.WriteFile(tmpFile, pngData, 0644); err != nil {
		t.Fatalf("failed to write test file: %v", err)
	}

	// Start test server
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "image/png")
		w.Write(pngData)
	}))
	defer ts.Close()

	encoded := base64.StdEncoding.EncodeToString(pngData)

	// Test that iconData takes priority
	t.Run("iconData priority", func(t *testing.T) {
		n := Notification{
			IconData: encoded,
			IconHref: ts.URL,
			IconPath: tmpFile,
		}
		img, err := resolveIcon(n)
		if err != nil {
			t.Fatalf("resolveIcon failed: %v", err)
		}
		if img == nil {
			t.Fatal("resolveIcon returned nil")
		}
	})

	// Test iconHref when no iconData
	t.Run("iconHref fallback", func(t *testing.T) {
		n := Notification{
			IconHref: ts.URL,
			IconPath: tmpFile,
		}
		img, err := resolveIcon(n)
		if err != nil {
			t.Fatalf("resolveIcon failed: %v", err)
		}
		if img == nil {
			t.Fatal("resolveIcon returned nil")
		}
	})

	// Test iconPath when no iconData or iconHref
	t.Run("iconPath fallback", func(t *testing.T) {
		n := Notification{
			IconPath: tmpFile,
		}
		img, err := resolveIcon(n)
		if err != nil {
			t.Fatalf("resolveIcon failed: %v", err)
		}
		if img == nil {
			t.Fatal("resolveIcon returned nil")
		}
	})

	// Test no icon
	t.Run("no icon", func(t *testing.T) {
		n := Notification{}
		img, err := resolveIcon(n)
		if err != nil {
			t.Fatalf("resolveIcon failed: %v", err)
		}
		if img != nil {
			t.Error("expected nil image when no icon specified")
		}
	})
}
