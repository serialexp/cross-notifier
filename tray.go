// ABOUTME: System tray icon for easy access to settings and app control.
// ABOUTME: Provides menu items for Notifications, Settings, and Quit.

package main

import (
	_ "embed"
	"fmt"
	"os"
	"time"

	"fyne.io/systray"
)

//go:embed tray.png
var trayIconData []byte

var (
	trayEnd func()
)

// StartTray initializes the system tray for use with an external event loop.
// Call this before starting the main GUI loop (giu).
// Callbacks:
// - onSettings: called when user clicks Settings menu item
// - onNotifications: called when user clicks Notifications menu item
// - getConnectionCount: returns the number of connected servers
// - getNotificationCount: returns the number of notifications in the center
func StartTray(onSettings func(), onNotifications func(), getConnectionCount func() int, getNotificationCount func() int) {
	var mStatus, mNotifications, mSettings, mQuit *systray.MenuItem

	start, end := systray.RunWithExternalLoop(func() {
		// onReady - called after nativeStart()
		systray.SetIcon(trayIconData)
		systray.SetTooltip("Cross-Notifier")

		mStatus = systray.AddMenuItem("Not connected", "Server connection status")
		mStatus.Disable()
		systray.AddSeparator()
		mNotifications = systray.AddMenuItem("Notifications", "Open notification center")
		mSettings = systray.AddMenuItem("Settings...", "Open settings window")
		systray.AddSeparator()
		mQuit = systray.AddMenuItem("Quit", "Quit cross-notifier")

		// Update status and notification count periodically
		go func() {
			for {
				if getConnectionCount != nil {
					count := getConnectionCount()
					if count == 0 {
						mStatus.SetTitle("Not connected")
					} else if count == 1 {
						mStatus.SetTitle("Connected to 1 server")
					} else {
						mStatus.SetTitle(fmt.Sprintf("Connected to %d servers", count))
					}
				}
				if getNotificationCount != nil {
					count := getNotificationCount()
					if count == 0 {
						mNotifications.SetTitle("Notifications")
					} else {
						mNotifications.SetTitle(fmt.Sprintf("Notifications (%d)", count))
					}
				}
				time.Sleep(2 * time.Second)
			}
		}()

		// Handle menu clicks in background
		go func() {
			for {
				select {
				case <-mNotifications.ClickedCh:
					if onNotifications != nil {
						onNotifications()
					}
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
