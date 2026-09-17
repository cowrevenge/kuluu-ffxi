# Chat windows in the supplied September 14 recording

Source: `/Users/jon/Downloads/Screen Recording 2026-09-14 at 4.51.47 PM.mov`,
57 seconds, provided by the user. Local extracted frames are under
`artifacts/verify/chat-retail/`. The DLL build and installed addons are not
identified by the recording; these observations describe this configuration,
not verified defaults across client builds.

## Observed

- Around 10-16 seconds, Config > Windows > Shared offers Vertical (upper/lower),
  Horizontal (left/right), and Off; timestamp and window type/effect controls
  are separate settings. Window 1 and Window 2 have separate settings pages.
- Around 20-40 seconds, the per-window pages expose maximum/minimum lines,
  column width, resize time and reactive sizing. The windows remain bottom
  aligned with independently changing heights.
- At 35 seconds, Window 1 contains synthesis, item loss and casting messages;
  Window 2 contains conversation with sender/location prefixes. This proves
  the observed grouping, not the client's default category routing.
- Synthesis/loss body text is pale yellow, casting text stronger yellow,
  item names bright yellow-green. Conversation in the second window is coral
  red. Auto-translate opening markers are green and closing markers red;
  phrase text retains the surrounding message color.
- The user reports using Compact keyboard F to change the focused window.
  Window labels distinguish the active send mode (for example Window 1:Say).

## Implementation scope and uncertainty

Kuluu exposes Tabbed, Vertical and Side by side layouts. Equal widths bounded
by a readable measure and a narrow-window stacking fallback are explicitly
requested product choices. Layout choice persists; no manual width option.
Action/system messages and conversation have distinct groups, and internal
Debug is visible only with the development HUD. This is a useful initial
routing policy matching the supplied setup; per-category user routing remains
separate backlog work.

The adjusted log/yell/item colors are visual approximations of this capture,
not extracted default RGB constants. Incoming message metadata currently
collapses distinct system/action color categories. Exact category colors,
retail bitmap font/frame assets, and location-aware yell prefixes require
additional data-path work. Other unobserved channels retain their existing
colors. Auto-translate uses the protocol emitter's brace delimiters as the
available glyph fallback, preserving text while coloring markers separately.
