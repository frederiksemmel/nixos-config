import time

from evdev import UInput, ecodes

mouse = UInput(
    {
        ecodes.EV_KEY: [ecodes.BTN_LEFT, ecodes.BTN_RIGHT],
        ecodes.EV_REL: [ecodes.REL_X, ecodes.REL_Y],
    },
    name="semmeltv-vm-test-mouse",
)
time.sleep(2)
for dx, dy in [(40, 30), (-25, 15), (60, -10)] * 8:
    mouse.write(ecodes.EV_REL, ecodes.REL_X, dx)
    mouse.write(ecodes.EV_REL, ecodes.REL_Y, dy)
    mouse.syn()
    time.sleep(0.05)
time.sleep(600)
