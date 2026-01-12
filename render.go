package main

import (
	"fmt"
	"image"
	"image/color"
	"log"

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
}

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
		window: window,
		width:  width,
		height: height,
	}

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
uniform mat4 projection;

void main() {
    gl_Position = projection * gl_Vertex;
    gl_TexCoord[0] = gl_MultiTexCoord0;
    gl_FrontColor = gl_Color;
}
`

	// Simple fragment shader
	fragmentShader := `
#version 120
uniform sampler2D tex;
uniform bool useTexture;

void main() {
    if (useTexture) {
        gl_FragColor = texture2D(tex, gl_TexCoord[0].st) * gl_Color;
    } else {
        gl_FragColor = gl_Color;
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

	// Position
	gl.VertexAttribPointer(0, 2, gl.FLOAT, false, stride, gl.PtrOffset(0))
	gl.EnableVertexAttribArray(0)

	// TexCoord
	gl.VertexAttribPointer(1, 2, gl.FLOAT, false, stride, gl.PtrOffset(2*4))
	gl.EnableVertexAttribArray(1)

	// Color
	gl.VertexAttribPointer(2, 4, gl.FLOAT, false, stride, gl.PtrOffset(4*4))
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
}

// EndFrame finishes rendering
func (r *Renderer) EndFrame() {
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
	vertices := []float32{
		x, y, 0, 0, c.R, c.G, c.B, c.A,
		x + w, y, 1, 0, c.R, c.G, c.B, c.A,
		x + w, y + h, 1, 1, c.R, c.G, c.B, c.A,
		x, y + h, 0, 1, c.R, c.G, c.B, c.A,
	}

	indices := []uint32{0, 1, 2, 2, 3, 0}

	gl.BindVertexArray(r.vao)
	gl.BindBuffer(gl.ARRAY_BUFFER, r.vbo)
	gl.BufferData(gl.ARRAY_BUFFER, len(vertices)*4, gl.Ptr(vertices), gl.DYNAMIC_DRAW)

	gl.BindBuffer(gl.ELEMENT_ARRAY_BUFFER, r.ebo)
	gl.BufferData(gl.ELEMENT_ARRAY_BUFFER, len(indices)*4, gl.Ptr(indices), gl.DYNAMIC_DRAW)

	loc := gl.GetUniformLocation(r.shaderProg, gl.Str("useTexture\x00"))
	gl.Uniform1i(loc, 0)

	gl.DrawElements(gl.TRIANGLES, 6, gl.UNSIGNED_INT, gl.PtrOffset(0))
}

func (r *Renderer) drawQuadTextured(x, y, w, h float32, c Color, texID uint32) {
	vertices := []float32{
		x, y, 0, 0, c.R, c.G, c.B, c.A,
		x + w, y, 1, 0, c.R, c.G, c.B, c.A,
		x + w, y + h, 1, 1, c.R, c.G, c.B, c.A,
		x, y + h, 0, 1, c.R, c.G, c.B, c.A,
	}

	indices := []uint32{0, 1, 2, 2, 3, 0}

	gl.BindVertexArray(r.vao)
	gl.BindBuffer(gl.ARRAY_BUFFER, r.vbo)
	gl.BufferData(gl.ARRAY_BUFFER, len(vertices)*4, gl.Ptr(vertices), gl.DYNAMIC_DRAW)

	gl.BindBuffer(gl.ELEMENT_ARRAY_BUFFER, r.ebo)
	gl.BufferData(gl.ELEMENT_ARRAY_BUFFER, len(indices)*4, gl.Ptr(indices), gl.DYNAMIC_DRAW)

	gl.ActiveTexture(gl.TEXTURE0)
	gl.BindTexture(gl.TEXTURE_2D, texID)
	loc := gl.GetUniformLocation(r.shaderProg, gl.Str("useTexture\x00"))
	gl.Uniform1i(loc, 1)

	gl.DrawElements(gl.TRIANGLES, 6, gl.UNSIGNED_INT, gl.PtrOffset(0))
}

func (r *Renderer) drawText(x, y float32, text string, c Color) Bounds {
	bounds := Bounds{X: x, Y: y}

	curX := x
	for _, ch := range text {
		glyph, ok := r.fontAtlas.Glyphs[ch]
		if !ok {
			continue
		}

		// Draw glyph quad
		texW := float32(glyph.Width) / float32(r.fontAtlas.Image.Bounds().Dx())
		texH := float32(glyph.Height) / float32(r.fontAtlas.Image.Bounds().Dy())
		texX := float32(glyph.X) / float32(r.fontAtlas.Image.Bounds().Dx())
		texY := float32(glyph.Y) / float32(r.fontAtlas.Image.Bounds().Dy())

		glyphW := float32(glyph.Width)
		glyphH := float32(glyph.Height)

		r.drawGlyph(curX, y, glyphW, glyphH, texX, texY, texW, texH, c)

		curX += float32(glyph.Advance)
		bounds.Width = curX - x
		bounds.Height = glyphH
	}

	return bounds
}

func (r *Renderer) drawTextWrapped(x, y, maxWidth float32, text string, c Color) Bounds {
	// TODO: Implement text wrapping
	return r.drawText(x, y, text, c)
}

func (r *Renderer) drawGlyph(x, y, w, h, texX, texY, texW, texH float32, c Color) {
	vertices := []float32{
		x, y, texX, texY, c.R, c.G, c.B, c.A,
		x + w, y, texX + texW, texY, c.R, c.G, c.B, c.A,
		x + w, y + h, texX + texW, texY + texH, c.R, c.G, c.B, c.A,
		x, y + h, texX, texY + texH, c.R, c.G, c.B, c.A,
	}

	indices := []uint32{0, 1, 2, 2, 3, 0}

	gl.BindVertexArray(r.vao)
	gl.BindBuffer(gl.ARRAY_BUFFER, r.vbo)
	gl.BufferData(gl.ARRAY_BUFFER, len(vertices)*4, gl.Ptr(vertices), gl.DYNAMIC_DRAW)

	gl.BindBuffer(gl.ELEMENT_ARRAY_BUFFER, r.ebo)
	gl.BufferData(gl.ELEMENT_ARRAY_BUFFER, len(indices)*4, gl.Ptr(indices), gl.DYNAMIC_DRAW)

	gl.ActiveTexture(gl.TEXTURE0)
	gl.BindTexture(gl.TEXTURE_2D, r.fontTexture)
	loc := gl.GetUniformLocation(r.shaderProg, gl.Str("useTexture\x00"))
	gl.Uniform1i(loc, 1)

	gl.DrawElements(gl.TRIANGLES, 6, gl.UNSIGNED_INT, gl.PtrOffset(0))
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
