//! Race audio: embedded WAV tones + pure step cue detection.
//!
//! Sounds are original generated PCM (see `tools/gen_audio.py`). Playback uses
//! `macroquad::audio`; if a load fails we log and keep running muted.

use macroquad::audio::{
    load_sound_from_bytes, play_sound, set_sound_volume, stop_sound, PlaySoundParams, Sound,
};

/// Which one-shots a single sim step should trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SoundCues {
    pub hit: bool,
    pub laser: bool,
}

/// Detect hit / laser from values visible on `Level` before and after `step`.
///
/// - Health drop => hit
/// - Laser cooldown 0 -> positive => laser (already-positive cooldown => no laser)
pub fn cues_from_step(
    prev_health: f32,
    prev_laser_cooldown: f32,
    new_health: f32,
    new_laser_cooldown: f32,
) -> SoundCues {
    SoundCues {
        hit: new_health < prev_health,
        laser: prev_laser_cooldown <= 0.0 && new_laser_cooldown > 0.0,
    }
}

/// Loaded (or missing) race sounds. Missing entries stay silent.
pub struct GameAudio {
    engine: Option<Sound>,
    hit: Option<Sound>,
    laser: Option<Sound>,
    music: Option<Sound>,
    engine_playing: bool,
    music_playing: bool,
}

impl GameAudio {
    /// Load embedded WAVs. Logs failures; never panics.
    pub async fn load() -> Self {
        Self {
            engine: try_load("engine", include_bytes!("../../../assets/audio/engine.wav")).await,
            hit: try_load("hit", include_bytes!("../../../assets/audio/hit.wav")).await,
            laser: try_load("laser", include_bytes!("../../../assets/audio/laser.wav")).await,
            music: try_load("music", include_bytes!("../../../assets/audio/music.wav")).await,
            engine_playing: false,
            music_playing: false,
        }
    }

    /// Play hit one-shot if the sound loaded.
    pub fn play_hit(&self) {
        if let Some(s) = &self.hit {
            play_sound(
                s,
                PlaySoundParams {
                    looped: false,
                    volume: 0.55,
                },
            );
        }
    }

    /// Play laser one-shot if the sound loaded.
    pub fn play_laser(&self) {
        if let Some(s) = &self.laser {
            play_sound(
                s,
                PlaySoundParams {
                    looped: false,
                    volume: 0.45,
                },
            );
        }
    }

    /// Start or keep engine + music loops for an active race.
    pub fn ensure_race_loops(&mut self) {
        if !self.engine_playing {
            if let Some(s) = &self.engine {
                play_sound(
                    s,
                    PlaySoundParams {
                        looped: true,
                        volume: 0.0,
                    },
                );
                self.engine_playing = true;
            }
        }
        if !self.music_playing {
            if let Some(s) = &self.music {
                play_sound(
                    s,
                    PlaySoundParams {
                        looped: true,
                        volume: 0.18,
                    },
                );
                self.music_playing = true;
            }
        }
    }

    /// Map player speed to engine volume (0 at standstill; louder near max).
    pub fn set_engine_from_speed(&self, speed: f32, max_speed: f32) {
        let Some(s) = &self.engine else {
            return;
        };
        if !self.engine_playing {
            return;
        }
        let denom = max_speed.max(1.0);
        let vol = (speed / denom).clamp(0.0, 1.0) * 0.4;
        set_sound_volume(s, vol);
    }

    /// Stop engine and music (title, pause, results, or leaving race).
    pub fn stop_race_loops(&mut self) {
        if self.engine_playing {
            if let Some(s) = &self.engine {
                stop_sound(s);
            }
            self.engine_playing = false;
        }
        if self.music_playing {
            if let Some(s) = &self.music {
                stop_sound(s);
            }
            self.music_playing = false;
        }
    }
}

/// Load one embedded WAV; log and return `None` on failure.
async fn try_load(name: &str, bytes: &'static [u8]) -> Option<Sound> {
    match load_sound_from_bytes(bytes).await {
        Ok(sound) => Some(sound),
        Err(e) => {
            eprintln!("audio: failed to load {name}.wav ({e}); continuing muted");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_drop_is_hit() {
        let c = cues_from_step(100.0, 0.0, 90.0, 0.0);
        assert!(c.hit);
        assert!(!c.laser);
    }

    #[test]
    fn no_health_drop_is_no_hit() {
        let c = cues_from_step(80.0, 0.0, 80.0, 0.0);
        assert!(!c.hit);
        assert!(!c.laser);
    }

    #[test]
    fn laser_cooldown_zero_to_positive_is_laser() {
        let c = cues_from_step(100.0, 0.0, 100.0, 0.6);
        assert!(!c.hit);
        assert!(c.laser);
    }

    #[test]
    fn laser_cooldown_already_positive_is_no_laser() {
        let c = cues_from_step(100.0, 0.3, 100.0, 0.2);
        assert!(!c.hit);
        assert!(!c.laser);
    }

    #[test]
    fn hit_and_laser_can_combine() {
        let c = cues_from_step(50.0, 0.0, 40.0, 0.6);
        assert!(c.hit);
        assert!(c.laser);
    }
}
