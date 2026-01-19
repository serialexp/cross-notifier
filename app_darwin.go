// ABOUTME: macOS-specific application lifecycle handling.
// ABOUTME: Handles dock Quit menu by intercepting NSApplication termination.

package main

/*
#cgo CFLAGS: -x objective-c
#cgo LDFLAGS: -framework Cocoa -framework Carbon

#import <Cocoa/Cocoa.h>
#import <objc/runtime.h>
#import <Carbon/Carbon.h>

@interface QuitObserver : NSObject
+ (void)install;
@end

@implementation QuitObserver

+ (void)install {
    // Observe all the possible termination-related notifications
    NSNotificationCenter *nc = [NSNotificationCenter defaultCenter];

    [nc addObserverForName:NSApplicationWillTerminateNotification
                    object:nil
                     queue:[NSOperationQueue mainQueue]
                usingBlock:^(NSNotification *note) {
        exit(0);
    }];

    // Handle system shutdown/restart
    [[[NSWorkspace sharedWorkspace] notificationCenter]
        addObserverForName:NSWorkspaceWillPowerOffNotification
                    object:nil
                     queue:[NSOperationQueue mainQueue]
                usingBlock:^(NSNotification *note) {
        exit(0);
    }];

    // Install Apple Event handler for quit
    NSAppleEventManager *aem = [NSAppleEventManager sharedAppleEventManager];
    [aem setEventHandler:[self class]
             andSelector:@selector(handleQuitEvent:withReplyEvent:)
           forEventClass:kCoreEventClass
              andEventID:kAEQuitApplication];
}

+ (void)handleQuitEvent:(NSAppleEventDescriptor *)event
         withReplyEvent:(NSAppleEventDescriptor *)reply {
    exit(0);
}

@end

void installQuitHandler() {
    [QuitObserver install];
}
*/
import "C"

func init() {
	C.installQuitHandler()
}
