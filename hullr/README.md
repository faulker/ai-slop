# hullr

A lean, friendly container tool for macOS and Linux. It pulls images from Docker Hub, GHCR or any OCI registry, builds Dockerfiles, runs compose files, and runs foreign CPU architectures under emulation. You get a CLI, a terminal dashboard (`hullr tui`) and a desktop app (`hullr gui`), all talking to the same engine.

It works with or without a daemon. Every container is owned by a small shim process, so containers keep running after the command that started them exits. The daemon is optional and adds a shared API socket, healthchecks and restart-at-boot.

## Quick start

```sh
cargo build --release
./target/release/hullr run --rm -it alpine sh
./target/release/hullr run -d -p 8080:80 --name web nginx
./target/release/hullr ps
./target/release/hullr logs -f web
./target/release/hullr compose up -d        # in a folder with compose.yaml
./target/release/hullr tui
```

Commands and flags mirror Docker's (`run -d -it --rm -p -v -e --name --platform --restart`, `ps -a`, `logs -f`, `exec -it`, `build -t -f --build-arg --target`, `compose up/down/ps/logs/build/pull`), so existing habits carry over.

## How it works

```
hullr CLI ─┐
hullr TUI ─┼─> Api trait ─┬─> in-process engine (Linux, no daemon)
hullr GUI ─┘              └─> unix socket ─> hullr daemon ─> engine
```

- **Linux:** containers run natively with namespaces, overlayfs and a per-container shim. Rootless by default. Private networking and port publishing use `pasta` (or `slirp4netns`) when installed.
- **macOS:** a tiny Linux VM runs on Apple's Virtualization.framework, started on demand. Your home folder is shared into it at the same path, so bind mounts and build contexts just work. Published ports are forwarded to the Mac.
- **Emulation:** `--platform linux/arm64` (or amd64) selects the right image variant. On Linux, foreign binaries run through qemu-user via binfmt (`sudo hullr system emulation enable`). On Apple Silicon, amd64 uses Rosetta inside the VM.
- **Compose:** each project's services share one network sandbox, and service names resolve to `127.0.0.1`, so `db:5432` works from `web`.

## Daemon (optional)

```sh
hullr daemon start              # background, until reboot
hullr daemon install            # systemd user service (Linux) or launchd agent (macOS)
hullr daemon status | stop | uninstall
```

## Workspace layout

| Crate | What it does |
|---|---|
| `hullr-core` | Shared types and the `Api` trait every front end uses |
| `hullr-image` | Registry client (Docker Hub, GHCR, any v2 registry) and local image store |
| `hullr-runtime` | Linux runtime: namespaces, overlay, shim, networking, emulation |
| `hullr-build` | Dockerfile parser and builder with layer cache |
| `hullr-compose` | Compose file loading, interpolation and ordering |
| `hullr-engine` | Ties it all together behind `Api` |
| `hullr-api` | Daemon protocol (JSON lines over unix socket or vsock) and client |
| `hullr-vm` | macOS VM host and the in-VM guest agent |
| `hullr-tui` | ratatui dashboard |
| `hullr-gui` | egui desktop app |
| `hullr-cli` | The `hullr` binary |

`vm/` holds the scripts that build the macOS guest kernel and initramfs.

## Development

```sh
cargo test --workspace
cargo test -p hullr-image -- --ignored     # network tests (real registry pulls)
```

## Data

Everything lives under `$HULLR_HOME` (default `~/.hullr`, or `/var/lib/hullr` as root).
