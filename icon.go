// ABOUTME: Icon loading from various sources (file, URL, base64).
// ABOUTME: Handles resolution priority and image scaling.

package main

import (
	"bytes"
	"encoding/base64"
	"fmt"
	"image"
	_ "image/jpeg"
	"image/png"
	"net/http"
	"os"
	"time"
)

// loadIconFromBase64 decodes base64 image data and returns a scaled image.
func loadIconFromBase64(data string) (image.Image, error) {
	decoded, err := base64.StdEncoding.DecodeString(data)
	if err != nil {
		return nil, fmt.Errorf("base64 decode: %w", err)
	}

	img, _, err := image.Decode(bytes.NewReader(decoded))
	if err != nil {
		return nil, fmt.Errorf("image decode: %w", err)
	}

	return scaleImage(img, iconSize, iconSize), nil
}

// loadIconFromURL fetches an image from a URL and returns a scaled image.
func loadIconFromURL(url string) (image.Image, error) {
	client := &http.Client{
		Timeout: 10 * time.Second,
	}

	resp, err := client.Get(url)
	if err != nil {
		return nil, fmt.Errorf("fetch URL: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("HTTP %d: %s", resp.StatusCode, resp.Status)
	}

	img, _, err := image.Decode(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("image decode: %w", err)
	}

	return scaleImage(img, iconSize, iconSize), nil
}

// loadIconFromPath loads an image from a file path and returns a scaled image.
func loadIconFromPath(path string) (image.Image, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()

	img, _, err := image.Decode(f)
	if err != nil {
		return nil, fmt.Errorf("image decode: %w", err)
	}

	return scaleImage(img, iconSize, iconSize), nil
}

// resolveIcon loads an icon from a notification using priority: iconData > iconHref > iconPath.
// Returns nil, nil if no icon is specified.
func resolveIcon(n Notification) (image.Image, error) {
	if n.IconData != "" {
		return loadIconFromBase64(n.IconData)
	}
	if n.IconHref != "" {
		return loadIconFromURL(n.IconHref)
	}
	if n.IconPath != "" {
		return loadIconFromPath(n.IconPath)
	}
	return nil, nil
}

// fetchAndEncodeIcon fetches an image from a URL, resizes it, and returns base64-encoded PNG.
func fetchAndEncodeIcon(url string) (string, error) {
	img, err := loadIconFromURL(url)
	if err != nil {
		return "", err
	}
	return encodeImageToBase64(img)
}

// encodeImageToBase64 encodes an image as base64 PNG.
func encodeImageToBase64(img image.Image) (string, error) {
	var buf bytes.Buffer
	if err := png.Encode(&buf, img); err != nil {
		return "", fmt.Errorf("PNG encode: %w", err)
	}
	return base64.StdEncoding.EncodeToString(buf.Bytes()), nil
}
