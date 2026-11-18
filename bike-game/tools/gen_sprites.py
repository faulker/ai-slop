#!/usr/bin/env python3
"""Write Neon Run's pixel sprites.

Hand-authored grids, not traced from other games. Bikes are a rear view
(helmet, bars, one tire). Pills are capsules. Debris is a traffic cone.
Run from the repo: python3 tools/gen_sprites.py
"""

from __future__ import annotations

import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "assets" / "sprites"

# Limited night-city palette (RGBA). Keys are single chars in the grids.
PALETTE: dict[str, tuple[int, int, int, int]] = {
    ".": (0, 0, 0, 0),
    "0": (16, 16, 24, 255),  # near black
    "1": (24, 24, 32, 255),  # tire
    "2": (64, 72, 80, 255),  # concrete dark
    "3": (96, 104, 112, 255),  # concrete
    "4": (144, 152, 160, 255),  # concrete light
    "5": (20, 16, 42, 255),  # night navy
    "6": (26, 32, 64, 255),  # building base
    "7": (42, 48, 88, 255),  # building mid
    "8": (64, 232, 255, 255),  # cyan neon
    "9": (32, 160, 192, 255),  # cyan dim
    "a": (232, 160, 48, 255),  # amber
    "b": (255, 200, 80, 255),  # amber bright
    "c": (224, 24, 48, 255),  # hot red
    "d": (192, 16, 40, 255),  # red mid
    "e": (255, 40, 72, 255),  # red bright
    "f": (128, 16, 32, 255),  # red dark
    "g": (208, 24, 56, 255),  # jacket
    "h": (90, 48, 32, 255),  # seat
    "i": (58, 32, 16, 255),  # seat dark
    "j": (240, 240, 232, 255),  # pill white
    "k": (64, 128, 255, 255),  # pill blue
    "l": (12, 12, 16, 255),  # hair
    "m": (48, 72, 88, 255),  # enemy body
    "n": (32, 48, 64, 255),  # enemy dark
    "o": (104, 64, 160, 255),  # enemy accent
    "p": (224, 64, 160, 255),  # magenta neon
    "q": (232, 232, 240, 255),  # highlight
    "r": (192, 255, 255, 255),  # laser core
    "s": (96, 224, 255, 255),  # laser glow
    "t": (160, 168, 176, 255),  # pipe metal
    "u": (104, 112, 120, 255),  # pipe dark
    "v": (40, 56, 72, 255),  # enemy jacket
    "w": (72, 88, 104, 255),  # enemy fairing
    # Light grays so a multiply tint turns a gang bike that color and keeps the shading.
    "A": (236, 238, 246, 255),
    "B": (206, 208, 220, 255),
    "C": (150, 154, 168, 255),
    "D": (92, 96, 110, 255),
    "G": (36, 196, 96, 255),  # pill plus
    "H": (255, 244, 180, 255),  # sparkle
    "W": (240, 100, 36, 255),  # cone
    "X": (255, 248, 236, 255),  # cone band
    "Y": (176, 64, 28, 255),  # cone shade
    "K": (32, 156, 240, 255),  # pill blue
}


def sprite(width: int, parts: list[str]) -> list[str]:
    """Center each hand-authored row on a transparent canvas."""
    rows = []
    for part in parts:
        if len(part) > width:
            raise ValueError(f"row is {len(part)}px, canvas is {width}: {part}")
        if len(part) % 2:
            raise ValueError(f"odd row {len(part)}: {part}")
        pad = width - len(part)
        left = pad // 2
        rows.append("." * left + part + "." * (pad - left))
    return rows


def write_png(path: Path, width: int, height: int, rgba: bytes) -> None:
    """Write an RGBA8 PNG with no filtering (hard edges)."""

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    raw = bytearray()
    stride = width * 4
    for y in range(height):
        raw.append(0)  # filter None
        raw.extend(rgba[y * stride : (y + 1) * stride])

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b"")
    path.write_bytes(png)


def from_grid(rows: list[str]) -> tuple[int, int, bytes]:
    height = len(rows)
    width = len(rows[0])
    for row in rows:
        if len(row) != width:
            raise ValueError(f"ragged grid: expected {width}, got {len(row)}")
    out = bytearray(width * height * 4)
    for y, row in enumerate(rows):
        for x, ch in enumerate(row):
            if ch not in PALETTE:
                raise ValueError(f"unknown palette key {ch!r} at {x},{y}")
            r, g, b, a = PALETTE[ch]
            i = (y * width + x) * 4
            out[i : i + 4] = bytes((r, g, b, a))
    return width, height, bytes(out)


def save(name: str, rows: list[str]) -> None:
    w, h, rgba = from_grid(rows)
    path = OUT / name
    write_png(path, w, h, rgba)
    print(f"wrote {path.relative_to(ROOT)} ({w}x{h})")


# player_rear: outlined sport bike from behind. One rear tire, wide bars, tail lamp.
# Not a fused blob. Canonical red, never seed-tinted.
PLAYER = sprite(32, [
    "00000000",
    "00llllll00",
    "00llllllll00",
    "0ll0qqqq0ll0",
    "0ll0kkkk0ll0",
    "0ll0jjkk0ll0",
    "0ll0kkkk0ll0",
    "00llllllll00",
    "00eggggggf00",
    "0eggggggggggf0",
    "0eggqqggqqggf0",
    "00gggggggggg00",
    "000gggggggg000",
    "00j0gggggggg0j00",
    "0q40gggggggg04q0",
    "0400gggggggg0040",
    "0000hhhhhhhh0000",
    "00hhhhhhhhhh00",
    "000iiiiiiii000",
    "00dddddddddd00",
    "0dccccccccccd0",
    "0dceeeeeeeecd0",
    "0dcqqqqqqqqcd0",
    "0dccccccccccd0",
    "00dddddddddd00",
    "3300111111110033",
    "34401111qq110443",
    "3300111111110033",
    "3000111111110003",
    "0000111111110000",
    "0000011111000000",
])

# enemy_rear: same rear-view bike, light body so the seed tint colors the gang.
# Full-face visor and a spoiler, so it is not the red hero bike.
ENEMY = sprite(32, [
    "0000000000",
    "0DDDDDDDD0",
    "00DDAAAADD00",
    "0D55555555D0",
    "0D5qqqqqq5D0",
    "0D55555555D0",
    "00DDDDDDDD00",
    "00ABBBBBBBBC00",
    "0ABBBBBBBBBBC0",
    "0ABBBqqqqBBBC0",
    "00BBBBBBBBBB00",
    "0000BBBBBBBB0000",
    "00B0BBBBBBBB0B00",
    "0q40BBBBBBBB04q0",
    "0400BBBBBBBB0040",
    "00000000000000",
    "0DDDDDDDDDDDD0",
    "00BBBBBBBBBB00",
    "0BqqqqqqqqqqB0",
    "0BAAAAAAAAAAB0",
    "0BBBBBBBBBBBB0",
    "00BBBBBBBBBB00",
    "3300111111110033",
    "34401111qq110443",
    "3300111111110033",
    "3000111111110003",
    "0000111111110000",
    "0000011111000000",
])

# Traffic cone. Orange and white, not a rider and not a capsule.
DEBRIS = sprite(16, [
    "00",
    "0WW0",
    "0WXXW0",
    "0WWWWWW0",
    "0WWXXXXWW0",
    "0WWWWWWWW0",
    "0YYYYYYYY0",
    "000000000000",
    "022222222220",
    "002222222200",
])

# Red/blue capsule on a diagonal, black rim, white highlights.
PILL = sprite(26, [
    "...............00KKKKKKK00",
    ".............000KKKjjjjKK0",
    "...........000KKKjjjjjKKK0",
    "..........00KKKKjjjjKKKKK0",
    "........000KKKKKjjKKKKKKK0",
    ".......00ccKKKKjjKKKKKKK00",
    ".....000ccccKjjKKKKKKKK00.",
    "...000cccccccKKKKKKKKK00..",
    "..00cccjccjjcKKKKKKK000...",
    ".00cccjjjjccccKKKK000.....",
    "00ccjjjjjccccccKK00.......",
    "0ccjjjjcccccccc000........",
    "0ccjjjcccccccc00..........",
    "0cjjcccccccc000...........",
    "0ccccccccc000.............",
    "00ccccccc00...............",
])

# Outlined night facades. Signs and windows, not noisy speckles.
_WALL = "0666666666666660"
_FRAME = "0000000000000000"
_WIN = "060q080q080q0860"


def _facade(sign_a: str, sign_b: str, floors: int, accent_win: str) -> list[str]:
    parts = [_FRAME, _WALL, sign_a, sign_b, sign_a, _WALL]
    for i in range(floors):
        win = accent_win if i == floors - 1 else _WIN
        parts.append(win)
        parts.append(win)
        parts.append(_WALL)
    parts.extend([
        "0666600000666660",
        "06666aaaaaa66660",
        "0666600000666660",
        _FRAME,
        "2222222222222222",
    ])
    return sprite(24, parts)


BUILDING_A = _facade("06pppppppppppp60", "06p8qqqqqqqq8p60", 8, _WIN)
BUILDING_B = _facade("06aaaaaaaaaaaa60", "06abbbbbbbbbba60", 6, "060b080b080b0860")
BUILDING_C = _facade("0688888888888860", "068qqqqqqqqqq860", 10, "0608080808080860")

# Short forward beam, few pixels tall / bright core.
LASER = [
    "........",
    "..ssss..",
    ".srrrrs.",
    ".srrrrs.",
    ".srrrrs.",
    ".srrrrs.",
    ".srrrrs.",
    ".srrrrs.",
    ".srrrrs.",
    ".srrrrs.",
    "..ssss..",
    "...ss...",
    "....s...",
    "........",
    "........",
    "........",
    "........",
    "........",
    "........",
    "........",
]

# Pipe/bar for swing flash.
PIPE = [
    "................",
    "................",
    "..uuuttttttuu...",
    ".utttqqqtttttu..",
    ".utttttttttttu..",
    ".utttttttttttu..",
    "..uuuttttttuu...",
    "................",
]


def trim_bottom_empty(rows: list[str]) -> list[str]:
    """Drop trailing fully-transparent rows so sprites stay compact."""
    while rows and all(c == "." for c in rows[-1]):
        rows = rows[:-1]
    return rows


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    save("player_rear.png", trim_bottom_empty(PLAYER))
    save("enemy_rear.png", trim_bottom_empty(ENEMY))
    save("debris.png", trim_bottom_empty(DEBRIS))
    save("pill.png", trim_bottom_empty(PILL))
    save("building_a.png", trim_bottom_empty(BUILDING_A))
    save("building_b.png", trim_bottom_empty(BUILDING_B))
    save("building_c.png", trim_bottom_empty(BUILDING_C))
    save("laser.png", trim_bottom_empty(LASER))
    save("pipe.png", trim_bottom_empty(PIPE))


if __name__ == "__main__":
    main()
