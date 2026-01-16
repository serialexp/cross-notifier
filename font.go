package main

import (
	"fmt"
	"image"
	"log"
	"os"
	"strings"

	"github.com/golang/freetype"
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
	Image   *image.RGBA
	Glyphs  map[rune]GlyphInfo
	Height  int // Font height in pixels (ascent + descent)
	Ascent  int
	Descent int
	Size    int

	// Caches for text operations (avoiding per-frame allocations)
	measureCache   map[string]measureResult
	truncateCache  map[truncateKey]string
	truncateLCache map[truncateLinesKey]string
}

type measureResult struct {
	width, height int
}

type truncateKey struct {
	text     string
	maxWidth int
}

type truncateLinesKey struct {
	text     string
	maxWidth int
	maxLines int
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

	// Calculate scale for glyph metrics (fontSize * DPI / 72)
	scale := fixed.Int26_6(fontSize * 64) // 72 DPI / 72 * 64 = 64
	bounds := ttf.Bounds(scale)
	ascent := int(bounds.Max.Y >> 6)
	descent := int(-bounds.Min.Y >> 6)
	totalHeight := ascent + descent

	atlas := &FontAtlas{
		Glyphs:         make(map[rune]GlyphInfo),
		Height:         totalHeight,
		Ascent:         ascent,
		Descent:        descent,
		Size:           fontSize,
		measureCache:   make(map[string]measureResult),
		truncateCache:  make(map[truncateKey]string),
		truncateLCache: make(map[truncateLinesKey]string),
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
		idx := ttf.Index(ch)
		if idx == 0 {
			continue
		}

		hm := ttf.HMetric(scale, idx)
		advInt := int(hm.AdvanceWidth >> 6) // Convert from fixed point
		totalWidth += advInt + padding
		if totalHeight > maxHeight {
			maxHeight = totalHeight
		}
	}

	// Create atlas image with proper size
	atlasW := totalWidth
	atlasH := maxHeight + padding*2

	atlas.Image = image.NewRGBA(image.Rect(0, 0, atlasW, atlasH))

	// Set up context for rendering to atlas
	c.SetDst(atlas.Image)
	c.SetSrc(image.White)

	// Second pass: render glyphs into atlas
	x := 0
	y := ascent + padding

	for _, ch := range chars {
		idx := ttf.Index(ch)
		if idx == 0 {
			continue
		}

		hm := ttf.HMetric(scale, idx)
		advInt := int(hm.AdvanceWidth >> 6)

		// Wrap if needed
		if x+advInt+padding > atlasW {
			x = 0
			y += totalHeight + padding
		}

		// Set position and draw glyph
		pt := fixed.Point26_6{X: fixed.Int26_6(x * 64), Y: fixed.Int26_6(y * 64)}
		c.SetClip(atlas.Image.Bounds())

		if _, err := c.DrawString(string(ch), pt); err == nil {
			atlas.Glyphs[ch] = GlyphInfo{
				X:       x,
				Y:       y - ascent,
				Width:   advInt,
				Height:  totalHeight,
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

// MeasureText returns the width and height of text (cached)
func (fa *FontAtlas) MeasureText(text string) (width int, height int) {
	if cached, ok := fa.measureCache[text]; ok {
		return cached.width, cached.height
	}

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

	fa.measureCache[text] = measureResult{w, h}
	return w, h
}

// MeasureTextUncached returns the width and height without caching (for temporary strings)
func (fa *FontAtlas) MeasureTextUncached(text string) (width int, height int) {
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

// Truncate truncates text to fit within maxWidth, adding ellipsis if needed (cached)
func (fa *FontAtlas) Truncate(text string, maxWidth int) string {
	if maxWidth <= 0 {
		return ""
	}

	key := truncateKey{text, maxWidth}
	if cached, ok := fa.truncateCache[key]; ok {
		return cached
	}

	w := 0
	ellipsis := "…"
	ellipsisW, _ := fa.MeasureText(ellipsis)

	for i, ch := range text {
		if glyph, ok := fa.Glyphs[ch]; ok {
			if w+glyph.Advance > maxWidth-ellipsisW {
				result := text[:i] + ellipsis
				fa.truncateCache[key] = result
				return result
			}
			w += glyph.Advance
		}
	}

	fa.truncateCache[key] = text
	return text
}

// TruncateLines truncates text to fit maxWidth and maxLines (cached)
func (fa *FontAtlas) TruncateLines(text string, maxWidth int, maxLines int) string {
	if maxWidth <= 0 || maxLines <= 0 {
		return ""
	}

	key := truncateLinesKey{text, maxWidth, maxLines}
	if cached, ok := fa.truncateLCache[key]; ok {
		return cached
	}

	ellipsis := "…"
	ellipsisW, _ := fa.MeasureText(ellipsis)

	lines := make([]string, 0, maxLines)
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
					// Last line, truncate and add ellipsis
					// Remove characters from end until ellipsis fits
					runes := []rune(currentLine)
					w := currentWidth
					for w+ellipsisW > maxWidth && len(runes) > 0 {
						lastRune := runes[len(runes)-1]
						if g, ok := fa.Glyphs[lastRune]; ok {
							w -= g.Advance
						}
						runes = runes[:len(runes)-1]
					}
					currentLine = string(runes) + ellipsis
					lines = append(lines, currentLine)
					currentLine = "" // Clear so post-loop check doesn't duplicate
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

	// Build result without extra allocations
	totalLen := 0
	for _, line := range lines {
		totalLen += len(line)
	}
	totalLen += len(lines) - 1 // newlines

	var builder strings.Builder
	builder.Grow(totalLen)
	for i, line := range lines {
		if i > 0 {
			builder.WriteByte('\n')
		}
		builder.WriteString(line)
	}
	result := builder.String()
	fa.truncateLCache[key] = result
	return result
}

// WrapText wraps text to fit within maxWidth, breaking at character boundaries
func (fa *FontAtlas) WrapText(text string, maxWidth int) string {
	if maxWidth <= 0 {
		return ""
	}

	var builder strings.Builder
	currentLine := ""
	currentWidth := 0

	for _, ch := range text {
		if ch == '\n' {
			if currentLine != "" {
				if builder.Len() > 0 {
					builder.WriteByte('\n')
				}
				builder.WriteString(currentLine)
			}
			currentLine = ""
			currentWidth = 0
			builder.WriteByte('\n')
			continue
		}

		if glyph, ok := fa.Glyphs[ch]; ok {
			if currentWidth+glyph.Advance > maxWidth && currentLine != "" {
				// Line too long, wrap
				if builder.Len() > 0 {
					builder.WriteByte('\n')
				}
				builder.WriteString(currentLine)
				currentLine = string(ch)
				currentWidth = glyph.Advance
			} else {
				currentLine += string(ch)
				currentWidth += glyph.Advance
			}
		}
	}

	if currentLine != "" {
		if builder.Len() > 0 {
			builder.WriteByte('\n')
		}
		builder.WriteString(currentLine)
	}

	return builder.String()
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

	// UI symbols (Geometric Shapes block and others)
	uiSymbols := []rune{
		'▶', // U+25B6 - play button
		'●', // U+25CF - filled circle (connection status)
		'○', // U+25CB - empty circle (connection status)
		'■', // U+25A0 - filled square
		'□', // U+25A1 - empty square
		'▼', // U+25BC - down arrow
		'▲', // U+25B2 - up arrow
		'◀', // U+25C0 - left arrow
		'✓', // U+2713 - check mark
		'⚙', // U+2699 - gear (settings)
		'…', // U+2026 - ellipsis (text truncation)
	}
	chars = append(chars, uiSymbols...)

	return chars
}

// LoadDefaultFont loads the default system font
// On Linux: /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf
// On macOS: /Library/Fonts/Arial.ttf or system fonts
func LoadDefaultFont() ([]byte, error) {
	// Try common font locations
	fontPaths := []string{
		"Hack-Regular.ttf",
		"/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", // Linux
		"/Library/Fonts/Arial.ttf",                        // macOS
		"/System/Library/Fonts/Helvetica.ttc",             // macOS
		"C:\\Windows\\Fonts\\arial.ttf",                   // Windows
		"/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
	}

	for _, path := range fontPaths {
		data, err := os.ReadFile(path)
		if err == nil {
			if _, err := freetype.ParseFont(data); err != nil {
				log.Printf("Skipping font %s: %v", path, err)
				continue
			}
			log.Printf("Using font: %s", path)
			return data, nil
		}
	}

	return nil, fmt.Errorf("no suitable font found in system")
}
