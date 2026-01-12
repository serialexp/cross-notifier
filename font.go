package main

import (
	"bytes"
	"fmt"
	"image"
	"image/draw"
	"log"
	"os"

	"github.com/golang/freetype"
	"github.com/golang/freetype/truetype"
	"golang.org/x/image/font"
	"golang.org/x/image/math/fixed"
)

// GlyphInfo contains rendering information for a glyph
type GlyphInfo struct {
	X       int // X position in atlas
	Y       int // Y position in atlas
	Width   int // Glyph width
	Height  int // Glyph height
	Advance int // Advance width for next glyph
}

// FontAtlas contains the rasterized font glyphs and their positions
type FontAtlas struct {
	Image  *image.RGBA
	Glyphs map[rune]GlyphInfo
	Height int // Font height in pixels
}

// LoadFontAtlas loads a font from file and creates an atlas of common glyphs
func LoadFontAtlas(fontPath string) (*FontAtlas, error) {
	data, err := os.ReadFile(fontPath)
	if err != nil {
		return nil, fmt.Errorf("failed to read font file: %w", err)
	}

	return CreateFontAtlas(data, 16) // 16px font size
}

// CreateFontAtlas creates a font atlas from raw font data
func CreateFontAtlas(fontData []byte, fontSize int) (*FontAtlas, error) {
	ttf, err := freetype.ParseFont(fontData)
	if err != nil {
		return nil, fmt.Errorf("failed to parse font: %w", err)
	}

	// Create freetype context
	c := freetype.NewContext()
	c.SetFont(ttf)
	c.SetFontSize(float64(fontSize))
	c.SetDPI(72)

	atlas := &FontAtlas{
		Glyphs: make(map[rune]GlyphInfo),
		Height: fontSize,
	}

	// Generate ASCII printable characters + common extras
	chars := generateCharacterSet()

	// Create a temporary image for measuring
	tempImage := image.NewRGBA(image.Rect(0, 0, 4096, 4096))
	c.SetDst(tempImage)
	c.SetSrc(image.Black)

	// First pass: measure all glyphs to determine atlas size
	totalWidth := 0
	maxHeight := 0
	padding := 2

	for _, ch := range chars {
		_, advance, ok := c.GlyphAdvance(ch)
		if !ok {
			continue
		}

		advInt := int((advance >> 6)) // Convert from fixed point
		totalWidth += advInt + padding
		if fontSize > maxHeight {
			maxHeight = fontSize
		}
	}

	// Create atlas image with proper size
	atlasW := totalWidth
	atlasH := maxHeight + padding*2

	atlas.Image = image.NewRGBA(image.Rect(0, 0, atlasW, atlasH))
	draw.Draw(atlas.Image, atlas.Image.Bounds(), image.NewUniform(image.White), image.Point{}, draw.Src)

	// Set up context for rendering to atlas
	c.SetDst(atlas.Image)
	c.SetSrc(image.Black)

	// Second pass: render glyphs into atlas
	x := 0
	y := fontSize + padding

	for _, ch := range chars {
		advance, ok := c.GlyphAdvance(ch)
		if !ok {
			continue
		}

		advInt := int((advance >> 6))

		// Wrap if needed
		if x+advInt+padding > atlasW {
			x = 0
			y += fontSize + padding
		}

		// Set position and draw glyph
		pt := fixed.Point26_6{X: fixed.Int26_6(x * 64), Y: fixed.Int26_6(y * 64)}
		c.SetClip(atlas.Image.Bounds())

		if _, err := c.DrawString(string(ch), pt); err == nil {
			atlas.Glyphs[ch] = GlyphInfo{
				X:       x,
				Y:       y - fontSize,
				Width:   advInt,
				Height:  fontSize,
				Advance: advInt,
			}
		}

		x += advInt + padding
	}

	return atlas, nil
}

// Destroy cleans up atlas resources
func (fa *FontAtlas) Destroy() {
	// No active resources to clean up, Image will be GC'd
}

// MeasureText returns the width and height of text
func (fa *FontAtlas) MeasureText(text string) (width int, height int) {
	w := 0
	h := fa.Height

	for _, ch := range text {
		if glyph, ok := fa.Glyphs[ch]; ok {
			w += glyph.Advance
			if glyph.Height > h {
				h = glyph.Height
			}
		}
	}

	return w, h
}

// Truncate truncates text to fit within maxWidth, adding ellipsis if needed
func (fa *FontAtlas) Truncate(text string, maxWidth int) string {
	if maxWidth <= 0 {
		return ""
	}

	w := 0
	ellipsis := "..."
	ellipsisW, _ := fa.MeasureText(ellipsis)

	for i, ch := range text {
		if glyph, ok := fa.Glyphs[ch]; ok {
			if w+glyph.Advance > maxWidth-ellipsisW {
				return text[:i] + ellipsis
			}
			w += glyph.Advance
		}
	}

	return text
}

// TruncateLines truncates text to fit maxWidth and maxLines
func (fa *FontAtlas) TruncateLines(text string, maxWidth int, maxLines int) string {
	if maxWidth <= 0 || maxLines <= 0 {
		return ""
	}

	ellipsis := "..."
	ellipsisW, _ := fa.MeasureText(ellipsis)

	lines := []string{}
	currentLine := ""
	currentWidth := 0
	lineCount := 0

	for _, ch := range text {
		if ch == '\n' {
			lines = append(lines, currentLine)
			currentLine = ""
			currentWidth = 0
			lineCount++
			if lineCount >= maxLines {
				break
			}
			continue
		}

		if glyph, ok := fa.Glyphs[ch]; ok {
			if currentWidth+glyph.Advance > maxWidth {
				// Line too long, wrap
				if lineCount+1 >= maxLines {
					// Last line, add ellipsis
					currentLine = fa.Truncate(currentLine, maxWidth-ellipsisW) + ellipsis
					lines = append(lines, currentLine)
					break
				}
				lines = append(lines, currentLine)
				currentLine = string(ch)
				currentWidth = glyph.Advance
				lineCount++
			} else {
				currentLine += string(ch)
				currentWidth += glyph.Advance
			}
		}
	}

	if currentLine != "" {
		lines = append(lines, currentLine)
	}

	result := ""
	for i, line := range lines {
		if i > 0 {
			result += "\n"
		}
		result += line
	}
	return result
}

// ==================== Helper functions ====================

func generateCharacterSet() []rune {
	// ASCII printable + space
	chars := []rune{}

	// Space
	chars = append(chars, ' ')

	// Printable ASCII 33-126
	for ch := rune(33); ch <= 126; ch++ {
		chars = append(chars, ch)
	}

	// Common symbols and extended ASCII
	// Keep it simple to avoid encoding issues
	for ch := rune(161); ch <= 255; ch++ {
		chars = append(chars, ch)
	}

	return chars
}

// LoadDefaultFont loads the default system font
// On Linux: /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf
// On macOS: /Library/Fonts/Arial.ttf or system fonts
func LoadDefaultFont() ([]byte, error) {
	// Try common font locations
	fontPaths := []string{
		"/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", // Linux
		"/Library/Fonts/Arial.ttf",                        // macOS
		"/System/Library/Fonts/Helvetica.ttc",             // macOS
		"C:\\Windows\\Fonts\\arial.ttf",                   // Windows
		"/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
	}

	for _, path := range fontPaths {
		data, err := os.ReadFile(path)
		if err == nil {
			log.Printf("Using font: %s", path)
			return data, nil
		}
	}

	return nil, fmt.Errorf("no suitable font found in system")
}
