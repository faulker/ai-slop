#!/usr/bin/env bash
# Build the hullr guest kernel and initramfs from Alpine packages.
#
#   vm/build.sh --hullr PATH [--arch aarch64|x86_64] [--alpine 3.22]
#               [--qemu-dir DIR] [--install [HULLR_HOME]]
#
# Needs only bash, curl, tar, gzip and python3 (no Docker, no root, no cpio),
# so it runs on Linux and macOS. Downloads go to vm/cache/, results to
# vm/out/<arch>/{kernel,initrd,versions.txt}.
#
#   --hullr PATH   the Linux `hullr` binary for the guest, e.g. from
#                  cargo build --release --target aarch64-unknown-linux-musl -p hullr-cli
#   --arch         guest architecture (default: the build host's)
#   --alpine       Alpine release branch (default 3.22); pin it for repeatable builds
#   --qemu-dir     directory of STATIC qemu-<arch> user binaries to include for
#                  foreign architectures. Alpine's qemu packages are dynamically
#                  linked, which breaks inside containers with binfmt "F", so
#                  they are not used.
#   --install      copy kernel and initrd to <HULLR_HOME>/vm (default ~/.hullr)
#
# Kernel: Alpine's linux-virt. On arm64 its vmlinuz is compressed (gzip or
# EFI zboot) but Virtualization.framework only boots a raw arm64 Image, so
# tools/kernel.py unpacks it. On x86_64 the bzImage is used as is.
#
# Environment: ALPINE_MIRROR (default https://dl-cdn.alpinelinux.org/alpine),
# SOURCE_DATE_EPOCH (cpio timestamps, default 0).
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
TOOLS="$HERE/tools"
MIRROR=${ALPINE_MIRROR:-https://dl-cdn.alpinelinux.org/alpine}
ALPINE=3.22
ARCH=
HULLR=
QEMU_DIR=
INSTALL=
INSTALL_ROOT=${HULLR_HOME:-$HOME/.hullr}

# Kernel modules the guest needs (dependencies are added automatically).
MODULES="virtio_pci virtio_mmio virtio_blk virtio_net virtio_console virtio_rng
virtio_balloon vsock vmw_vsock_virtio_transport fuse virtiofs overlay ext4
binfmt_misc tun af_packet veth bridge"
# Userland on top of the minirootfs (busybox, musl, apk-tools).
PACKAGES="e2fsprogs e2fsprogs-extra passt ca-certificates-bundle"

die() { echo "build.sh: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --arch) ARCH=$2; shift 2 ;;
        --hullr) HULLR=$2; shift 2 ;;
        --alpine) ALPINE=$2; shift 2 ;;
        --qemu-dir) QEMU_DIR=$2; shift 2 ;;
        --install)
            INSTALL=1; shift
            if [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; then INSTALL_ROOT=$1; shift; fi ;;
        -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
        *) die "unknown argument $1" ;;
    esac
done

if [ -z "$ARCH" ]; then
    case "$(uname -m)" in
        arm64|aarch64) ARCH=aarch64 ;;
        x86_64|amd64) ARCH=x86_64 ;;
        *) die "unsupported host arch $(uname -m); pass --arch" ;;
    esac
fi
case "$ARCH" in aarch64|x86_64) ;; *) die "--arch must be aarch64 or x86_64" ;; esac
[ -n "$HULLR" ] || die "--hullr PATH is required (the Linux hullr binary for $ARCH)"
[ -f "$HULLR" ] || die "$HULLR not found"
for t in curl tar gzip python3; do command -v $t >/dev/null || die "$t is required"; done

TAR_FLAGS=
tar --version 2>/dev/null | grep -q 'GNU tar' && TAR_FLAGS=--warning=no-unknown-keyword
py() { python3 -I "$@"; }

BRANCH=v$ALPINE
CACHE="$HERE/cache/$BRANCH/$ARCH"
OUT="$HERE/out/$ARCH"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/hullr-vm.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$CACHE" "$OUT"

fetch() { # url dest: download once into the cache
    [ -s "$2" ] && return 0
    echo "  fetch $1" >&2
    curl -fsSL --retry 3 -o "$2.part" "$1" || die "download failed: $1"
    mv "$2.part" "$2"
}

echo "==> Alpine $BRANCH $ARCH index"
REPOS="main community"
INDEXES=()
for repo in $REPOS; do
    fetch "$MIRROR/$BRANCH/$repo/$ARCH/APKINDEX.tar.gz" "$CACHE/APKINDEX-$repo.tar.gz"
    mkdir -p "$WORK/idx-$repo"
    tar -xzf "$CACHE/APKINDEX-$repo.tar.gz" -C "$WORK/idx-$repo" $TAR_FLAGS APKINDEX
    INDEXES+=("$WORK/idx-$repo/APKINDEX")
done
REPO_ARR=($REPOS)

echo "==> minirootfs"
fetch "$MIRROR/$BRANCH/releases/$ARCH/latest-releases.yaml" "$CACHE/latest-releases.yaml"
MINI=$(grep -o 'alpine-minirootfs-[0-9][0-9.]*-'"$ARCH"'\.tar\.gz' "$CACHE/latest-releases.yaml" | head -n1)
[ -n "$MINI" ] || die "no minirootfs listed in latest-releases.yaml"
fetch "$MIRROR/$BRANCH/releases/$ARCH/$MINI" "$CACHE/$MINI"
ROOT="$WORK/rootfs"
mkdir -p "$ROOT"
tar -xpzf "$CACHE/$MINI" -C "$ROOT" $TAR_FLAGS

fetch_pkg() { # "repo-index name version" -> path of the cached .apk
    local idx=$1 name=$2 ver=$3
    local file="$name-$ver.apk"
    fetch "$MIRROR/$BRANCH/${REPO_ARR[$idx]}/$ARCH/$file" "$CACHE/$file"
    echo "$CACHE/$file"
}

extract_apk() { # apk dir: data files only, no package metadata
    tar -xpzf "$1" -C "$2" $TAR_FLAGS --exclude='.PKGINFO' --exclude='.SIGN.*' \
        --exclude='.pre-*' --exclude='.post-*' --exclude='.trigger' 2>/dev/null \
        || tar -xpzf "$1" -C "$2" $TAR_FLAGS
    rm -f "$2"/.PKGINFO "$2"/.SIGN.* "$2"/.pre-* "$2"/.post-* "$2"/.trigger
}

echo "==> packages: $PACKAGES"
: > "$OUT/versions.txt.new"
echo "alpine $BRANCH $MINI" >> "$OUT/versions.txt.new"
while read -r idx name ver; do
    [ -n "$name" ] || continue
    apk=$(fetch_pkg "$idx" "$name" "$ver")
    extract_apk "$apk" "$ROOT"
    echo "$name $ver" >> "$OUT/versions.txt.new"
done < <(py "$TOOLS/apkindex.py" --installed "$ROOT/lib/apk/db/installed" "${INDEXES[@]}" -- $PACKAGES)

echo "==> kernel (linux-virt)"
read -r kidx kname kver < <(py "$TOOLS/apkindex.py" --no-deps "${INDEXES[@]}" -- linux-virt)
KAPK=$(fetch_pkg "$kidx" "$kname" "$kver")
KDIR="$WORK/kernel"
mkdir -p "$KDIR"
extract_apk "$KAPK" "$KDIR"
echo "linux-virt $kver" >> "$OUT/versions.txt.new"
py "$TOOLS/kernel.py" "$ARCH" "$KDIR/boot/vmlinuz-virt" "$OUT/kernel.new"

MODDIR=$(ls -d "$KDIR"/lib/modules/*/ | head -n1)
MODDIR=${MODDIR%/}
DEST="$ROOT/lib/modules/hullr"
mkdir -p "$DEST"
: > "$DEST/load.list"
# shellcheck disable=SC2086
for rel in $(py "$TOOLS/modules.py" "$MODDIR" $MODULES); do
    name=$(basename "$rel")
    case "$name" in
        *.ko.gz) gzip -dc "$MODDIR/$rel" > "$DEST/${name%.gz}"; name=${name%.gz} ;;
        *.ko.xz) xz -dc "$MODDIR/$rel" > "$DEST/${name%.xz}"; name=${name%.xz} ;;
        *.ko.zst) zstd -qdc "$MODDIR/$rel" > "$DEST/${name%.zst}"; name=${name%.zst} ;;
        *) cp "$MODDIR/$rel" "$DEST/$name" ;;
    esac
    echo "$name" >> "$DEST/load.list"
done

echo "==> hullr, init, extras"
install -m 0755 "$HULLR" "$ROOT/usr/bin/hullr"
install -m 0755 "$HERE/initramfs/init" "$ROOT/init"
install -m 0644 "$HERE/initramfs/inittab" "$ROOT/etc/inittab"
if [ -n "$QEMU_DIR" ]; then
    for q in "$QEMU_DIR"/qemu-*; do
        [ -f "$q" ] && install -m 0755 "$q" "$ROOT/usr/bin/$(basename "$q" | sed 's/-static$//')"
    done
fi
rm -rf "$ROOT/var/cache/apk"/* "$ROOT/usr/share/man" "$ROOT/usr/share/doc"

echo "==> initramfs"
py "$TOOLS/mkcpio.py" "$ROOT" "$WORK/initrd.cpio" --node dev/console:0600:5:1 --node dev/null:0666:1:3
gzip -9 -n -c "$WORK/initrd.cpio" > "$OUT/initrd.new"

mv "$OUT/kernel.new" "$OUT/kernel"
mv "$OUT/initrd.new" "$OUT/initrd"
mv "$OUT/versions.txt.new" "$OUT/versions.txt"
echo "built $OUT/kernel ($(wc -c < "$OUT/kernel") bytes) and $OUT/initrd ($(wc -c < "$OUT/initrd") bytes)"

if [ -n "$INSTALL" ]; then
    mkdir -p "$INSTALL_ROOT/vm"
    cp "$OUT/kernel" "$OUT/initrd" "$INSTALL_ROOT/vm/"
    echo "installed into $INSTALL_ROOT/vm"
fi
