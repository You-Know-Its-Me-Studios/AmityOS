#!/bin/bash
# Build the AmityOS disk image with the pinned mkosi. Runs as root inside WSL
# (AmityOS.ps1 image). Every byte stays on F:\AmityOS:
#   .cache/image-build/work.ext4   Linux filesystem for the build (loop-mounted)
#   image/out/                     results: amityos-<version>.qcow2, manifest, hashes
# The image contains the Ubuntu packages listed in image/mkosi.conf plus the
# AmityOS programs built from this repository (compositor and shell), which
# are compiled here first, in the build root (tooling/dev.sh).
set -euo pipefail
. "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/work_env.sh"

# Everything runs from main, so that bash has read the whole file before it
# executes anything: editing this script while it runs cannot derail it.
main() {
    OUT="$ROOT/image/out"
    PROFILE="${AMITY_IMAGE_PROFILE-devel}"

    require_root_on_f
    mount_work
    mkdir -p "$OUT"

    tools="$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"
    # No image with a dependency whose licence the rules forbid (docs/licensing.md).
    bash "$tools/dev.sh" metadata
    python3 -B "$tools/licenses.py" "$ROOT/.cache/tmp/cargo-metadata.json" --check
    bash "$tools/dev.sh" cargo build --release --locked -p amity-comp -p amity-shell
    bash "$tools/dev.sh" stage

    sync_config "$ROOT/image" "$WORK/src"

    cd "$WORK/src"
    profile_args=()
    [ -n "$PROFILE" ] && profile_args=(--profile "$PROFILE")
    run_mkosi "${profile_args[@]}" \
        --output-directory "$WORK/out" \
        --workspace-directory "$WORK/workspace" \
        --cache-directory "$WORK/cache/incremental" \
        --package-cache-dir "$WORK/cache/packages" \
        --extra-tree "$WORK/image-stage" \
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
}
main "$@"; exit
