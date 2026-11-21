#!/bin/bash
# Shared by image_build.sh and dev.sh. Sourced, not executed. Runs as root in WSL.
# Provides the Linux work filesystem (a loop-mounted ext4 file on F:) and a
# wrapper that runs the pinned mkosi with working name resolution.

ROOT=/mnt/f/AmityOS
MKOSI="$ROOT/.deps/mkosi-v27.1"
STORE="$ROOT/.cache/image-build"
WORK=/run/amityos-image
WORK_GIB=40

require_root_on_f() {
    [ "$(id -u)" = 0 ] || { echo "This script must run as root inside WSL (wsl -u root)"; exit 1; }
    [ -x "$MKOSI/bin/mkosi" ] || { echo "Pinned mkosi is missing: $MKOSI (see dependencies.lock.json)"; exit 1; }
    case "$(findmnt -n -o FSTYPE --target "$ROOT")" in 9p|drvfs) ;; *) echo "Expected $ROOT on the Windows F: drive"; exit 1 ;; esac
}

# A Linux root filesystem cannot be assembled on DrvFS, and Cargo's target
# directory is far faster on ext4, so both live in one ext4 file stored on F:.
mount_work() {
    mkdir -p "$STORE" "$WORK"
    if ! mountpoint -q "$WORK"; then
        if [ ! -f "$STORE/work.ext4" ]; then
            truncate -s "${WORK_GIB}G" "$STORE/work.ext4"
            mkfs.ext4 -q -F -m 0 -E lazy_itable_init=1,lazy_journal_init=1,nodiscard "$STORE/work.ext4"
        fi
        mount -o loop,noatime "$STORE/work.ext4" "$WORK"
    fi
    mkdir -p "$WORK"/{out,workspace,cache/incremental,cache/packages,tmp,home}
}

# sync_config SOURCE_DIR DEST_DIR: copy an mkosi configuration onto the work
# filesystem with normal permissions (DrvFS shows every file as 0777 and owned
# by the WSL user), keeping mkosi's tools tree in DEST_DIR.
sync_config() {
    local source=$1 dest=$2 stage="$WORK/config-stage"
    rm -rf "$stage"
    mkdir -p "$stage" "$dest"
    (cd "$source" && tar --exclude=./out --exclude=./README.md -cf - .) | tar -C "$stage" --no-same-owner --no-same-permissions -xf -
    find "$stage" -type d -exec chmod 755 {} +
    find "$stage" -type f -exec chmod 644 {} +
    while IFS= read -r -d '' file; do
        if [ "$(head -c 2 "$file")" = '#!' ]; then chmod 755 "$file"; fi
    done < <(find "$stage" -type f -print0)
    chown -R root:root "$stage"
    find "$dest" -mindepth 1 -maxdepth 1 ! -name 'mkosi.tools*' -exec rm -rf {} +
    cp -a "$stage/." "$dest/"
    rm -rf "$stage"
}

# run_mkosi ARGS...: run mkosi in the current directory. WSL's /etc/resolv.conf
# is a symlink into /mnt/wsl, which does not exist in mkosi's sandbox, so name
# resolution fails there. mkosi therefore runs in a private mount namespace
# where /etc is an overlay (upper layer on the work filesystem) and resolv.conf
# is a regular file. The WSL system itself is not changed.
run_mkosi() {
    rm -rf "$WORK/etc-upper" "$WORK/etc-work"
    mkdir -p "$WORK/etc-upper" "$WORK/etc-work"
    local resolv
    resolv=$(readlink -f /etc/resolv.conf)
    [ -s "$resolv" ] || { echo "No usable resolver configuration at $resolv"; exit 1; }
    HOME="$WORK/home" XDG_CACHE_HOME="$WORK/cache" TMPDIR="$WORK/tmp" PYTHONDONTWRITEBYTECODE=1 \
    MKOSI="$MKOSI" WORK="$WORK" RESOLV="$resolv" \
    unshare --mount --propagation private bash -euo pipefail -c '
        mount -t overlay overlay -o "lowerdir=/etc,upperdir=$WORK/etc-upper,workdir=$WORK/etc-work" /etc
        rm -f /etc/resolv.conf
        cat "$RESOLV" > /etc/resolv.conf
        exec "$MKOSI/bin/mkosi" "$@"
    ' mkosi "$@"
}
