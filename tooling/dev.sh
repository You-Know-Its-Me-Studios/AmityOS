#!/bin/bash
# Run a command inside the AmityOS build root (same Ubuntu release as the image,
# with the development libraries). Runs as root inside WSL.
#   dev.sh setup            build or rebuild the build root with mkosi
#   dev.sh cargo ARGS...    run the pinned cargo in the workspace
#   dev.sh run CMD ARGS...  run any command in the build root
#   dev.sh export PROFILE   copy built binaries (debug|release) to image/out/bin on F:
#   dev.sh stage            lay out the stripped release binaries for the image build
#   dev.sh metadata         write the dependency graph to .cache/tmp/cargo-metadata.json
# The workspace is /mnt/f/AmityOS; Cargo's target directory is on the work
# filesystem (/work/target inside the build root), which is a file on F:.
set -euo pipefail
. "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/work_env.sh"

DEVROOT="$WORK/devroot/amityos-devroot"
STAMP="$WORK/devroot/config.sha256"

config_hash() { (cd "$ROOT/tooling/devroot" && find . -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1); }

build_devroot() {
    sync_config "$ROOT/tooling/devroot" "$WORK/devroot-src"
    mkdir -p "$WORK/devroot"
    local tools=()
    # Reuse the image build's tools tree when it exists.
    [ -d "$WORK/src/mkosi.tools/usr" ] && tools=(--tools-tree "$WORK/src/mkosi.tools")
    [ ${#tools[@]} -eq 0 ] && tools=(--tools-tree=yes --tools-tree-distribution ubuntu --tools-tree-release resolute)
    (cd "$WORK/devroot-src" && run_mkosi "${tools[@]}" \
        --output-directory "$WORK/devroot" \
        --workspace-directory "$WORK/workspace" \
        --package-cache-dir "$WORK/cache/packages" \
        --force build)
    [ -x "$DEVROOT/usr/bin/cc" ] || { echo "Build root is incomplete: $DEVROOT"; exit 1; }
    config_hash > "$STAMP"
    echo "BUILD ROOT READY: $DEVROOT"
}

enter() {
    local resolv
    resolv=$(readlink -f /etc/resolv.conf)
    mkdir -p "$WORK/target" "$WORK/devhome"
    DEVROOT="$DEVROOT" WORK="$WORK" ROOT="$ROOT" RESOLV="$resolv" \
    unshare --mount --propagation private bash -euo pipefail -c '
        for d in proc sys dev run tmp work mnt/f/AmityOS root; do mkdir -p "$DEVROOT/$d"; done
        mount -t proc proc "$DEVROOT/proc"
        mount --rbind /sys "$DEVROOT/sys"
        mount --rbind /dev "$DEVROOT/dev"
        mount -t tmpfs tmpfs "$DEVROOT/run"
        mount -t tmpfs tmpfs "$DEVROOT/tmp"
        mount --bind "$WORK" "$DEVROOT/work"
        mount --bind "$ROOT" "$DEVROOT/mnt/f/AmityOS"
        mount --bind "$WORK/devhome" "$DEVROOT/root"
        # WSLg display sockets, when present, for running the nested compositor.
        if [ -d /mnt/wslg ]; then mkdir -p "$DEVROOT/mnt/wslg"; mount --rbind /mnt/wslg "$DEVROOT/mnt/wslg"; fi
        # The host X11 displays (WSLg). The host socket directory is read-only, and a
        # nested compositor adds a socket for its own XWayland, so the host sockets are
        # linked into a writable directory and marked as taken with a lock file.
        mkdir -p "$DEVROOT/tmp/.X11-unix"; chmod 1777 "$DEVROOT/tmp/.X11-unix"
        for socket in /mnt/wslg/.X11-unix/X*; do
            [ -S "$socket" ] || continue
            ln -s "$socket" "$DEVROOT/tmp/.X11-unix/"
            printf "%10d\n" 1 > "$DEVROOT/tmp/.${socket##*/}-lock"
        done
        cat "$RESOLV" > "$DEVROOT/run/resolv.conf"
        ln -sf /run/resolv.conf "$DEVROOT/etc/resolv.conf"
        exec chroot "$DEVROOT" /usr/bin/env -i \
            HOME=/root TERM="${TERM:-dumb}" LANG=C.UTF-8 \
            PATH=/mnt/f/AmityOS/.deps/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
            CARGO_HOME=/mnt/f/AmityOS/.deps/cargo RUSTUP_HOME=/mnt/f/AmityOS/.deps/rustup \
            CARGO_TARGET_DIR=/work/target TMPDIR=/tmp \
            XDG_RUNTIME_DIR=/mnt/wslg/runtime-dir WAYLAND_DISPLAY=wayland-0 DISPLAY=:0 \
            /bin/bash -c "cd /mnt/f/AmityOS && exec \"\$@\"" dev "$@"
    ' dev "$@"
}

# Everything runs from main, so that bash has read the whole file before it
# executes anything: editing this script while it runs cannot derail it.
main() {
    require_root_on_f
    mount_work
    command=${1:-}
    [ -n "$command" ] || { echo "usage: dev.sh setup | cargo ARGS... | run CMD ARGS... | export PROFILE | stage | metadata"; exit 2; }
    shift
    if [ "$command" = setup ] || [ ! -x "$DEVROOT/usr/bin/cc" ] || [ "$(cat "$STAMP" 2>/dev/null)" != "$(config_hash)" ]; then
        build_devroot
        [ "$command" = setup ] && exit 0
    fi
    case "$command" in
        cargo) enter cargo "$@" ;;
        export)
            profile=${1:-release}
            mkdir -p "$ROOT/image/out/bin"
            for name in amity-comp amity-shell amityd amity-spaces amity-apps amity-tasks; do
                if [ -x "$WORK/target/$profile/$name" ]; then
                    cp -f "$WORK/target/$profile/$name" "$ROOT/image/out/bin/$name"
                    echo "exported $name ($profile, $(du -h "$ROOT/image/out/bin/$name" | cut -f1))"
                fi
            done ;;
        stage)
            # Release binaries for the image, without debug information, laid out as
            # they are installed: $WORK/image-stage/usr/bin/<name>.
            rm -rf "$WORK/image-stage"
            enter sh -c 'set -e
                for name in amity-comp amity-shell; do
                    install -D -m 755 "/work/target/release/$name" "/work/image-stage/usr/bin/$name"
                    strip --strip-debug "/work/image-stage/usr/bin/$name"
                done'
            install -D -m 644 "$ROOT/assets/wallpapers/amity-default.png" \
                "$WORK/image-stage/usr/share/amityos/wallpapers/amity-default.png"
            chown -R root:root "$WORK/image-stage"
            (cd "$WORK/image-stage" && find . -type f -exec du -h {} +) ;;
        metadata)
            # The dependency graph for the licence ledger (tooling/licenses.py).
            mkdir -p "$ROOT/.cache/tmp"
            enter sh -c 'cargo metadata --format-version 1 --locked --filter-platform x86_64-unknown-linux-gnu \
                > /mnt/f/AmityOS/.cache/tmp/cargo-metadata.json'
            echo "wrote .cache/tmp/cargo-metadata.json" ;;
        run) enter "$@" ;;
        *) echo "unknown command: $command"; exit 2 ;;
    esac
}
main "$@"; exit
