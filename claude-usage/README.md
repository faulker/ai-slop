# claude-usage

Fetches Claude Code usage data via the Anthropic OAuth API.

## How it works

1. Reads the OAuth access token from the macOS Keychain (`Claude Code-credentials`)
2. Calls `GET https://api.anthropic.com/api/oauth/usage` with the token
3. Prints the JSON response to stdout

## Usage

```sh
claude-usage
```

Pipe through `jq` for pretty output:

```sh
claude-usage | jq .
```

## Requirements

- macOS (uses the Keychain)
- Claude Code CLI logged in (stores OAuth credentials in Keychain)

## Building

```sh
cargo build --release
```

The binary will be at `target/release/claude-usage`.

## Testing

```sh
cargo test
```
