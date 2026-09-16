# humble-game-check

Lists unrevealed Humble Bundle Steam keys for games not already in your Steam library. Matches games by app id first, then by normalized name, then by fuzzy similarity to catch sequels and edition variants you may have forgotten you own.

```
NOT OWNED (1)
  Fixture Quest 3 https://www.humblebundle.com/downloads?key=fixtureorderA1

POSSIBLE MATCH, review (0)

LIKELY OWNED, edition or near-name match (1)
  Fixture Racer Deluxe https://www.humblebundle.com/downloads?key=fixtureorderB2   best: Fixture Racer (100002) 1.00

total unrevealed steam keys: 3  not owned: 1  possible: 0  likely owned: 1  owned: 1
```

## Setup

### Humble Session

Log into https://www.humblebundle.com in your browser, then extract the `_simpleauth_sess` cookie:

- **Chrome**: DevTools > Application > Cookies > https://www.humblebundle.com > copy the `_simpleauth_sess` value.
- **Firefox**: Storage tab > Cookies > https://www.humblebundle.com > copy the `_simpleauth_sess` value.
- **Safari**: Storage tab > Cookies > https://www.humblebundle.com > copy the `_simpleauth_sess` value.

This is a full login session. Keep it private, and note that it will expire. You may need to copy a fresh value if you see an auth error.

Pass it as `--humble-session` or set `HUMBLE_SESSION`:

```sh
export HUMBLE_SESSION='long_cookie_value_here'
```

If you run the tool without this (or the Steam values below), an interactive wizard walks you through copying each value and pasting it in. After that it asks whether to store them for next time.

### Steam API Key

1. Log into Steam as the account you want to check.
2. Go to https://steamcommunity.com/dev/apikey and generate a key (any domain name string works).

A friend's private or friends-only profile is only readable when the key belongs to that same account. If you get a "no games" error for another account, have them generate their own key, or ask them to set Privacy Settings > Game details > Public.

Pass the key as `--steam-key` or set `STEAM_API_KEY`:

```sh
export STEAM_API_KEY='your_steam_api_key_here'
```

### Steam Account

You need one of:

- `--steam-id` or `STEAM_ID` with your 17-digit SteamID64. Find it at https://steamcommunity.com/my/ (the number in the URL if your profile has no custom URL, or under Steam > Account details).
- `--steam-vanity` or `STEAM_VANITY` with the name from your profile URL (e.g. `yourvanityname` from https://steamcommunity.com/id/yourvanityname). This is resolved to a SteamID64 through the Steam API, so it only works when looking up your own account.

Flags override environment variables, which override a saved config file.

```sh
export STEAM_ID=7656119XXXXXXXXXX   # your 17-digit SteamID64
# or
export STEAM_VANITY=yourvanityname
```

## Build, run, and test

```sh
cargo build --release
cargo run --release -- --from-dump tests/fixtures/dump
cargo test
```

### Options

| Flag                        | Env var        | Meaning                                              |
| --------------------------- | -------------- | ---------------------------------------------------- |
| `--humble-session <VAL>`    | `HUMBLE_SESSION` | Humble Bundle login cookie                           |
| `--steam-key <VAL>`         | `STEAM_API_KEY` | Steam Web API key                                    |
| `--steam-id <ID>`           | `STEAM_ID` | Your SteamID64 (alternative to vanity name)          |
| `--steam-vanity <NAME>`     | `STEAM_VANITY` | Your custom profile name (resolved to SteamID64)    |
| `--format text\|json`       | (none)         | Output format (default: text)                        |
| `--show-owned`              | (none)         | Also show keys you already own on Steam              |
| `--dump-dir <DIR>`          | (none)         | Save raw API responses for offline use               |
| `--from-dump <DIR>`         | (none)         | Load from a saved dump instead of hitting the APIs   |

If any of the Humble/Steam values are missing and stdin is a terminal, a wizard prompts for them (how to get each value, then a place to paste it) and asks whether to save. Saved credentials go in `~/Library/Application Support/humble-game-check/config.json` on macOS, or `$XDG_CONFIG_HOME/humble-game-check/config.json` elsewhere. The file is mode `600`. Override the path with `HUMBLE_GAME_CHECK_CONFIG`. Set `HUMBLE_GAME_CHECK_WIZARD=never` to keep the old error-and-exit behavior in scripts.

## How matching works

Each key is compared against your Steam library in three tiers:

1. **Steam app ID exact match** (e.g. `12345` on both sides). This is definitive. If the Humble key includes a steam_app_id, it wins, even if the name looks wrong.

2. **Normalized name match**. Names are normalized by lowercasing, removing™ ® © symbols and apostrophes, replacing `&` with "and", and stripping edition phrases as complete words. Edition phrases are stripped longest-first and repeatedly until stable, never to empty: `game of the year edition`, `game of the year`, `goty edition`, `goty`, `definitive edition`, `deluxe edition`, `complete edition`, `ultimate edition`, `gold edition`, `enhanced edition`, `special edition`, `anniversary edition`, `legendary edition`, `directors cut`, `remastered`, `remaster`, `steam key`, `steam edition`, `edition`, `deluxe`, `definitive`, `complete`, `ultimate`.

   Numerals (`2`, `ii`, `iv`) and words like `collection`, `bundle`, `pack` are never stripped, so "Portal" and "Portal 2" never match, and "Game Collection" stays distinct from "Game".

   If the normalized Humble name exactly matches a Steam game's normalized name:
   - If the original names already matched, it's `OwnedName` (you definitely have it).
   - If normalization made them match (e.g. "Awesome Game 2 Deluxe" vs "Awesome Game 2"), it's `LikelyOwned` (visible in the output because it may be a different edition you need to verify).

3. **Fuzzy similarity** over all Steam game names using a combination of Levenshtein distance and token-based matching. Scores at or above 0.85 are `LikelyOwned`; 0.65 to 0.84 are `PossibleMatch`; below 0.65 are `NotOwned`. Sequel numbers must match (so "Portal" and "Portal 2" always score 0.0), but fuzzy matching catches titles like "Awesome Game 2 Ultra Pack" vs "Awesome Game 2" with a high score.

### Output sections

- **NOT OWNED**: Keys that don't match anything in your Steam library. These are the ones to redeem.
- **POSSIBLE MATCH, review**: Keys that scored 0.65+ but below 0.85. Check these manually against your library.
- **LIKELY OWNED, edition or near-name match**: Keys that matched via tier 2 (name normalization) or scored 0.85+. Probably duplicates, but visible because they might be a different edition.
- **OWNED** (shown only with `--show-owned`): Keys that matched exactly by app id or name (tier 1 or 2 with raw names equal).

Within each section, rows are padded so the URLs line up.

## Development

To iterate without hitting the APIs, save the raw responses:

```sh
HUMBLE_SESSION='...' STEAM_API_KEY='...' STEAM_VANITY=yourvanityname \
  cargo run --release -- --dump-dir dump
```

Then iterate offline:

```sh
cargo run -- --from-dump dump --show-owned
cargo run -- --from-dump dump --format json | jq .
```

The `tests/fixtures/dump/` directory is a small checked-in example with deterministic output, useful for regression tests.

## Privacy

The session cookie and Steam API key are never logged or printed. When matching, key values are never revealed; only keys that have already been redeemed on your account are considered (and thus already known). Dumps saved with `--dump-dir` contain your order history and game list, so treat them as sensitive: `dump/` and `.env` are in `.gitignore`. A saved config file holds the same secrets; keep it private.

## Limitations

- **Family Sharing**: The Steam API does not include games your account can access through Family Sharing. Only games owned directly appear.
- **DLC**: The Steam API returns DLC as separate entries. If a DLC is unrevealed in Humble but not in your Steam library, it may appear in `POSSIBLE MATCH` since the name alone won't match the base game.
- **Missing steam_app_id on Humble**: When Humble's `steam_app_id` is missing or is a package id instead of an app id, fuzzy name matching becomes the fallback. This is why `LIKELY OWNED` and `POSSIBLE MATCH` exist.
- **Delisted Steam apps**: Games removed from Steam have no name in the API, so they only match if the Humble key includes the correct app id.
- **Revealed but unactivated keys**: Keys you've already revealed (viewed) but not yet redeemed are out of scope. Only unrevealed keys are checked.
