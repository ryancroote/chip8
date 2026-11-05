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
| Rush Hour: first board, slide, native frame comparison | All 2,048 pixel centers matched; 2,046 full pixel interiors matched excluding the two native rounded bottom corner cells; capture 0.258 s after reply |
| Rush Hour: controller handoff | Reconnected controller received an identical frame, preserving the board and move counter |
| Rush Hour: native close after disconnect | Process gone 0.232 s after click; launcher exited successfully |

All close cases were bounded by five-second deadlines and checked the emulator
process itself, not just whether the controller disconnected. No panic was
reported. Machine-readable measurements are checked in beside this note.

## Rush Hour completion

The first attempt was interrupted by desktop lock and was not counted as a pass.
After the user unlocked the Mac, a fresh graphical Rush Hour test instance
entered its first board, accepted a slide, matched the native window to its RPC
frame, preserved that exact frame across controller handoff, and exited through
the native close button. No headless fallback was used. The user's existing
Rush Hour instance on port 8791 was left untouched.

The initial full-interior Rush Hour comparison detected macOS's rounded window
mask in the bottom-left corner cell, where the game draws white. The final
comparison checked all 2,048 pixel centers, plus the full interiors of the 2,046
non-corner cells. Only the native rounded bottom corner cells were excluded from
full-interior checks, not from pixel-center checks. This accounts for native
window chrome rather than accepting a game-pixel mismatch.

All pending desktop verification is complete. No additional production-code
change was necessary. Machine-readable Rush Hour measurements are in
`graphical-rpc-rushhour-results.json` beside this note.
