# claude-usage

I created this app to get the usage info for my Claude account because there is no API for doing that for Pro/Max accounts.

It extracts the `sessionKey` and `lastActiveOrg` cookies from the Brave browser on macOS, then calls the Claude.ai usage endpoint and prints the JSON response to stdout.

## How it works

1. Reads the Brave encryption key from the macOS Keychain
2. Opens the Brave Cookies SQLite database (copies it to a temp file to avoid lock contention)
3. Decrypts the `sessionKey` and `lastActiveOrg` cookies for `claude.ai`
4. Makes a GET request to `https://claude.ai/api/organizations/{lastActiveOrg}/usage`
5. Prints the JSON response

## Usage

```sh
# Default — uses Brave's default Cookies database path
claude-usage

# Custom database path
claude-usage --db /path/to/Cookies
```

## Requirements

- macOS (uses the Keychain and Brave's cookie encryption format)
- Brave browser with an active Claude.ai session

## Building

```sh
cargo build --release
```

The binary will be at `target/release/claude-usage`.
