#!/bin/bash
# Run amity-comp nested (as a window of the host's display server, here WSLg)
# with the shell and one client, and check its state. Runs inside the build root:
#   tooling/dev.sh run bash tooling/nested_test.sh [debug|release]
# A window appears on the host desktop for about 15 seconds.
set -u
profile=${1:-release}
bin=/work/target/$profile
host_display=${XDG_RUNTIME_DIR:-/mnt/wslg/runtime-dir}/${WAYLAND_DISPLAY:-wayland-0}
runtime=/tmp/amity-nested
log=$runtime/amity-comp.log
status=0

[ -x "$bin/amity-comp" ] && [ -x "$bin/amity-shell" ] || { echo "FAIL: build amity-comp and amity-shell ($profile) first"; exit 1; }
[ -S "$host_display" ] || { echo "NOT RUN: no host Wayland display at $host_display (WSLg is not available)"; exit 3; }

rm -rf "$runtime" && mkdir -p "$runtime" && chmod 700 "$runtime"
# The nested compositor reaches the host display by absolute path and keeps
# its own sockets in a private runtime directory.
env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$host_display" \
    AMITY_COMP_DEBUG_SOCKET="$runtime/debug.sock" AMITY_WALLPAPER=/mnt/f/AmityOS/assets/wallpapers/amity-default.png \
    "$bin/amity-comp" --winit >"$log" 2>&1 &
compositor=$!

state() { curl -s --max-time 3 --unix-socket "$runtime/debug.sock" http://localhost/state; }
wait_for() {  # wait_for DESCRIPTION PATTERN
    for _ in $(seq 1 40); do
        if state | grep -qE "$2"; then echo "PASS: $1"; return 0; fi
        kill -0 "$compositor" 2>/dev/null || break
        sleep 0.5
    done
    echo "FAIL: $1"; status=1; return 1
}

wait_for "nested compositor is running with one output" '"outputs":\[\{"name":"winit"'
wait_for "the shell connected" '"shell_running":true'
# The work area must be smaller than the output: the taskbar reserved its strip.
reserved() {
    local heights
    heights=$(state | sed -n 's/.*"outputs":\[{"name":"winit","geometry":\[0,0,[0-9]*,\([0-9]*\)\],"work_area":\[0,0,[0-9]*,\([0-9]*\)\].*/\1 \2/p')
    set -- $heights
    [ $# -eq 2 ] && [ "$2" -lt "$1" ]
}
for _ in $(seq 1 40); do reserved && break; sleep 0.5; done
if reserved; then echo "PASS: the taskbar reserved space"; else echo "FAIL: the taskbar reserved space"; status=1; fi
state | grep -oE '"outputs":\[[^]]*\]' | head -1
socket=$(state | grep -oE '"wayland_display":"[^"]+"' | cut -d'"' -f4)
if [ -n "$socket" ]; then
    env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$socket" foot --app-id=nested.test >"$runtime/foot.log" 2>&1 &
    client=$!
    wait_for "a client window is mapped and focused" '"app_id":"nested.test".*"keyboard_focus":"window:1"'
    wait_for "XWayland started" '"x11_display":[0-9]+'
    echo "STATE: $(state)"
    kill "$client" 2>/dev/null
else
    echo "FAIL: the compositor did not report its Wayland socket"; status=1
fi
grep -E "panicked|ERROR" "$log" | head -5
echo "--- XWayland"; grep -iE "xwayland|x11" "$log" | head -8 | cut -c1-260; ls -ld /tmp/.X11-unix /tmp/.X11-unix/* 2>&1 | head -5
kill "$compositor" 2>/dev/null; wait "$compositor" 2>/dev/null
echo "--- compositor log (first lines)"; grep -vE "Supported (EGL|GL)" "$log" | head -12 | cut -c1-200
[ "$status" = 0 ] && echo "NESTED TEST PASSED" || echo "NESTED TEST FAILED"
exit "$status"
