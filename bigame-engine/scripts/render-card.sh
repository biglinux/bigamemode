# shellcheck shell=bash
# Sourced by the session scripts: the DRM card games render on.
#
# A discrete card before an integrated one, the most VRAM among equals: the
# order BiGame-mode's own hardware::pick_render_gpu uses. Each vendor needs its
# own evidence of "discrete", because only amdgpu publishes its memory:
#   - NVIDIA: every NVIDIA card on PCI is discrete, and the proprietary driver
#     publishes no VRAM or DPM attribute at all;
#   - amdgpu: VRAM with a memory vendor (an APU leaves the vendor blank);
#   - i915/xe: off the root bus (an Arc card); the one at 0000:00:02.0 is the
#     CPU's own.
# Requiring amdgpu's attributes, as these scripts once did, found no GPU at all
# on a laptop with Intel graphics and an NVIDIA card.
render_card() {
    local card driver slot vendor vram discrete best="" best_discrete=-1 best_vram=-1
    for card in /sys/class/drm/card[0-9]*; do
        [[ $card =~ /card[0-9]+$ ]] || continue
        driver=$(basename "$(readlink -f "$card/device/driver" 2>/dev/null)")
        slot=$(basename "$(readlink -f "$card/device" 2>/dev/null)")
        vram=$(cat "$card/device/mem_info_vram_total" 2>/dev/null || echo 0)
        vendor=$(cat "$card/device/mem_info_vram_vendor" 2>/dev/null || true)
        case $driver in
            nvidia | nouveau) discrete=1 ;;
            amdgpu) discrete=$([ -n "$vendor" ] && echo 1 || echo 0) ;;
            i915 | xe) discrete=$([[ $slot == 0000:00:* ]] && echo 0 || echo 1) ;;
            *) continue ;;
        esac
        if [ "$discrete" -gt "$best_discrete" ] \
            || { [ "$discrete" -eq "$best_discrete" ] && [ "$vram" -gt "$best_vram" ]; }; then
            best=$(basename "$card") best_discrete=$discrete best_vram=$vram
        fi
    done
    echo "$best"
}

# Whether the card has a DPM level to force (amdgpu's attribute). Where it has
# none, a DPM arm cannot be run and says so instead of measuring nothing.
has_dpm() {
    [ -e "/sys/class/drm/$1/device/power_dpm_force_performance_level" ]
}
