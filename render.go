package main

import (
	"fmt"
	"image"
	"image/color"
	"log"
	"unsafe"

	"github.com/go-gl/gl/v2.1/gl"
	"github.com/go-gl/glfw/v3.3/glfw"
)

// Renderer manages OpenGL state and 2D drawing primitives
type Renderer struct {
	window      *glfw.Window
	width       int
	height      int
	vao         uint32
	vbo         uint32
	ebo         uint32
	shaderProg  uint32
	fontTexture uint32
	fontAtlas   *FontAtlas

	// Pre-allocated batch storage to avoid per-frame allocations
	batches    []renderBatch // Use value type with pre-allocated slices
	batchCount int           // Number of active batches this frame

	// Pre-allocated frame buffers
	draws      []renderDraw
	drawCount  int
	frameVerts []float32
	frameIdx   []uint32
	vboSize    int
	eboSize    int

	// Initial capacity for pre-allocation
	vertCapacity int
	idxCapacity  int
}

type renderBatch struct {
	tex         uint32
	vertices    []float32
	indices     []uint32
	vertexCount int // Actual used count (not len)
	indexCount  int // Actual used count (not len)
}

type renderDraw struct {
	tex        uint32
	startIndex int
	indexCount int
}

const (
	initialBatchCount   = 16
	initialVertCapacity = 4096 // 512 quads worth of vertices
	initialIdxCapacity  = 6144 // 1024 quads worth of indices
	initialDrawCapacity = 32
	floatsPerVertex     = 8
	verticesPerQuad     = 4
	indicesPerQuad      = 6
)

var debugFontMetrics bool

// Color represents an RGBA color for rendering
type Color struct {
	R, G, B, A float32
}

func ColorRGBA(c color.Color) Color {
	r, g, b, a := c.RGBA()
	return Color{
		R: float32(r) / 65535.0,
		G: float32(g) / 65535.0,
		B: float32(b) / 65535.0,
		A: float32(a) / 65535.0,
	}
}

// NewRenderer initializes OpenGL and returns a renderer
func NewRenderer(window *glfw.Window, width, height int, fontData []byte) (*Renderer, error) {
	if err := gl.Init(); err != nil {
		return nil, fmt.Errorf("failed to initialize OpenGL: %w", err)
	}

	r := &Renderer{
		window:       window,
		width:        width,
		height:       height,
		vertCapacity: initialVertCapacity,
		idxCapacity:  initialIdxCapacity,
	}

	// Pre-allocate batches with their own buffers
	r.batches = make([]renderBatch, initialBatchCount)
	for i := range r.batches {
		r.batches[i].vertices = make([]float32, 0, initialVertCapacity/initialBatchCount)
		r.batches[i].indices = make([]uint32, 0, initialIdxCapacity/initialBatchCount)
	}

	// Pre-allocate frame buffers
	r.draws = make([]renderDraw, initialDrawCapacity)
	r.frameVerts = make([]float32, 0, initialVertCapacity)
	r.frameIdx = make([]uint32, 0, initialIdxCapacity)

	// Load font atlas
	atlas, err := CreateFontAtlas(fontData, 16)
	if err != nil {
		return nil, fmt.Errorf("failed to load font atlas: %w", err)
	}
	r.fontAtlas = atlas
	r.fontTexture = uploadTexture(atlas.Image)

	// Setup OpenGL
	gl.ClearColor(0, 0, 0, 0)
	gl.Enable(gl.BLEND)
	gl.BlendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA)

	// Create shader program
	if err := r.setupShaders(); err != nil {
		return nil, err
	}

	// Setup VAO/VBO
	r.setupBuffers()

	return r, nil
}

func (r *Renderer) setupShaders() error {
	// Simple 2D vertex shader
	vertexShader := `
#version 120
attribute vec2 position;
attribute vec2 texCoord;
attribute vec4 color;
uniform mat4 projection;
varying vec2 vTexCoord;
varying vec4 vColor;

void main() {
    gl_Position = projection * vec4(position, 0.0, 1.0);
    vTexCoord = texCoord;
    vColor = color;
}
`

	// Simple fragment shader
	fragmentShader := `
#version 120
uniform sampler2D tex;
uniform bool useTexture;
varying vec2 vTexCoord;
varying vec4 vColor;

void main() {
    if (useTexture) {
        gl_FragColor = texture2D(tex, vTexCoord) * vColor;
    } else {
        gl_FragColor = vColor;
    }
}
`

	vs, err := compileShader(vertexShader, gl.VERTEX_SHADER)
	if err != nil {
		return fmt.Errorf("vertex shader: %w", err)
	}

	fs, err := compileShader(fragmentShader, gl.FRAGMENT_SHADER)
	if err != nil {
		return fmt.Errorf("fragment shader: %w", err)
	}

	prog := gl.CreateProgram()
	gl.AttachShader(prog, vs)
	gl.AttachShader(prog, fs)
	gl.BindAttribLocation(prog, 0, gl.Str("position\x00"))
	gl.BindAttribLocation(prog, 1, gl.Str("texCoord\x00"))
	gl.BindAttribLocation(prog, 2, gl.Str("color\x00"))
	gl.LinkProgram(prog)

	var status int32
	gl.GetProgramiv(prog, gl.LINK_STATUS, &status)
	if status == gl.FALSE {
		var logLength int32
		gl.GetProgramiv(prog, gl.INFO_LOG_LENGTH, &logLength)
		logBytes := make([]byte, logLength)
		gl.GetProgramInfoLog(prog, logLength, &logLength, &logBytes[0])
		return fmt.Errorf("shader link error: %s", string(logBytes))
	}

	r.shaderProg = prog
	gl.DeleteShader(vs)
	gl.DeleteShader(fs)

	return nil
}

func (r *Renderer) setupBuffers() {
	gl.GenVertexArrays(1, &r.vao)
	gl.GenBuffers(1, &r.vbo)
	gl.GenBuffers(1, &r.ebo)

	gl.BindVertexArray(r.vao)
	gl.BindBuffer(gl.ARRAY_BUFFER, r.vbo)
	gl.BindBuffer(gl.ELEMENT_ARRAY_BUFFER, r.ebo)

	// Vertex layout: position (2), texCoord (2), color (4)
	stride := int32(8 * 4) // 8 floats per vertex

	// Position -> attribute 0
	gl.VertexAttribPointer(0, 2, gl.FLOAT, false, stride, nil)
	gl.EnableVertexAttribArray(0)

	// TexCoord -> attribute 1
	texCoordOffset := uintptr(2 * 4)
	gl.VertexAttribPointer(1, 2, gl.FLOAT, false, stride, unsafe.Pointer(texCoordOffset))
	gl.EnableVertexAttribArray(1)

	// Color -> attribute 2
	colorOffset := uintptr(4 * 4)
	gl.VertexAttribPointer(2, 4, gl.FLOAT, false, stride, unsafe.Pointer(colorOffset))
	gl.EnableVertexAttribArray(2)

	gl.BindVertexArray(0)
}

// BeginFrame prepares for rendering
func (r *Renderer) BeginFrame() {
	gl.Viewport(0, 0, int32(r.width), int32(r.height))
	gl.Clear(gl.COLOR_BUFFER_BIT)

	gl.UseProgram(r.shaderProg)

	// Setup orthographic projection
	setProjectionMatrix(r.shaderProg, float32(r.width), float32(r.height))

	// Reset batches without deallocating (reuse slices)
	for i := 0; i < r.batchCount; i++ {
		r.batches[i].vertices = r.batches[i].vertices[:0]
		r.batches[i].indices = r.batches[i].indices[:0]
		r.batches[i].vertexCount = 0
		r.batches[i].indexCount = 0
		r.batches[i].tex = 0
	}
	r.batchCount = 0
	r.drawCount = 0
}

// EndFrame finishes rendering
func (r *Renderer) EndFrame() {
	r.frameVerts = r.frameVerts[:0]
	r.frameIdx = r.frameIdx[:0]
	r.drawCount = 0

	for i := 0; i < r.batchCount; i++ {
		batch := &r.batches[i]
		if len(batch.indices) == 0 {
			continue
		}
		if len(batch.vertices)%floatsPerVertex != 0 {
			log.Printf("render batch has misaligned vertex data: %d floats", len(batch.vertices))
			continue
		}
		vertexCount := len(batch.vertices) / floatsPerVertex
		if vertexCount == 0 {
			continue
		}

		// Validate indices (only in debug builds ideally, but keep for safety)
		valid := true
		for _, idx := range batch.indices {
			if int(idx) >= vertexCount {
				log.Printf("render batch has out-of-range index %d (verts=%d)", idx, vertexCount)
				valid = false
				break
			}
		}
		if !valid {
			continue
		}

		baseVertex := uint32(len(r.frameVerts) / floatsPerVertex)
		startIndex := len(r.frameIdx)
		r.frameVerts = append(r.frameVerts, batch.vertices...)

		// Pre-grow frameIdx if needed
		neededCap := len(r.frameIdx) + len(batch.indices)
		if cap(r.frameIdx) < neededCap {
			newIdx := make([]uint32, len(r.frameIdx), neededCap*2)
			copy(newIdx, r.frameIdx)
			r.frameIdx = newIdx
		}

		for _, idx := range batch.indices {
			r.frameIdx = append(r.frameIdx, baseVertex+idx)
		}

		// Grow draws slice if needed
		if r.drawCount >= len(r.draws) {
			r.draws = append(r.draws, renderDraw{})
		}
		r.draws[r.drawCount] = renderDraw{
			tex:        batch.tex,
			startIndex: startIndex,
			indexCount: len(batch.indices),
		}
		r.drawCount++
	}

	if len(r.frameIdx) == 0 {
		gl.UseProgram(0)
		return
	}

	gl.BindVertexArray(r.vao)
	gl.BindBuffer(gl.ARRAY_BUFFER, r.vbo)
	vertexBytes := len(r.frameVerts) * 4
	if vertexBytes > r.vboSize {
		gl.BufferData(gl.ARRAY_BUFFER, vertexBytes*2, nil, gl.DYNAMIC_DRAW) // Double capacity
		r.vboSize = vertexBytes * 2
	}
	gl.BufferSubData(gl.ARRAY_BUFFER, 0, vertexBytes, gl.Ptr(r.frameVerts))

	gl.BindBuffer(gl.ELEMENT_ARRAY_BUFFER, r.ebo)
	indexBytes := len(r.frameIdx) * 4
	if indexBytes > r.eboSize {
		gl.BufferData(gl.ELEMENT_ARRAY_BUFFER, indexBytes*2, nil, gl.DYNAMIC_DRAW) // Double capacity
		r.eboSize = indexBytes * 2
	}
	gl.BufferSubData(gl.ELEMENT_ARRAY_BUFFER, 0, indexBytes, gl.Ptr(r.frameIdx))

	// Re-assert attribute bindings in case external code modified GL state.
	stride := int32(floatsPerVertex * 4)
	gl.VertexAttribPointer(0, 2, gl.FLOAT, false, stride, nil)
	gl.EnableVertexAttribArray(0)
	texCoordOffset := uintptr(2 * 4)
	gl.VertexAttribPointer(1, 2, gl.FLOAT, false, stride, unsafe.Pointer(texCoordOffset))
	gl.EnableVertexAttribArray(1)
	colorOffset := uintptr(4 * 4)
	gl.VertexAttribPointer(2, 4, gl.FLOAT, false, stride, unsafe.Pointer(colorOffset))
	gl.EnableVertexAttribArray(2)

	loc := gl.GetUniformLocation(r.shaderProg, gl.Str("useTexture\x00"))
	for i := 0; i < r.drawCount; i++ {
		draw := &r.draws[i]
		if draw.indexCount == 0 {
			continue
		}
		if draw.tex != 0 {
			gl.ActiveTexture(gl.TEXTURE0)
			gl.BindTexture(gl.TEXTURE_2D, draw.tex)
			gl.Uniform1i(loc, 1)
		} else {
			gl.Uniform1i(loc, 0)
		}

		offset := unsafe.Pointer(uintptr(draw.startIndex * 4))
		gl.DrawElements(gl.TRIANGLES, int32(draw.indexCount), gl.UNSIGNED_INT, offset)
	}
	gl.UseProgram(0)
}

// DrawRect draws a filled rectangle
func (r *Renderer) DrawRect(x, y, w, h float32, c Color) {
	r.drawQuad(x, y, w, h, c, false)
}

// DrawBorder draws a rectangle outline
func (r *Renderer) DrawBorder(x, y, w, h, thickness float32, c Color) {
	// Top
	r.DrawRect(x, y, w, thickness, c)
	// Bottom
	r.DrawRect(x, y+h-thickness, w, thickness, c)
	// Left
	r.DrawRect(x, y, thickness, h, c)
	// Right
	r.DrawRect(x+w-thickness, y, thickness, h, c)
}

// DrawImage draws a textured rectangle
func (r *Renderer) DrawImage(x, y, w, h float32, img *image.RGBA, c Color) {
	texID := uploadTexture(img)
	defer gl.DeleteTextures(1, &texID)

	r.drawQuadTextured(x, y, w, h, c, texID)
}

// DrawTexture draws a textured rectangle using a pre-uploaded texture ID.
func (r *Renderer) DrawTexture(x, y, w, h float32, texID uint32, c Color) {
	if texID == 0 {
		return
	}
	r.drawQuadTextured(x, y, w, h, c, texID)
}

// DrawText draws text at position using the font atlas
func (r *Renderer) DrawText(x, y float32, text string, c Color) Bounds {
	return r.drawText(x, y, text, c)
}

// DrawTextWrapped draws text wrapped to maxWidth
func (r *Renderer) DrawTextWrapped(x, y, maxWidth float32, text string, c Color) Bounds {
	return r.drawTextWrapped(x, y, maxWidth, text, c)
}

// Resize updates renderer viewport
func (r *Renderer) Resize(width, height int) {
	r.width = width
	r.height = height
}

// SetScissor enables scissor testing to clip rendering to a rectangle.
// Coordinates are in screen space (y=0 at top).
// Note: This affects all subsequent draws until ClearScissor is called.
// For batched rendering, scissor is applied when draws are submitted in EndFrame.
func (r *Renderer) SetScissor(x, y, w, h float32) {
	// OpenGL scissor uses bottom-left origin, so convert y coordinate
	glY := float32(r.height) - y - h
	gl.Enable(gl.SCISSOR_TEST)
	gl.Scissor(int32(x), int32(glY), int32(w), int32(h))
}

// ClearScissor disables scissor testing.
func (r *Renderer) ClearScissor() {
	gl.Disable(gl.SCISSOR_TEST)
}

// Destroy cleans up OpenGL resources
func (r *Renderer) Destroy() {
	if r.fontAtlas != nil {
		r.fontAtlas.Destroy()
	}
	gl.DeleteTextures(1, &r.fontTexture)
	gl.DeleteBuffers(1, &r.vbo)
	gl.DeleteBuffers(1, &r.ebo)
	gl.DeleteVertexArrays(1, &r.vao)
	gl.DeleteProgram(r.shaderProg)
}

// ==================== Internal helpers ====================

func (r *Renderer) drawQuad(x, y, w, h float32, c Color, useTexture bool) {
	r.addQuad(0, x, y, w, h, 0, 0, 1, 1, c)
}

func (r *Renderer) drawQuadTextured(x, y, w, h float32, c Color, texID uint32) {
	r.addQuad(texID, x, y, w, h, 0, 0, 1, 1, c)
}

func (r *Renderer) drawText(x, y float32, text string, c Color) Bounds {
	offsetY := y
	if r.fontAtlas != nil {
		extra := r.fontAtlas.Height - r.fontAtlas.Size
		if extra < 0 {
			extra = 0
		}
		offsetY = y - float32(extra)
	}
	bounds := Bounds{X: x, Y: offsetY}

	if r.fontAtlas == nil || text == "" {
		return bounds
	}

	if debugFontMetrics {
		curX := x
		for _, ch := range text {
			glyph, ok := r.fontAtlas.Glyphs[ch]
			if !ok {
				continue
			}
			texW := float32(glyph.Width) / float32(r.fontAtlas.Image.Bounds().Dx())
			texH := float32(glyph.Height) / float32(r.fontAtlas.Image.Bounds().Dy())
			texX := float32(glyph.X) / float32(r.fontAtlas.Image.Bounds().Dx())
			texY := float32(glyph.Y) / float32(r.fontAtlas.Image.Bounds().Dy())
			glyphW := float32(glyph.Width)
			glyphH := float32(glyph.Height)
			r.drawGlyph(curX, offsetY, glyphW, glyphH, texX, texY, texW, texH, c)
			r.drawDebugRect(curX, offsetY, glyphW, glyphH, Color{R: 1, G: 0.2, B: 0.2, A: 0.6})
			curX += float32(glyph.Advance)
			bounds.Width = curX - x
			bounds.Height = glyphH
		}
		ascent := float32(r.fontAtlas.Ascent)
		descent := float32(r.fontAtlas.Descent)
		baseline := offsetY + ascent
		r.drawDebugLine(x, offsetY, bounds.Width, Color{R: 0.2, G: 0.6, B: 1, A: 0.6})
		r.drawDebugLine(x, baseline, bounds.Width, Color{R: 0.2, G: 1, B: 0.4, A: 0.6})
		r.drawDebugLine(x, baseline+descent, bounds.Width, Color{R: 1, G: 0.6, B: 0.2, A: 0.6})
		return bounds
	}

	curX := x
	atlasW := float32(r.fontAtlas.Image.Bounds().Dx())
	atlasH := float32(r.fontAtlas.Image.Bounds().Dy())

	for _, ch := range text {
		glyph, ok := r.fontAtlas.Glyphs[ch]
		if !ok {
			continue
		}

		texW := float32(glyph.Width) / atlasW
		texH := float32(glyph.Height) / atlasH
		texX := float32(glyph.X) / atlasW
		texY := float32(glyph.Y) / atlasH

		glyphW := float32(glyph.Width)
		glyphH := float32(glyph.Height)

		r.drawGlyph(curX, offsetY, glyphW, glyphH, texX, texY, texW, texH, c)
		curX += float32(glyph.Advance)
		if glyphH > bounds.Height {
			bounds.Height = glyphH
		}
	}

	bounds.Width = curX - x
	return bounds
}

func (r *Renderer) drawTextWrapped(x, y, maxWidth float32, text string, c Color) Bounds {
	// TODO: Implement text wrapping
	return r.drawText(x, y, text, c)
}

func (r *Renderer) drawDebugLine(x, y, w float32, c Color) {
	if w <= 0 {
		return
	}
	r.drawQuad(x, y, w, 1, c, false)
}

func (r *Renderer) drawDebugRect(x, y, w, h float32, c Color) {
	if w <= 0 || h <= 0 {
		return
	}
	r.DrawBorder(x, y, w, h, 1, c)
}

func (r *Renderer) drawGlyph(x, y, w, h, texX, texY, texW, texH float32, c Color) {
	r.addQuad(r.fontTexture, x, y, w, h, texX, texY, texW, texH, c)
}

func (r *Renderer) addQuad(texID uint32, x, y, w, h, texX, texY, texW, texH float32, c Color) {
	// Find or create a batch for this texture
	var batch *renderBatch
	if r.batchCount > 0 {
		last := &r.batches[r.batchCount-1]
		if last.tex == texID {
			batch = last
		}
	}

	if batch == nil {
		// Need a new batch
		if r.batchCount >= len(r.batches) {
			// Grow the batches slice
			newBatch := renderBatch{
				vertices: make([]float32, 0, initialVertCapacity/initialBatchCount),
				indices:  make([]uint32, 0, initialIdxCapacity/initialBatchCount),
			}
			r.batches = append(r.batches, newBatch)
		}
		batch = &r.batches[r.batchCount]
		batch.tex = texID
		batch.vertices = batch.vertices[:0]
		batch.indices = batch.indices[:0]
		r.batchCount++
	}

	base := uint32(len(batch.vertices) / floatsPerVertex)
	batch.vertices = append(batch.vertices,
		x, y, texX, texY, c.R, c.G, c.B, c.A,
		x+w, y, texX+texW, texY, c.R, c.G, c.B, c.A,
		x+w, y+h, texX+texW, texY+texH, c.R, c.G, c.B, c.A,
		x, y+h, texX, texY+texH, c.R, c.G, c.B, c.A,
	)
	batch.indices = append(batch.indices, base, base+1, base+2, base+2, base+3, base)
}

// ==================== Utility functions ====================

func compileShader(source string, shaderType uint32) (uint32, error) {
	shader := gl.CreateShader(shaderType)
	csources, free := gl.Strs(source)
	gl.ShaderSource(shader, 1, csources, nil)
	free()
	gl.CompileShader(shader)

	var status int32
	gl.GetShaderiv(shader, gl.COMPILE_STATUS, &status)
	if status == gl.FALSE {
		var logLength int32
		gl.GetShaderiv(shader, gl.INFO_LOG_LENGTH, &logLength)
		logBytes := make([]byte, logLength)
		gl.GetShaderInfoLog(shader, logLength, &logLength, &logBytes[0])
		gl.DeleteShader(shader)
		return 0, fmt.Errorf("compile error: %s", string(logBytes))
	}

	return shader, nil
}

func uploadTexture(img *image.RGBA) uint32 {
	var tex uint32
	gl.GenTextures(1, &tex)
	gl.BindTexture(gl.TEXTURE_2D, tex)

	gl.TexParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
	gl.TexParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
	gl.TexParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
	gl.TexParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)

	bounds := img.Bounds()
	gl.TexImage2D(gl.TEXTURE_2D, 0, gl.RGBA, int32(bounds.Dx()), int32(bounds.Dy()),
		0, gl.RGBA, gl.UNSIGNED_BYTE, gl.Ptr(img.Pix))

	return tex
}

func setProjectionMatrix(prog uint32, w, h float32) {
	// Orthographic projection: (0,0) is top-left, (w,h) is bottom-right
	proj := ortho(0, w, h, 0, -1, 1)

	loc := gl.GetUniformLocation(prog, gl.Str("projection\x00"))
	gl.UniformMatrix4fv(loc, 1, false, &proj[0])
}

// ortho creates an orthographic projection matrix
func ortho(left, right, bottom, top, near, far float32) [16]float32 {
	result := [16]float32{}
	result[0] = 2 / (right - left)
	result[5] = 2 / (top - bottom)
	result[10] = -2 / (far - near)
	result[12] = -(right + left) / (right - left)
	result[13] = -(top + bottom) / (top - bottom)
	result[14] = -(far + near) / (far - near)
	result[15] = 1

	return result
}

// Bounds represents text bounds
type Bounds struct {
	X, Y, Width, Height float32
}
