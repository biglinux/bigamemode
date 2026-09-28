# shellcheck shell=bash
# Arms for SuperTuxKart on a hybrid laptop, read by bench-lab.sh (ARMS_FILE).
# Each directory beside this file is a BiGame-mode configuration
# (bigame-mode/video.toml, games/supertuxkart.toml) the launcher reads.
ARMS_DIR=$(dirname "${BASH_SOURCE[0]}")

# The installed game started as is: no offload, so it renders on the GPU that
# drives the panel (the integrated one on a hybrid laptop).
arm_igpu() { :; }

# BiGame-mode's launch: render offload to the discrete GPU.
arm_dgpu() { use_launch_plan "$ARMS_DIR/plain"; }

# The same inside nested Gamescope (no upscaling: game and output at the
# panel's size).
arm_dgpu_gamescope() { use_launch_plan "$ARMS_DIR/gamescope"; }

# The same with MangoHud's overlay (Forced: the wrapper, which reaches OpenGL).
arm_dgpu_mangohud() { use_launch_plan "$ARMS_DIR/mangohud"; }
