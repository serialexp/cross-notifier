// ABOUTME: Proof-of-concept for cross-platform notification display.
// ABOUTME: Tests transparent, frameless, always-on-top window with giu.

package main

import (
	"image/color"
	"time"

	"github.com/AllenDang/cimgui-go/imgui"
	g "github.com/AllenDang/giu"
)

var wnd *g.MasterWindow

func loop() {
	imgui.PushStyleVarFloat(imgui.StyleVarWindowBorderSize, 0)
	g.PushColorWindowBg(color.RGBA{30, 30, 30, 200})

	g.SingleWindow().Layout(
		g.Label("Test Notification"),
		g.Label("This should be transparent, frameless, and on top."),
	)

	g.PopStyleColor()
	imgui.PopStyleVar()
}

func main() {
	wnd = g.NewMasterWindow(
		"Notification",
		300, 100,
		g.MasterWindowFlagsFloating|
			g.MasterWindowFlagsFrameless|
			g.MasterWindowFlagsTransparent,
	)

	wnd.SetBgColor(color.RGBA{0, 0, 0, 0})

	// Auto-close after 5 seconds
	go func() {
		time.Sleep(5 * time.Second)
		wnd.Close()
	}()

	wnd.Run(loop)
}
