# claude-usage

Reports how much of your Claude usage limits you've consumed, so you can tell when you're running low. It reads two sources: Claude Desktop's local history file (no authentication, no network) and the Anthropic OAuth usage API (richer, but needs the Keychain token).

The default path never touches the Keychain, so it never triggers a macOS password prompt and is safe to call from a status bar, shell prompt, or script.

## How it works

`--source auto` (the default):

1. Read `~/Library/Application Support/Claude/plan-usage-history.json`, the file Claude Desktop writes for its own usage display. If its newest sample is younger than `--max-age`, print it and stop. No Keychain, no network.
2. Otherwise fall back to `GET https://api.anthropic.com/api/oauth/usage`, using the OAuth token from the Keychain item `Claude Code-credentials`.
3. If the API also fails, print the stale local data with a warning on stderr rather than failing.

## Sources compared

|                         | `local`                          | `api`                        |
| ----------------------- | -------------------------------- | ---------------------------- |
| 5-hour and 7-day used % | yes                              | yes                          |
| Extra-usage dollars     | yes, when extra usage is enabled | yes                          |
| Reset times             | estimated (see below)            | authoritative, plus the same estimate as a supplement |
| Needs the Keychain      | no                               | yes                          |
| Freshness               | updates every ~15-18 min while Claude Desktop runs; goes stale when it's closed | live |

Claude Desktop appends a sample roughly every 15-18 minutes, which is why `--max-age` defaults to 30 minutes.

There's no per-model (Opus/Sonnet) breakdown from either source: the live API response doesn't return one for this tool's accounts (verified against a live response — `seven_day_opus`/`seven_day_sonnet` are always `null`, and the `limits` array has no per-model entries either), so it isn't part of the output at all.

### Estimated reset times

The local file has no reset times, but window boundaries are visible in the sample history: the 5-hour percentage either rises from 0 or drops mid-use (`79` to `1`). The most recent boundary is treated as the current window's start, so it resets five hours later, reported as `five_hour.resets_at_estimated_ms` and labeled `(est)` in text output.

This estimate is computed from the local history regardless of which source answered the request. On the api source it's a supplement alongside the authoritative `resets_at` (which text output prefers, so `(est)` only shows up there in JSON, not on screen); if no local history is available it's just `null` there, same as if local had never been checked.

The estimate is omitted rather than guessed when there's no boundary in the history, when a sampling gap wider than 20 minutes could be hiding one, or when the computed reset has already elapsed. A window at 0% used is still running and still gets an estimate; it's not treated as "no window". It is never merged into `resets_at`, which stays `null` on the local source.

## Usage

```sh
claude-usage                       # auto: local if fresh, else the API
claude-usage --source local        # guaranteed no Keychain, no network
claude-usage --source api          # always live
claude-usage --format json | jq .  # normalized JSON
claude-usage --format raw | jq .   # the untouched API body (implies --source api)
```

```
5h window   79% used   resets in ~1h 20m (est)
7d window   18% used
source: local, sampled 4m ago
note: reset time is estimated
```

Options:

| Flag                        | Default | Meaning                                              |
| --------------------------- | ------- | ----------------------------------------------------- |
| `--source auto\|local\|api` | `auto`  | Where to read usage from                             |
| `--max-age <MINUTES>`       | `30`    | How old a local sample may be before it counts stale |
| `--format text\|json\|raw`  | `text`  | Output format                                        |

`--format raw` always uses the API regardless of `--source` (it prints the API's own response body, so there's nothing to print for `local`); pairing it explicitly with `--source local` is rejected.

Because `auto` is the default even for `--format json`, JSON output commonly comes from the local file, and `five_hour.resets_at` will then be `null` (only the api source has an authoritative reset time; see "Sources compared" above). `five_hour.resets_at_estimated_ms` is populated either way when local history is available. Pass `--source api` explicitly when a caller needs the authoritative reset time every time.

## Using this from other apps

Shell out to this and parse stdout:

```sh
claude-usage --source local --format json
```

That exact command never reads the Keychain, never hits the network, and never blocks on a dialog. The contract:

- Exit 0 with a complete JSON object on success, including when the data is stale (`"stale": true` says so, and the warning goes to stderr, not stdout).
- Non-zero exit with a message on stderr when no local data is available, so you can detect it without inspecting the shape.
- Keys are additive-only. `null` always means "this source does not report the field", never "zero".

```json
{
  "source": "local",
  "sampled_at_ms": 1784991027660,
  "age_seconds": 203,
  "stale": false,
  "five_hour": { "used_percentage": 51.0, "resets_at": null, "resets_at_estimated_ms": 1785007827659 },
  "seven_day": { "used_percentage": 23.0, "resets_at": null, "resets_at_estimated_ms": null },
  "extra_usage_dollars": null
}
```

`resets_at` is the API's authoritative ISO timestamp, passed through as an opaque string; it's `null` on the local source. `resets_at_estimated_ms` is derived from the local history, in epoch milliseconds, and is populated on both sources whenever that history is available — it's not exclusive to the local source.

## Avoiding the Keychain password prompt

macOS prompts for your login password whenever this reads `Claude Code-credentials`, because builds are ad-hoc (linker) signed. The binary's cdhash changes on every `cargo build`, so the ACL entry created by clicking "Always Allow" no longer matches and the prompt comes back. Options, easiest first:

1. Use the default `auto`, or `--source local`, so the Keychain is never read at all.
2. Set `CLAUDE_USAGE_TOKEN` to an OAuth access token, which takes precedence over the Keychain.
3. Sign the binary with a stable identity so the ACL sticks across rebuilds. Create a self-signed Code Signing certificate in Keychain Access (Certificate Assistant → Create a Certificate, type "Code Signing"), then after each build run:

   ```sh
   codesign --force --sign "Claude Usage Dev" --identifier me.faulk.claude-usage target/release/claude-usage
   ```

   This is a manual note on purpose; it isn't wired into `build.rs` or any cargo config.

## Environment

| Variable                    | Effect                                     |
| --------------------------- | ------------------------------------------ |
| `CLAUDE_USAGE_TOKEN`        | OAuth token to use instead of the Keychain |
| `CLAUDE_USAGE_HISTORY_FILE` | Override the local history file path       |

## Requirements

- macOS. The local source needs Claude Desktop installed and run at least once; the api source needs Claude Code logged in.

## Building

```sh
cargo build --release
```

The binary will be at `target/release/claude-usage`.

For a status bar or anything else that runs it long-term, install it instead so `cargo clean` can't delete it out from under you:

```sh
./install.sh   # builds, copies to ~/.local/bin/claude-usage, and signs it
```

It signs with the first code-signing identity matching `Apple Development` (override with `CLAUDE_USAGE_SIGN_IDENTITY`, a name substring or SHA-1 hash, e.g. `"Claude Usage Dev"` for the self-signed cert from option 3) so the Keychain "Always Allow" sticks across reinstalls. With an Apple-issued cert it even survives renewing the cert, since the requirement is keyed on the cert's common name, not its hash. If that identity doesn't exist it installs unsigned and warns. `CLAUDE_USAGE_INSTALL_DIR` changes the destination. Re-run it after code changes.

## Testing

```sh
cargo test
```

The suite is fully offline and never prompts for a password: history fixtures are written to a temp directory, and every integration test runs with a `USER` value that cannot match a real Keychain item. One Keychain test is `#[ignore]`d for the same reason; run it with `cargo test -- --ignored` if you want to check real credential access.
