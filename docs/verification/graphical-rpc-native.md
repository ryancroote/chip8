# Graphical RPC native verification

Spec: #33. Tickets: #34 and #35. PR: #36.

Verified on macOS on 2026-11-05 UTC, using integration commit `0141a01`.
This supplements, rather than replaces, the automated native-window RPC tests
and the reproducible checklist in [TESTING.md](../../TESTING.md#graphical-rpc-regression-probe).

## Method

A temporary macOS application bundle pointed its executable at the project's
built `chip8` binary, without modifying that binary or its rendering code. This
allowed normal application activation for focused-window checks. Each test used
one graphical instance and connected to that instance's printed loopback endpoint;
no headless instance supplied the images.

The deterministic test ROM toggled display pixel (0,0) once per keypad press,
then waited for release. Its bytes were:

```text
6000 6100 A210 F20A D011 E29E 1206 120A 80
```

Native window IDs/bounds and foreground process IDs came from Core Graphics and
AppKit. Window captures used `screencapture -x -o -l WINDOW_ID`. The comparison
removed the title bar, accounted for Retina scaling, and checked the interior of
every one of the 2,048 scaled display pixels against the decoded RPC PNG.
Two physical pixels at each scaled-pixel edge were excluded to avoid native
chrome/compositor edge artifacts. This was not a comparison of two RPC images.

Physical keyboard and close-button input used permitted Core Graphics event
posting. Keyboard posting was refused unless the emulator was the foreground
application. No Accessibility permission bypass or screen-unlock attempt was made.

## Results

| Check | Result |
| --- | --- |
| Three successive display changes, focused | All 2,048 pixel interiors matched on each frame; captures 0.214–0.226 s after reply |
| Three successive display changes, unfocused | All 2,048 pixel interiors matched on each frame; captures 0.225–0.228 s after reply |
| Paused display, focused and unfocused | Unchanged after 1.1 s without actions; native pixels still matched |
| Physical W while an RPC no-key action ran | No display change; native pixels matched the initial blank frame |
| Native close button, idle connected controller | Process gone 0.307 s after click; launcher exited successfully |
| Native close button, during 60-frame action | Process gone 0.773 s after click; launcher exited successfully |
| Native close button, disconnected controller | Process gone 0.222 s after click; launcher exited successfully |

All close cases were bounded by five-second deadlines and checked the emulator
process itself, not just whether the controller disconnected. No panic was
reported. Machine-readable measurements are checked in beside this note.

## Remaining check

A final Rush Hour-specific graphical session entered its first board and accepted
a slide, but the desktop locked before its native screenshot/reconnect/close
sequence finished. Window capture failed with `could not create image from
window`, and the foreground process was `loginwindow`. Do not count that sequence
as a passed end-to-end Rush Hour check.

Unlock the desktop and repeat checklist step 6, including native capture,
controller handoff, and native window close. The generic deterministic native
checks above passed before the desktop locked. PR #36 remains draft pending the
last Rush Hour check; no further production-code change has been identified.
