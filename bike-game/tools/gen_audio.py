#!/usr/bin/env python3
"""Write original Neon Run PCM WAV tones into assets/audio/.

Sine / square / noise only — no samples from other media.
Run from repo: python3 tools/gen_audio.py
"""

from __future__ import annotations

import math
import struct
import wave
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "assets" / "audio"
RATE = 22050


def write_wav(path: Path, samples: list[float]) -> None:
    """Write mono 16-bit PCM WAV from float samples in [-1, 1]."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), "w") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        frames = bytearray()
        for s in samples:
            v = max(-1.0, min(1.0, s))
            frames += struct.pack("<h", int(v * 32767.0))
        w.writeframes(frames)


def sine(t: float, freq: float) -> float:
    return math.sin(2.0 * math.pi * freq * t)


def square(t: float, freq: float) -> float:
    return 1.0 if sine(t, freq) >= 0.0 else -1.0


def noise(i: int) -> float:
    # Deterministic hash noise (no stdlib random needed).
    x = (i * 1103515245 + 12345) & 0x7FFFFFFF
    return (x / 0x7FFFFFFF) * 2.0 - 1.0


def gen_engine() -> list[float]:
    """Short low rumble loop (~0.4 s)."""
    n = int(RATE * 0.4)
    out: list[float] = []
    for i in range(n):
        t = i / RATE
        # Fade ends so looping is less clicky.
        fade = 1.0
        edge = int(RATE * 0.02)
        if i < edge:
            fade = i / edge
        elif i > n - edge:
            fade = (n - i) / edge
        s = 0.45 * square(t, 55.0) + 0.25 * sine(t, 110.0) + 0.12 * noise(i)
        out.append(s * fade * 0.55)
    return out


def gen_hit() -> list[float]:
    """Short noise thump (~0.12 s)."""
    n = int(RATE * 0.12)
    out: list[float] = []
    for i in range(n):
        t = i / RATE
        env = math.exp(-t * 28.0)
        s = 0.7 * noise(i + 7) + 0.3 * sine(t, 180.0 - t * 900.0)
        out.append(s * env * 0.7)
    return out


def gen_laser() -> list[float]:
    """Short rising chirp (~0.1 s)."""
    n = int(RATE * 0.1)
    out: list[float] = []
    for i in range(n):
        t = i / RATE
        env = math.exp(-t * 18.0)
        freq = 880.0 + t * 2200.0
        s = 0.55 * square(t, freq) + 0.25 * sine(t, freq * 2.0)
        out.append(s * env * 0.5)
    return out


def gen_music() -> list[float]:
    """Quiet few-second loop: simple arpeggio of sines."""
    n = int(RATE * 4.0)
    # A minor-ish night pulse (Hz).
    notes = [220.0, 261.63, 329.63, 392.0, 329.63, 261.63]
    beat = RATE // 2  # 0.5 s per note
    out: list[float] = []
    for i in range(n):
        t = i / RATE
        ni = (i // beat) % len(notes)
        freq = notes[ni]
        # Soft attack within each beat.
        local = i % beat
        attack = min(1.0, local / (RATE * 0.03))
        release = 1.0
        if local > beat - int(RATE * 0.05):
            release = (beat - local) / (RATE * 0.05)
        bass = 0.18 * sine(t, freq * 0.5)
        lead = 0.22 * sine(t, freq)
        pad = 0.08 * sine(t, freq * 1.5)
        # Loop fade at ends.
        fade = 1.0
        edge = int(RATE * 0.05)
        if i < edge:
            fade = i / edge
        elif i > n - edge:
            fade = (n - i) / edge
        out.append((bass + lead + pad) * attack * release * fade * 0.35)
    return out


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    write_wav(OUT / "engine.wav", gen_engine())
    write_wav(OUT / "hit.wav", gen_hit())
    write_wav(OUT / "laser.wav", gen_laser())
    write_wav(OUT / "music.wav", gen_music())
    print(f"wrote WAVs to {OUT}")


if __name__ == "__main__":
    main()
