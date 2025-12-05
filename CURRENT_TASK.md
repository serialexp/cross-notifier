# Current Task: Cross-platform Sound Playback

## Status
CI is failing because sound.go uses macOS-specific CGO (NSSound). Need cross-platform solution.

## What We've Done
1. Added notification sound feature with rule-based matching (server, status, regex pattern)
2. Created settings UI with card-based rule editor, preview button, file picker
3. Used NSSound via CGO for fast macOS playback with sound caching
4. Added Hack font for Unicode icons (↑↓✕▶)

## The Problem
- `sound.go` uses `#import <AppKit/AppKit.h>` - macOS only
- CI runs lint/test on Ubuntu, which can't compile this
- Created platform split (`sound_darwin.go`, `sound_other.go`) but that makes Linux a no-op

## Attempted Solutions
1. **beep library** (`github.com/gopxl/beep/v2`) - cross-platform Go audio
   - Problem: Doesn't decode AIFF (macOS system sounds are .aiff)
   - Added `go-audio/aiff` but bridging to beep is complex

2. **Platform-specific approach** - NSSound on macOS, beep on Linux
   - Would work but Linux wouldn't have access to macOS system sounds

## Proposed Solution (Not Yet Implemented)
Bundle our own notification sounds (wav format) in the binary:
- Works identically on all platforms
- Use beep for playback everywhere
- Embed sounds with `//go:embed`
- Remove platform-specific code

## Files Changed (Uncommitted)
- `sound.go` - common matching logic
- `sound_darwin.go` - macOS NSSound (created but may revert)
- `sound_other.go` - stub (created but may revert)
- `go.mod/go.sum` - added beep, go-audio/aiff dependencies

## Git State
- 2 commits pushed to main (sound feature + font/caching)
- Tag v0.3.0 pushed (but CI failing)
- Uncommitted changes for platform split

## Next Steps
1. Find/create a few wav notification sounds to bundle
2. Rewrite sound playback to use beep with embedded wav files
3. Remove platform-specific CGO code
4. Test on both macOS and Linux
5. Fix CI and re-tag release
