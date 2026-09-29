#!/usr/bin/env python3
"""Hold one key on a virtual keyboard: press-key.py <key> [seconds].

For bench-game.sh where there is no X server to send keys through (a Wayland
session): a uinput device, which the compositor delivers to the focused
window. The caller checks that the game has focus first. Needs write access to
/dev/uinput (the input group on most distributions). Keys: a-z, 0-9, enter,
esc, space, up, down, left, right, f1-f12.
"""
import fcntl
import os
import struct
import sys
import time

EV_SYN, EV_KEY = 0, 1
UI_SET_EVBIT, UI_SET_KEYBIT = 0x40045564, 0x40045565
UI_DEV_CREATE, UI_DEV_DESTROY = 0x5501, 0x5502

LETTERS = "qwertyuiop" "asdfghjkl" "zxcvbnm"
LETTER_CODES = list(range(16, 26)) + list(range(30, 39)) + list(range(44, 51))
KEYS = dict(zip(LETTERS, LETTER_CODES))
KEYS.update({str(n): 1 + n if n else 11 for n in range(10)})
KEYS.update({"enter": 28, "esc": 1, "space": 57, "up": 103, "down": 108,
             "left": 105, "right": 106})
KEYS.update({f"f{n}": 58 + n for n in range(1, 11)})
KEYS.update({"f11": 87, "f12": 88})


def main() -> int:
    if len(sys.argv) < 2 or sys.argv[1].lower() not in KEYS:
        print(__doc__.strip().splitlines()[0], file=sys.stderr)
        return 2
    code = KEYS[sys.argv[1].lower()]
    hold = float(sys.argv[2]) if len(sys.argv) > 2 else 0.25
    fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
    try:
        fcntl.ioctl(fd, UI_SET_EVBIT, EV_KEY)
        fcntl.ioctl(fd, UI_SET_KEYBIT, code)
        name = b"bigame-bench-keyboard".ljust(80, b"\0")
        os.write(fd, name + struct.pack("<HHHHi", 3, 1, 1, 1, 0) + b"\0" * (4 * 64 * 4))
        fcntl.ioctl(fd, UI_DEV_CREATE)
        # The compositor has to see the new device before it takes events.
        time.sleep(1.0)

        def event(kind: int, key: int, value: int) -> None:
            os.write(fd, struct.pack("<qqHHi", 0, 0, kind, key, value))

        event(EV_KEY, code, 1)
        event(EV_SYN, 0, 0)
        # Held rather than tapped: a game samples key state once per frame.
        time.sleep(hold)
        event(EV_KEY, code, 0)
        event(EV_SYN, 0, 0)
        time.sleep(0.2)
        fcntl.ioctl(fd, UI_DEV_DESTROY)
    finally:
        os.close(fd)
    return 0


if __name__ == "__main__":
    sys.exit(main())
