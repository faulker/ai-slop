# Technology Stack

**Project:** stash-mgr
**Researched:** 2025-02-11
**Overall Confidence:** MEDIUM (based on training data only - external verification blocked)

## Recommended Stack

### Core TUI Framework
| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| ratatui | 0.29+ | Terminal UI framework | De facto standard for Rust TUI apps. Fork of tui-rs with active maintenance. Excellent widget ecosystem, stateless rendering model. |
| crossterm | 0.28+ | Terminal backend | Cross-platform (Windows/macOS/Linux), actively maintained, works seamlessly with ratatui. Supports all needed terminal operations (raw mode, events, cursor control). |

**Confidence:** MEDIUM - These are industry standard but versions need verification with crates.io

### Git Integration
| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| git2 | 0.19+ | Git operations (stash CRUD) | Rust bindings to libgit2. Battle-tested, comprehensive API for all git operations. Required for programmatic stash management. |
| Command execution fallback | N/A | Complex git diff parsing | For diff parsing, shelling out to `git diff` may be simpler than libgit2's diff API for getting patch text with proper formatting. |

**Confidence:** MEDIUM - git2 is standard but command fallback is architectural decision

**Rationale for hybrid approach:**
- `git2` for stash operations (create, list, apply, drop, show) - type-safe, no parsing needed
- Shell out to `git diff` for generating human-readable patches - avoids complexity of libgit2 diff callbacks
- This is a pragmatic split: use library for state-changing operations, use CLI for display operations

### Configuration & Parsing
| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| serde | 1.0+ | Serialization framework | Standard for Rust serialization/deserialization. |
| toml | 0.8+ | Config file parsing | If user config needed. TOML is Rust-native and human-friendly. |
| clap | 4.5+ | CLI argument parsing | Industry standard. Derive macros make it ergonomic. Only needed if CLI args beyond repo path. |

**Confidence:** HIGH - These are Rust ecosystem standards

### Error Handling
| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| thiserror | 2.0+ | Error type derivation | Zero-cost error type boilerplate reduction. Note: v2 reserves `{source}` in `#[error()]` macros. |
| anyhow | 1.0+ | Error propagation in main/tests | Ergonomic error handling for application code. Don't use in library code. |

**Confidence:** HIGH - Standard error handling stack, thiserror v2 caveat from project memory

### Development Tools
| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| cargo-watch | - | Dev workflow | Auto-rebuild on file changes. `cargo install cargo-watch` |
| cargo-nextest | - | Faster test runner | Parallel test execution, better output formatting. |

**Confidence:** MEDIUM - Common tools but optional

## Alternatives Considered

| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| TUI Framework | ratatui | cursive | cursive is callback-based which is harder to reason about for complex state. ratatui's stateless model is more predictable. |
| TUI Framework | ratatui | tui-rs | tui-rs is unmaintained. ratatui is its actively maintained fork. |
| Terminal Backend | crossterm | termion | termion doesn't support Windows. crossterm is cross-platform and more actively maintained. |
| Git Integration | git2 (hybrid) | Pure git2 | libgit2 diff API is complex for display purposes. Hybrid approach is more pragmatic. |
| Git Integration | git2 (hybrid) | Pure shell-out | Shelling out for stash mutations is fragile and harder to test. git2 provides type safety for state changes. |
| Error Handling | thiserror | manual impls | thiserror eliminates boilerplate without runtime cost. |

## NOT Recommended

| Technology | Why Avoid |
|------------|-----------|
| tokio / async runtime | TUI apps are event-driven but synchronous. Git operations are blocking. Async adds complexity without benefit. |
| termwiz | Less ecosystem support than crossterm. Primarily used by wezterm. |
| Direct termios | crossterm abstracts platform differences. Raw termios isn't cross-platform. |

## Installation

```bash
# Create new project (if not already created)
cargo new stash-mgr
cd stash-mgr

# Add core dependencies
cargo add ratatui
cargo add crossterm
cargo add git2
cargo add thiserror
cargo add anyhow

# Add optional dependencies
cargo add serde --features derive
cargo add toml  # if config file support needed
cargo add clap --features derive  # if CLI args needed

# Dev dependencies
cargo add --dev pretty_assertions  # better test output

# Dev tools
cargo install cargo-watch
cargo install cargo-nextest
```

## Cargo.toml Structure

```toml
[package]
name = "stash-mgr"
version = "0.1.0"
edition = "2021"
rust-version = "1.70"  # ratatui minimum

[dependencies]
ratatui = "0.29"
crossterm = "0.28"
git2 = "0.19"
thiserror = "2.0"
anyhow = "1.0"

[dev-dependencies]
pretty_assertions = "1.4"

[profile.release]
strip = true  # Remove debug symbols
lto = true    # Link-time optimization
codegen-units = 1  # Better optimization
```

## Architecture Notes

### Why This Stack Works for stash-mgr

1. **Stateless Rendering (ratatui):** Each frame is rendered from scratch based on app state. Perfect for a TUI with two tabs that need to react to git state changes.

2. **Event-Driven Model (crossterm):** Poll for terminal events (keypresses, resizes), update state, render. Natural fit for interactive stash browsing.

3. **Type-Safe Git Operations (git2):** Creating stashes with specific pathspecs, applying stashes, dropping stashes - all type-checked at compile time.

4. **Simple Diff Display (git CLI):** For showing patch previews, `git stash show -p stash@{n}` is simpler than navigating libgit2's diff callback API.

### Cross-Platform Considerations

- **crossterm** handles platform differences (Windows cmd.exe vs Unix terminals)
- **git2** wraps libgit2 which is cross-platform
- **ratatui** is platform-agnostic (works with any backend)

Primary target is macOS but stack supports Linux and Windows without code changes.

## Dependency Graph

```
stash-mgr
├── ratatui (TUI rendering)
│   └── crossterm (terminal backend)
├── git2 (stash operations)
├── thiserror (error types)
└── anyhow (error propagation in main)
```

## Version Verification Status

**IMPORTANT:** Versions listed here are based on training data (cutoff January 2025). Before implementation:

1. Verify current versions at https://crates.io
2. Check for breaking changes in release notes
3. Verify ratatui + crossterm compatibility (they release in sync)

## Confidence Assessment

| Area | Confidence | Reason |
|------|------------|--------|
| Core framework (ratatui) | MEDIUM | Industry standard but version unverified |
| Terminal backend (crossterm) | MEDIUM | Standard choice for ratatui but version unverified |
| Git library (git2) | HIGH | Only viable option for programmatic git access |
| Hybrid git approach | HIGH | Best practice from ecosystem experience |
| Error handling | HIGH | Standard Rust patterns |
| Async avoidance | HIGH | TUI apps don't benefit from async overhead |
| Cross-platform support | HIGH | All libraries are cross-platform |

## Sources

**Note:** External verification tools (WebSearch, WebFetch, Context7) were unavailable during research. Recommendations are based on:
- Training data knowledge of Rust TUI ecosystem (as of January 2025)
- Project memory notes on similar Rust projects
- Established Rust ecosystem patterns

**Action Required:** Verify all version numbers and check for 2025/2026 updates before implementation.

## Next Steps for Verification

When online verification is available:

1. Check crates.io for current versions of ratatui, crossterm, git2
2. Review ratatui changelog for breaking changes since 0.29
3. Verify crossterm 0.28+ compatibility with latest ratatui
4. Check git2 crate for any platform-specific gotchas on macOS
5. Confirm thiserror v2 is stable (was released recently relative to cutoff)
