// ABOUTME: System tray icon for easy access to settings and app control.
// ABOUTME: Provides menu items for Settings and Quit.

package main

import (
	_ "embed"
	"os"

	"fyne.io/systray"
)

//go:embed tray.png
var trayIconData []byte

var (
	trayEnd func()
)

// StartTray initializes the system tray for use with an external event loop.
// Call this before starting the main GUI loop (giu).
// The onSettings callback is called when user clicks Settings menu item.
func StartTray(onSettings func()) {
	var mSettings, mQuit *systray.MenuItem

	start, end := systray.RunWithExternalLoop(func() {
		// onReady - called after nativeStart()
		systray.SetIcon(trayIconData)
		systray.SetTooltip("Cross-Notifier")

		mSettings = systray.AddMenuItem("Settings...", "Open settings window")
		systray.AddSeparator()
		mQuit = systray.AddMenuItem("Quit", "Quit cross-notifier")

		// Handle menu clicks in background
		go func() {
			for {
				select {
				case <-mSettings.ClickedCh:
					if onSettings != nil {
						onSettings()
					}
				case <-mQuit.ClickedCh:
					systray.Quit()
					os.Exit(0)
				}
			}
		}()
	}, func() {
		// onExit
	})

	trayEnd = end
	start()
}

// StopTray cleans up the system tray.
func StopTray() {
	if trayEnd != nil {
		trayEnd()
	}
}
