package main

import (
	"fmt"
	"log"
	"sync"

	"github.com/go-gl/glfw/v3.3/glfw"
)

// WindowType identifies the type of window
type WindowType int

const (
	WindowTypeNotifications WindowType = iota
	WindowTypeSettings
	WindowTypeCenter
)

// WindowManager manages multiple GLFW windows
type WindowManager struct {
	windows   map[*glfw.Window]*ManagedWindow
	windowsMu sync.RWMutex
	primary   *glfw.Window // Primary window for GL context
	running   bool
}

// ManagedWindow wraps a GLFW window with metadata
type ManagedWindow struct {
	Window   *glfw.Window
	Type     WindowType
	Renderer *Renderer
	OnClose  func()
	OnResize func(width, height int)
	OnRender func() error
}

// NewWindowManager creates a window manager
func NewWindowManager() *WindowManager {
	return &WindowManager{
		windows: make(map[*glfw.Window]*ManagedWindow),
	}
}

// CreateNotificationWindow creates the main notification window
func (wm *WindowManager) CreateNotificationWindow() (*glfw.Window, error) {
	return wm.createWindow("Cross-Notifier", 320, 120, WindowTypeNotifications, true)
}

// CreateSettingsWindow creates the settings window
func (wm *WindowManager) CreateSettingsWindow() (*glfw.Window, error) {
	return wm.createWindow("Cross-Notifier Settings", 500, 600, WindowTypeSettings, false)
}

// CreateCenterWindow creates the notification center window
func (wm *WindowManager) CreateCenterWindow() (*glfw.Window, error) {
	return wm.createWindow("Notification Center", 600, 400, WindowTypeCenter, false)
}

func (wm *WindowManager) createWindow(title string, width, height int, wtype WindowType, isNotification bool) (*glfw.Window, error) {
	// Set window hints
	if isNotification {
		// Notification window: borderless, floating, transparent
		glfw.WindowHint(glfw.Decorated, glfw.False)
		glfw.WindowHint(glfw.Floating, glfw.True)
		glfw.WindowHint(glfw.TransparentFramebuffer, glfw.True)
		glfw.WindowHint(glfw.FocusOnShow, glfw.False)
	} else {
		// Normal windows: regular decorations
		glfw.WindowHint(glfw.Decorated, glfw.True)
		glfw.WindowHint(glfw.Floating, glfw.False)
		glfw.WindowHint(glfw.TransparentFramebuffer, glfw.False)
	}

	glfw.WindowHint(glfw.Visible, glfw.True)
	glfw.WindowHint(glfw.ContextVersionMajor, 2)
	glfw.WindowHint(glfw.ContextVersionMinor, 1)

	// Create window
	glfwWnd, err := glfw.CreateWindow(width, height, title, nil, wm.primary)
	if err != nil {
		return nil, fmt.Errorf("failed to create window: %w", err)
	}

	// Make it the primary window if first window
	if wm.primary == nil {
		wm.primary = glfwWnd
		glfwWnd.MakeContextCurrent()
		glfw.SwapInterval(1) // Enable vsync
	} else {
		glfwWnd.MakeContextCurrent()
	}

	// Load font and setup renderer
	fontData, err := LoadDefaultFont()
	if err != nil {
		glfwWnd.Destroy()
		return nil, fmt.Errorf("failed to load font: %w", err)
	}

	renderer, err := NewRenderer(glfwWnd, width, height, fontData)
	if err != nil {
		glfwWnd.Destroy()
		return nil, fmt.Errorf("failed to initialize renderer: %w", err)
	}

	// Setup window callbacks
	glfwWnd.SetCloseCallback(func(w *glfw.Window) {
		wm.closeWindow(w)
	})

	glfwWnd.SetSizeCallback(func(w *glfw.Window, width, height int) {
		wm.resizeWindow(w, width, height)
	})

	// Store managed window
	mw := &ManagedWindow{
		Window:   glfwWnd,
		Type:     wtype,
		Renderer: renderer,
	}

	wm.windowsMu.Lock()
	wm.windows[glfwWnd] = mw
	wm.windowsMu.Unlock()

	// Platform-specific setup
	if isNotification {
		setupNotificationWindowPlatform(glfwWnd)
	}

	return glfwWnd, nil
}

// GetManagedWindow returns the managed window data
func (wm *WindowManager) GetManagedWindow(w *glfw.Window) *ManagedWindow {
	wm.windowsMu.RLock()
	defer wm.windowsMu.RUnlock()
	return wm.windows[w]
}

// SetWindowRenderCallback sets the render function for a window
func (wm *WindowManager) SetWindowRenderCallback(w *glfw.Window, cb func() error) {
	wm.windowsMu.Lock()
	defer wm.windowsMu.Unlock()
	if mw, ok := wm.windows[w]; ok {
		mw.OnRender = cb
	}
}

// SetWindowCloseCallback sets the close handler for a window
func (wm *WindowManager) SetWindowCloseCallback(w *glfw.Window, cb func()) {
	wm.windowsMu.Lock()
	defer wm.windowsMu.Unlock()
	if mw, ok := wm.windows[w]; ok {
		mw.OnClose = cb
	}
}

// SetWindowResizeCallback sets the resize handler for a window
func (wm *WindowManager) SetWindowResizeCallback(w *glfw.Window, cb func(width, height int)) {
	wm.windowsMu.Lock()
	defer wm.windowsMu.Unlock()
	if mw, ok := wm.windows[w]; ok {
		mw.OnResize = cb
	}
}

// Run starts the main event loop
func (wm *WindowManager) Run(renderFn func() error) error {
	if err := glfw.Init(); err != nil {
		return fmt.Errorf("failed to initialize GLFW: %w", err)
	}
	defer glfw.Terminate()

	wm.running = true
	defer func() { wm.running = false }()

	for wm.running && len(wm.windows) > 0 {
		glfw.PollEvents()

		// Render all windows
		wm.windowsMu.RLock()
		windows := make([]*glfw.Window, 0, len(wm.windows))
		for w := range wm.windows {
			windows = append(windows, w)
		}
		wm.windowsMu.RUnlock()

		for _, w := range windows {
			if w.ShouldClose() {
				continue
			}

			mw := wm.GetManagedWindow(w)
			if mw == nil {
				continue
			}

			w.MakeContextCurrent()

			width, height := w.GetSize()
			mw.Renderer.Resize(width, height)
			mw.Renderer.BeginFrame()

			// Call custom render function
			if mw.OnRender != nil {
				if err := mw.OnRender(); err != nil {
					log.Printf("render error: %v", err)
				}
			}

			// Call global render function
			if err := renderFn(); err != nil {
				log.Printf("global render error: %v", err)
			}

			mw.Renderer.EndFrame()
			w.SwapBuffers()
		}

		// Remove closed windows
		wm.windowsMu.Lock()
		for w, mw := range wm.windows {
			if w.ShouldClose() {
				w.MakeContextCurrent()
				mw.Renderer.Destroy()
				w.Destroy()
				if mw.OnClose != nil {
					mw.OnClose()
				}
				delete(wm.windows, w)
			}
		}
		wm.windowsMu.Unlock()
	}

	// Cleanup remaining windows
	wm.windowsMu.Lock()
	for w, mw := range wm.windows {
		w.MakeContextCurrent()
		mw.Renderer.Destroy()
		w.Destroy()
	}
	wm.windowsMu.Unlock()

	return nil
}

// Stop stops the event loop
func (wm *WindowManager) Stop() {
	wm.running = false
}

// closeWindow handles window close
func (wm *WindowManager) closeWindow(w *glfw.Window) {
	wm.windowsMu.Lock()
	mw := wm.windows[w]
	wm.windowsMu.Unlock()

	if mw != nil && mw.OnClose != nil {
		mw.OnClose()
	}

	w.SetShouldClose(true)
}

// resizeWindow handles window resize
func (wm *WindowManager) resizeWindow(w *glfw.Window, width, height int) {
	wm.windowsMu.RLock()
	mw := wm.windows[w]
	wm.windowsMu.RUnlock()

	if mw != nil {
		mw.Renderer.Resize(width, height)
		if mw.OnResize != nil {
			mw.OnResize(width, height)
		}
	}
}

// ==================== Platform-specific code ====================

// setupNotificationWindowPlatform sets up platform-specific window properties
func setupNotificationWindowPlatform(w *glfw.Window) {
	// Platform-specific window setup for notifications
	// This will be filled in with native code for macOS/Linux taskbar hiding

	// For now, just ensure the window is positioned
	// Actual taskbar hiding requires native APIs
}
