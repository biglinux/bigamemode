# shellcheck shell=bash
# Arms for Shadow of the Tomb Raider, read by bench-relaunch.sh (ARMS_FILE).
# Applied with the game closed. The game's own settings live in its Wine
# registry, written only once the prefix's wineserver has exited (it writes
# its in-memory copy back when it does).
SOTTR_DIR="$HOME/.local/share/Steam/steamapps/common/Shadow of the Tomb Raider"
SOTTR_REG="$HOME/.local/share/Steam/steamapps/compatdata/750920/pfx/user.reg"
ARMS_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
GRAPHICS_APPLY="$ARMS_DIR/../../target/release/examples/graphics_apply"

wineserver_gone() {
    local _i
    for _i in $(seq 1 60); do pgrep -x wineserver >/dev/null || return 0; sleep 1; done
    return 1
}

# One DWORD under the game's Graphics key.
sottr_setting() {
    wineserver_gone || { echo "the game's wineserver is still running" >&2; return 1; }
    python3 - "$SOTTR_REG" "$1" "$2" <<'EOF'
import re, sys
path, name, value = sys.argv[1], sys.argv[2], int(sys.argv[3])
text = open(path, encoding="utf-8", errors="surrogateescape").read()
head = "[Software\\\\Eidos Montreal\\\\Shadow of the Tomb Raider\\\\Graphics]"
start = text.index(head)
end = text.find("\n[", start + 1)
end = len(text) if end < 0 else end
section = text[start:end]
line = f'"{name}"=dword:{value:08x}'
new, n = re.subn(rf'^"{re.escape(name)}"=dword:[0-9a-f]{{8}}$', line, section, flags=re.M)
if n != 1:
    sys.exit(f"{name} not found once in the Graphics key")
open(path, "w", encoding="utf-8", errors="surrogateescape").write(text[:start] + new + text[end:])
EOF
}

# DX12, the game's XeSS Quality taken over by OptiScaler 0.9.4 (FSR 3.1): AI
# Graphics' Recommended plan, applied as its page does.
arm_optiscaler() {
    [ -f "$SOTTR_DIR/dxgi.dll" ] || "$GRAPHICS_APPLY" SOTTR.exe >/dev/null || return 1
    sottr_setting EnableDX12 1
}

# DX12, the game's own XeSS Quality: OptiScaler restored away, and XeSS put back
# on (Restore returns it to what it was before Apply, off) with DLSS as Apply
# leaves it, so the arms differ in OptiScaler alone.
arm_native_xess() {
    if [ -f "$SOTTR_DIR/dxgi.dll" ]; then
        "$GRAPHICS_APPLY" SOTTR.exe --remove >/dev/null || return 1
    fi
    sottr_setting XESS 3 && sottr_setting DLSS 0 && sottr_setting EnableDX12 1
}

# The graphics API alone: 1920×1080 native, no upscaler (XeSS and DLSS off,
# OptiScaler restored away), DX12 through VKD3D-Proton or DX11 through DXVK.
api_native() {
    if [ -f "$SOTTR_DIR/dxgi.dll" ]; then
        "$GRAPHICS_APPLY" SOTTR.exe --remove >/dev/null || return 1
    fi
    sottr_setting XESS 0 && sottr_setting DLSS 0 && sottr_setting EnableDX12 "$1"
}
arm_dx12() { api_native 1; }
arm_dx11() { api_native 0; }
