#!/bin/bash
# Build the AmityOS disk image with the pinned mkosi. Runs as root inside WSL
# (AmityOS.ps1 image). Every byte stays on F:\AmityOS:
#   .cache/image-build/work.ext4   Linux filesystem for the build (loop-mounted;
#                                  a root filesystem cannot be assembled on DrvFS)
#   image/out/                     results: amityos-<version>.qcow2, manifest, hashes
set -euo pipefail

ROOT=/mnt/f/AmityOS
MKOSI="$ROOT/.deps/mkosi-v27.1"
STORE="$ROOT/.cache/image-build"
WORK=/run/amityos-image
OUT="$ROOT/image/out"
PROFILE="${AMITY_IMAGE_PROFILE-devel}"
WORK_GIB=40

[ "$(id -u)" = 0 ] || { echo "image_build.sh must run as root inside WSL (wsl -u root)"; exit 1; }
[ -x "$MKOSI/bin/mkosi" ] || { echo "Pinned mkosi is missing: $MKOSI (see dependencies.lock.json)"; exit 1; }
case "$(findmnt -n -o FSTYPE --target "$ROOT")" in 9p|drvfs) ;; *) echo "Expected $ROOT on the Windows F: drive"; exit 1 ;; esac

mkdir -p "$STORE" "$WORK" "$OUT"
if ! mountpoint -q "$WORK"; then
    if [ ! -f "$STORE/work.ext4" ]; then
        truncate -s "${WORK_GIB}G" "$STORE/work.ext4"
        mkfs.ext4 -q -F -m 0 -E lazy_itable_init=1,lazy_journal_init=1,nodiscard "$STORE/work.ext4"
    fi
    mount -o loop,noatime "$STORE/work.ext4" "$WORK"
fi
mkdir -p "$WORK"/{src,out,workspace,cache/incremental,cache/packages,tmp,home}

# Copy the configuration onto the Linux filesystem with normal permissions
# (DrvFS shows every file as 0777 and owned by the WSL user).
stage="$WORK/config-stage"
rm -rf "$stage"
mkdir -p "$stage" "$WORK/src"
(cd "$ROOT/image" && tar --exclude=./out --exclude=./README.md -cf - .) | tar -C "$stage" --no-same-owner --no-same-permissions -xf -
find "$stage" -type d -exec chmod 755 {} +
find "$stage" -type f -exec chmod 644 {} +
while IFS= read -r -d '' file; do
    if [ "$(head -c 2 "$file")" = '#!' ]; then chmod 755 "$file"; fi
done < <(find "$stage" -type f -print0)
chown -R root:root "$stage"
# Replace the configuration but keep mkosi's tools tree (mkosi.tools*, about
# 2.7 GB) so it is not rebuilt on every run.
find "$WORK/src" -mindepth 1 -maxdepth 1 ! -name 'mkosi.tools*' -exec rm -rf {} +
cp -a "$stage/." "$WORK/src/"
rm -rf "$stage"

export HOME="$WORK/home" XDG_CACHE_HOME="$WORK/cache" TMPDIR="$WORK/tmp" PYTHONDONTWRITEBYTECODE=1
cd "$WORK/src"
profile_args=()
[ -n "$PROFILE" ] && profile_args=(--profile "$PROFILE")
# WSL's /etc/resolv.conf is a symlink into /mnt/wsl, which does not exist in
# mkosi's sandbox, so name resolution fails there. Run mkosi in a private mount
# namespace where /etc is an overlay (upper layer on the F: work filesystem)
# and resolv.conf is a regular file. The WSL system itself is not changed.
rm -rf "$WORK/etc-upper" "$WORK/etc-work"
mkdir -p "$WORK/etc-upper" "$WORK/etc-work"
RESOLV=$(readlink -f /etc/resolv.conf)
[ -s "$RESOLV" ] || { echo "No usable resolver configuration at $RESOLV"; exit 1; }
export MKOSI WORK RESOLV
unshare --mount --propagation private bash -euo pipefail -c '
    mount -t overlay overlay -o "lowerdir=/etc,upperdir=$WORK/etc-upper,workdir=$WORK/etc-work" /etc
    rm -f /etc/resolv.conf
    cat "$RESOLV" > /etc/resolv.conf
    exec "$MKOSI/bin/mkosi" "$@"
' mkosi "${profile_args[@]}" \
    --output-directory "$WORK/out" \
    --workspace-directory "$WORK/workspace" \
    --cache-directory "$WORK/cache/incremental" \
    --package-cache-dir "$WORK/cache/packages" \
    --force "$@" build

version=$(sed -n 's/^ImageVersion=//p' mkosi.conf)
raw="$WORK/out/amityos_${version}.raw"
[ -f "$raw" ] || { echo "mkosi did not produce $raw"; ls -la "$WORK/out"; exit 1; }
name="amityos-${version}${PROFILE:+-$PROFILE}"
qemu-img convert -f raw -O qcow2 -c "$raw" "$OUT/$name.qcow2.partial"
mv -f "$OUT/$name.qcow2.partial" "$OUT/$name.qcow2"
cp -f "$WORK/out/amityos_${version}.manifest" "$OUT/$name.manifest.json" 2>/dev/null || true
(cd "$OUT" && sha256sum "$name.qcow2" > "$name.qcow2.sha256")
echo "IMAGE BUILT: image/out/$name.qcow2 ($(du -h "$OUT/$name.qcow2" | cut -f1), raw $(du -h --apparent-size "$raw" | cut -f1))"
