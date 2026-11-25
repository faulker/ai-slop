# CLAUDE.md (hullr)

Lean Docker replacement in Rust for macOS and Linux. Cargo workspace under `crates/`.

## Commands
- `cargo test --workspace` (unit tests; no root or network needed)
- `cargo test -p hullr-image -- --ignored` (real registry pulls)
- `cargo test -p hullr-runtime -- --ignored --test-threads=1` (needs Linux, namespaces, network)
- `cargo build --release -p hullr-cli` builds the `hullr` binary
- `vm/build.sh` builds the macOS guest kernel + initramfs; `vm/sign.sh` adds the virtualization entitlement

## Architecture rules
- Front ends (CLI, TUI, GUI) only talk to `hullr_core::Api`. Never reach into the engine from them.
- `Api` has two impls: `hullr_engine::Engine` (in-process) and `hullr_api::Client` (socket). Any new operation needs: core trait method, engine impl, server dispatch arm, client method.
- No in-memory state that matters: container records, runtime state (owned by each shim), events and images are all files under the data root, written atomically, so several hullr processes can share it.
- `hullr_runtime::hidden_entry()` must run first in `main` (re-exec helpers: `__shim`, `__init`, `__exec`, `__sandbox`). The CLI also dispatches `__vm` and `__guest-agent` to `hullr-vm`.
- macOS never runs the engine locally; it always goes through the VM's socket.
- Keep it lean: no heavy runtime deps (no youki, no oci-client). Prefer small hand-written modules.
