// Sound playback for notification sounds.
// Supports embedded WAV files, generated sine tones, and custom audio files via rodio.

use std::io::{BufReader, Cursor};
use std::path::Path;
use std::time::Duration;

use rodio::source::SineWave;
use rodio::{Decoder, OutputStream, Sink, Source};

// Embedded WAV sounds
static SOUND_BELL: &[u8] = include_bytes!("../sounds/mixkit-bell-notification-933.wav");
static SOUND_CONFIRMATION: &[u8] = include_bytes!("../sounds/mixkit-confirmation-tone-2867.wav");
static SOUND_CORRECT: &[u8] = include_bytes!("../sounds/mixkit-correct-answer-tone-2870.wav");
static SOUND_DIGITAL: &[u8] = include_bytes!("../sounds/mixkit-digital-quick-tone-2866.wav");
static SOUND_HAPPY_BELLS: &[u8] =
    include_bytes!("../sounds/mixkit-happy-bells-notification-937.wav");
static SOUND_HARP: &[u8] =
    include_bytes!("../sounds/mixkit-arabian-mystery-harp-notification-2489.wav");
static SOUND_POP: &[u8] = include_bytes!("../sounds/mixkit-long-pop-2358.wav");
static SOUND_POSITIVE: &[u8] = include_bytes!("../sounds/mixkit-positive-notification-951.wav");
static SOUND_INTERFACE: &[u8] =
    include_bytes!("../sounds/mixkit-software-interface-start-2574.wav");

struct GeneratedTone {
    frequency: f32,
    duration: Duration,
}

const GENERATED_TONES: &[(&str, GeneratedTone)] = &[
    (
        "ping",
        GeneratedTone {
            frequency: 880.0,
            duration: Duration::from_millis(100),
        },
    ),
    (
        "alert",
        GeneratedTone {
            frequency: 440.0,
            duration: Duration::from_millis(200),
        },
    ),
    (
        "low",
        GeneratedTone {
            frequency: 220.0,
            duration: Duration::from_millis(200),
        },
    ),
    (
        "chime",
        GeneratedTone {
            frequency: 659.0,
            duration: Duration::from_millis(150),
        },
    ),
    (
        "beep",
        GeneratedTone {
            frequency: 523.0,
            duration: Duration::from_millis(100),
        },
    ),
    (
        "notify",
        GeneratedTone {
            frequency: 587.0,
            duration: Duration::from_millis(120),
        },
    ),
];

/// All built-in sound names: WAV sounds first, then generated tones.
/// Order matches the Go version for settings dropdown compatibility.
static BUILTIN_SOUNDS: &[&str] = &[
    "Bell",
    "Confirmation",
    "Correct",
    "Digital",
    "Happy Bells",
    "Harp",
    "Pop",
    "Positive",
    "Interface",
    "tone:ping",
    "tone:alert",
    "tone:low",
    "tone:chime",
    "tone:beep",
    "tone:notify",
];

fn wav_data_for_name(name: &str) -> Option<&'static [u8]> {
    match name {
        "Bell" => Some(SOUND_BELL),
        "Confirmation" => Some(SOUND_CONFIRMATION),
        "Correct" => Some(SOUND_CORRECT),
        "Digital" => Some(SOUND_DIGITAL),
        "Happy Bells" => Some(SOUND_HAPPY_BELLS),
        "Harp" => Some(SOUND_HARP),
        "Pop" => Some(SOUND_POP),
        "Positive" => Some(SOUND_POSITIVE),
        "Interface" => Some(SOUND_INTERFACE),
        _ => None,
    }
}

fn tone_for_name(name: &str) -> Option<&'static GeneratedTone> {
    GENERATED_TONES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, t)| t)
}

/// Returns true if the name refers to a built-in sound (WAV or tone).
#[allow(dead_code)]
pub fn is_builtin(name: &str) -> bool {
    BUILTIN_SOUNDS.contains(&name)
}

/// Play a sound by name or file path. Non-blocking — spawns playback on a background thread.
/// name can be a WAV sound name (e.g. "Bell"), a generated tone (e.g. "tone:ping"),
/// or an absolute file path to an audio file (WAV, MP3, OGG, FLAC).
pub fn play_sound(name: &str) {
    if name.is_empty() || name == "none" {
        return;
    }

    // Resolve what to play before spawning the thread
    if let Some(tone_name) = name.strip_prefix("tone:") {
        if let Some(tone) = tone_for_name(tone_name) {
            let freq = tone.frequency;
            let dur = tone.duration;
            std::thread::spawn(move || {
                play_tone_blocking(freq, dur);
            });
        } else {
            tracing::warn!("Unknown generated tone: {}", tone_name);
        }
        return;
    }

    if let Some(data) = wav_data_for_name(name) {
        std::thread::spawn(move || {
            play_wav_blocking(data);
        });
        return;
    }

    // Try as file path
    let path = Path::new(name);
    if path.is_file() {
        let path = path.to_path_buf();
        std::thread::spawn(move || {
            play_file_blocking(&path);
        });
        return;
    }

    tracing::warn!("Unknown sound: {}", name);
}

fn play_wav_blocking(data: &'static [u8]) {
    let (_stream, handle) = match OutputStream::try_default() {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!("Failed to open audio output: {}", e);
            return;
        }
    };
    let sink = match Sink::try_new(&handle) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to create audio sink: {}", e);
            return;
        }
    };
    let cursor = Cursor::new(data);
    match Decoder::new(cursor) {
        Ok(source) => {
            sink.append(source);
            sink.sleep_until_end();
        }
        Err(e) => {
            tracing::error!("Failed to decode WAV: {}", e);
        }
    }
}

fn play_file_blocking(path: &Path) {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("Failed to open sound file {:?}: {}", path, e);
            return;
        }
    };
    let (_stream, handle) = match OutputStream::try_default() {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!("Failed to open audio output: {}", e);
            return;
        }
    };
    let sink = match Sink::try_new(&handle) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to create audio sink: {}", e);
            return;
        }
    };
    match Decoder::new(BufReader::new(file)) {
        Ok(source) => {
            sink.append(source);
            sink.sleep_until_end();
        }
        Err(e) => {
            tracing::error!("Failed to decode audio file {:?}: {}", path, e);
        }
    }
}

fn play_tone_blocking(frequency: f32, duration: Duration) {
    let (_stream, handle) = match OutputStream::try_default() {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!("Failed to open audio output: {}", e);
            return;
        }
    };
    let sink = match Sink::try_new(&handle) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to create audio sink: {}", e);
            return;
        }
    };
    let source = SineWave::new(frequency)
        .take_duration(duration)
        .amplify(0.3);
    sink.append(source);
    sink.sleep_until_end();
}

/// Returns the list of all built-in sound names (for settings dropdown).
pub fn builtin_sounds() -> &'static [&'static str] {
    BUILTIN_SOUNDS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_sounds_list() {
        let sounds = builtin_sounds();
        assert_eq!(sounds.len(), 15);
        assert_eq!(sounds[0], "Bell");
        assert_eq!(sounds[8], "Interface");
        assert_eq!(sounds[9], "tone:ping");
        assert_eq!(sounds[14], "tone:notify");
    }

    #[test]
    fn test_builtin_sounds_unique() {
        let sounds = builtin_sounds();
        let mut seen = std::collections::HashSet::new();
        for s in sounds {
            assert!(seen.insert(s), "Duplicate sound name: {}", s);
        }
    }

    #[test]
    fn test_wav_data_for_name() {
        assert!(wav_data_for_name("Bell").is_some());
        assert!(wav_data_for_name("Interface").is_some());
        assert!(wav_data_for_name("Unknown").is_none());
    }

    #[test]
    fn test_tone_for_name() {
        let tone = tone_for_name("ping").unwrap();
        assert_eq!(tone.frequency, 880.0);
        assert_eq!(tone.duration, Duration::from_millis(100));
        assert!(tone_for_name("unknown").is_none());
    }

    #[test]
    fn test_is_builtin() {
        assert!(is_builtin("Bell"));
        assert!(is_builtin("tone:ping"));
        assert!(!is_builtin(""));
        assert!(!is_builtin("/path/to/file.wav"));
        assert!(!is_builtin("unknown"));
    }
}
