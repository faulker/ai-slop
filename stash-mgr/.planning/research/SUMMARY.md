# Research Summary: stash-mgr

**Domain:** Rust TUI application for git stash management
**Researched:** 2026-02-11
**Overall confidence:** MEDIUM-HIGH

## Executive Summary

The Rust TUI ecosystem has a stable, well-supported stack for this project. The recommended stack centers on **ratatui** + **crossterm** (TUI) and **git2** (git operations) with a **hybrid approach** — git2 for mutations, git CLI for diff display.

The architecture follows the Elm Architecture pattern (Event → Update → View) with a single-threaded synchronous design. No async runtime needed.

## Key Findings

**Stack:** ratatui + crossterm + git2 + thiserror/anyhow. No async runtime.

**Table Stakes:** List stashes, apply/pop/drop, diff preview, file-level selective stash, keyboard navigation, stash message input.

**Key Differentiator:** Hunk-level selective stashing with visual UI. This is what makes stash-mgr worth using over lazygit/gitui stash panels.

**Architecture:** TEA pattern. Single App struct owns all state. Git backend abstracted behind GitRepo struct. Module split: main / app / event / ui/ / git/ / types.

**Watch Out For:**
- Terminal state not restored on panic (set panic hook early)
- Git index lock contention (single Repository per operation)
- Binary files crashing diff parser (check for binary, use lossy UTF-8)
- Unbounded memory from large diffs (virtual scrolling, size limits)
- Race conditions between preview and stash creation (validate before write)

## Implications for Roadmap

Based on research, suggested phase structure:

1. **Foundation: TUI scaffold + basic git integration**
   - Rationale: Establish the ratatui app structure and verify git2 can list/show stashes
   - Addresses: Core framework setup, git repository detection, basic stash listing
   - Avoids: Diving into complex diff parsing before basic UI works
   - Risk: LOW - Well-trodden path with extensive examples

2. **Stash Browser Tab**
   - Rationale: Read-only operations are simpler than write operations
   - Addresses: List stashes, preview diffs, apply/delete actions
   - Avoids: Complex working directory scanning (deferred to creation tab)
   - Risk: LOW - git2 has straightforward APIs for these operations

3. **Stash Creation Tab**
   - Rationale: Requires working directory diff parsing and hunk selection UI
   - Addresses: Working directory status, file-level selection, hunk-level selection
   - Avoids: Rushing into hunk selection before file-level works
   - Risk: MEDIUM - Hunk-level selection requires careful UI design

4. **Polish & Cross-Platform Testing**
   - Rationale: Verify behavior on non-macOS platforms
   - Addresses: Windows/Linux compatibility, error handling, edge cases
   - Avoids: Assuming cross-platform works without testing
   - Risk: LOW - Stack is inherently cross-platform

**Phase ordering rationale:**
- Browser before creation: Read operations before write operations reduces risk
- Foundation first: Need working TUI before adding features
- Polish last: Can't test edge cases until core features exist

**Research flags for phases:**
- Phase 1: Unlikely to need research (standard TUI setup)
- Phase 2: Unlikely to need research (standard git2 usage)
- Phase 3: MAY need research on hunk parsing strategies (CLI vs libgit2 diff API)
- Phase 4: Unlikely to need research (testing is execution, not research)

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | MEDIUM | Libraries are correct but versions unverified with crates.io |
| Features | HIGH | Git stash operations are well-defined |
| Architecture | HIGH | TUI patterns are well-established in Rust ecosystem |
| Pitfalls | HIGH | Known gotchas from ecosystem experience and project memory |

## Gaps to Address

- **Version verification:** All crate versions must be verified against crates.io before use
- **Breaking changes:** Check changelogs for ratatui, crossterm, git2, thiserror v2
- **Platform testing:** macOS is primary but Windows/Linux behavior should be validated
- **Hunk parsing strategy:** Phase 3 may need deeper research into diff parsing approaches

## Verification Required

Due to unavailability of external verification tools during this research session:

1. Visit https://crates.io and verify current versions of:
   - ratatui (listed as 0.29+)
   - crossterm (listed as 0.28+)
   - git2 (listed as 0.19+)
   - thiserror (listed as 2.0+)

2. Check compatibility:
   - ratatui + crossterm version compatibility matrix
   - git2 platform-specific requirements for macOS

3. Review recent changes:
   - ratatui breaking changes since 0.29
   - thiserror v2 migration guide (v2 was recent relative to training cutoff)

4. Confirm no major ecosystem shifts since January 2025

## Research Quality Notes

This research was conducted under constraints:
- WebSearch: Unavailable (could not verify current best practices)
- WebFetch: Unavailable (could not access official documentation)
- Context7: Unavailable (could not query library-specific docs)
- Read: Unavailable (could not access local template files)

Recommendations are based on:
- Training data knowledge of Rust TUI ecosystem (January 2025 cutoff)
- Project memory patterns from similar Rust projects (TradeWatch stack experience)
- Established Rust community conventions

Despite these constraints, confidence remains MEDIUM-to-HIGH because:
- Rust TUI ecosystem is relatively stable (ratatui is established standard)
- git2 is the only viable option for programmatic git access in Rust
- Error handling patterns (thiserror/anyhow) are ecosystem-wide standards
- Architectural recommendations are based on fundamental TUI design patterns, not library-specific features

The primary risk is version drift - libraries may have released breaking changes between January 2025 and February 2025. Version verification is the critical next step.
