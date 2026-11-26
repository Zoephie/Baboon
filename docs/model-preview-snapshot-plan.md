# Model Preview snapshot: the current frame to the clipboard, background-free

Status: **built** on branch `material-editor`, 2026-11-26. Where the build departed from this plan:

- **Bloom comes from one pass at the viewport's size.** A snapshot first draws the whole frame at the viewport's size on black, which builds each game's bloom exactly as the viewport does; every render after (both backgrounds, every tile) adds that bloom (`RenderCore::held_bloom`, through `bloom_window`), instead of building its own. This answers three things at once: Halo 2's bright pass reads the whole scene, so a white background bloomed itself (the matte's premise test found it); Halo 3's and Halo 4's blur taps are in texels, so a larger image's bloom was narrower, and Halo 4's MCC resolution scale dimmed it; and Reach's bloom surfaces are a fixed size at high resolutions, which no tile could build its share of.
- **Tiles** are a window of the picture: `ModelGpuCamera::clip_offset`, applied in the GLSL `clip_position`, the gen3 view-projection and the CPU `clip_position`. Each is drawn with a margin of 32 pixels times the scale, within the image. Tiles are at most 4096 a side (about a gigabyte of targets), not the GPU's own limit.
- **A field-of-view screen effect's zoom** (`view_scale`) can't be applied per tile; tiled composites leave it out and the assembled image is resampled about its centre (`snapshot::zoom_about_centre`).
- **The large-image limit** asks at 1 GiB of image (a second press of the camera), not 4 GiB of total memory.
- **The clipboard** under test writes to a test sink, so the suite never overwrites the machine's clipboard.

A camera button in the Model Preview's top bar, right of the **View** dropdown,
copies the frame the viewport is showing to the clipboard as an image with a
transparent background and no ground grid. Every pixel that shows background
is fully transparent. What the model gives off beyond its outline still shows:
bloom, additive effects, glows, transparent and partly transparent surfaces,
each with the alpha it has over the background. And the frame is the one on
screen when the button was pressed: shader animations, function-driven values,
particles and the animation pose as they were then.

## What the code has now

- **Paint.** `renderer::paint_model_gl` runs in egui's paint callback with the
  frame's `ModelGpuFrame`. It clears the viewport rectangle to
  `frame.background`, then `ModelGlRenderer::paint` does the rest: the slot,
  the game's `prepare`, `begin_scene`, the grid and shared state
  (`begin_drawing`, which draws the grid when `frame.show_grid`), the game's
  `shade`, wireframe, overlays, the game's `finish`, the Material Editor's
  highlight, then `end_frame`.
- **Scene target.** With Game Shaders on, `begin_scene` points everything at
  `RenderCore::scene` (`SceneTarget`, **RGBA8**, the viewport's size), cleared
  to `frame.background`, and `finish_scene` blits it to the viewport.
- **Each game's background.** The background is only ever blended in; nothing
  lights or filters it:
  - Halo 3, ODST and Reach (`gen3`) draw into an accumulation buffer cleared to
    `(0,0,0,0)` whose alpha "is what covers the preview's background"
    (`gen3/pipeline/blend.rs`; additive modes leave alpha alone,
    `gen3_adds_light`). `h3/composite.rs` tone-maps accumulation plus bloom
    and writes `alpha = accumulated.a`, blended `ONE, ONE_MINUS_SRC_ALPHA`
    (`gl::screen::premultiplied_over`) onto the scene.
  - Halo 4's composite is the same, with `alpha = drawn ? 1 : accumulated.a`
    (`h4/shaders/composite.frag`).
  - Halo 2's bloom clears the scene's **alpha** channel and draws its bloom
    layer into it as the mask (`h2/bloom.rs`), not from the background.
  - Halo CE, and every game with Game Shaders off, draw straight onto the
    background with the material's blend.
- **The frame is a value.** `ModelGpuFrame` is `Clone` and carries everything a
  draw reads: `time`, `function_values`, `bones`, `change_colors`,
  `background`, `show_grid`, `highlight`, the render mode, and the Game
  Shaders switches (`bloom`, `active_camo`, `self_shadow`, …).
- **Simulations step to a time.** Particle systems keep the time they last ran
  to and step from there to `frame.time` (`ce/particle_systems/sim.rs`:
  `frame_steps(now, time)`). Drawing the same frame again steps nothing.
- **Clipboard.** egui's `Context::copy_image` goes to the fork's egui-winit,
  which writes through `arboard` with `image-data` (`clipboard.rs`
  `set_image`), converting egui's premultiplied `Color32` to straight alpha.
  Baboon already depends on `arboard` directly (`string_row_clipboard.rs`).
- **Reading pixels back.** The headless GL harness (`harness/shot.rs`) already
  renders frames into a framebuffer and reads them back into PNGs.

## The approach: a two-background matte, from the frame on screen

### Why two renders

Every pass blends the background in linearly, so each output pixel is
**`out = C + T·bg`**:
- `C` is what the model contributes.
- `T` is how much of the background shows through, per channel.

This holds for alpha blending (`T = 1 − a`), additive light (`T = 1`), multiply
blends (`T = src`), the gen3/H4 composites (`T = 1 − a`), and distortion over a
uniform background (the background is the same wherever it is sampled from).

Rendering the frame twice, once on `bg = 0` and once on `bg = 1`, gives:

- `C = out₀`
- `T = out₁ − out₀`

This is exact, for every game and every shading mode, with no shader changes.
It needs no new alpha bookkeeping in sixty-odd blend paths. An alpha channel
written per pass would need exactly that, and would break with each new
effect.

**No background pixels, by construction:**
- **Untouched pixel:** `C = 0, T = 1`, so it is exported fully transparent.
- **Partial cover:** a pixel the model covers partly keeps that cover.
- **Light beyond the outline:** bloom or a glow there has `C > 0`, so it shows
  rather than being cut to the outline.

### From `(C, T)` to an image a paste can use

A pasted PNG is composited by straight-alpha "over", which can't represent
pure addition (light with no cover). So each pixel becomes:

- `cover = 1 − mean(T)`: how much of the background it hides.
- `alpha = clamp(max(cover, max(C.r, C.g, C.b)), 0, 1)`: light that adds
  without covering still counts, as much as it is bright. This is the standard
  conversion for exporting glows.
- `color = C / alpha` (straight), when `alpha > 0`; `(0,0,0,0)` otherwise.

Over black this reproduces the viewport exactly. Over anything else, a glow
lies over the backdrop rather than adding to it, which is the closest a
straight-alpha image can come.

A pixel whose alpha rounds to 0 in 8 bits is written as `(0,0,0,0)`, so no
stray colour survives in "transparent" pixels.

### Why a float target

On an RGBA8 target, a bright additive pixel on `bg = 1` clamps at 1, and `T`
comes out wrong exactly where bloom is. The snapshot therefore renders into
targets of its own in **RGBA16F**: its own scene target, and its own copy of
the viewport rectangle for the non-Game-Shaders path.

The gen3 composite's own clamp to `[0, 1]` happens before the background is
blended, so it doesn't affect `T`.

### The frame on screen, not the next one

**What gets drawn.** Pressing the button doesn't start a new frame. The
snapshot draws **the last `ModelGpuFrame` the viewport painted**, with its
`time`, function values, pose and change colours, so animated shaders and
function-driven values are exactly as shown. It changes only:
- `show_grid = false`;
- `highlight` cleared, so the Material Editor's hover doesn't show;
- `background` set to 0, then 1.

**When it draws.** The state keeps a clone of the last frame it built, and the
press stores that as the snapshot's frame. The next paint callback draws the
snapshot (both backgrounds) **before** the viewport's own new frame.

**Why the simulations don't move.** They have run to the snapshot frame's time
and no further, so re-drawing it steps nothing. Then the viewport draws its new
frame, and the simulations continue from where they were.

A test confirms this (below), because a per-game simulation that steps by
draws instead of by time would quietly break it.

### Size: the viewport's, times a scale

The image is the viewport's size in physical pixels (the rectangle as drawn, at
the screen's pixels-per-point) times the snapshot **scale**, 1× by default.

- **The menu.** A small dropdown beside the camera button opens a menu with:
  - a slider from 0.1× to 10×;
  - a number box that takes any scale above 0, past 10× too;
  - the resolution that scale gives, from the viewport's size as it is now:
    "2160 × 1800" for a 1080 × 900 viewport at 2×.

  The scale is kept in the preferences. The camera button always copies at it.
- **The same picture, more pixels.** The camera's projection is the
  viewport's, so the framing is identical.
  - Effects sized in world units scale with the picture: geometry, particles,
    sprites, flares in the game's units.
  - Pixel-sized things don't. The wireframe and marker lines stay one pixel
    wide, and get relatively thinner at large scales.
  - Step 1's audit lists each pass's screen-space constants (bloom radii, blur
    kernels, half-resolution chains, `view_scale`). Each must be relative to
    the target's size, or be scaled by the snapshot's scale. Otherwise bloom
    at 4× would be a quarter as wide as on screen.
- **Bigger than the GPU allows: tiles.** An image wider or taller than
  `GL_MAX_TEXTURE_SIZE` (or `GL_MAX_RENDERBUFFER_SIZE`, whichever is less) is
  drawn in tiles. Each tile has the projection's sub-rectangle of the frame (an
  off-centre frustum) and a margin on every side wide enough for the widest
  screen-space pass (the bloom's blur at that scale). Only each tile's inside
  is kept, so blurs don't stop at tile edges.
- **Memory.** Before it starts, the snapshot works out what the targets, both
  read-backs and the image will take. Above a limit (proposed 4 GiB), it says
  so and asks before going on, rather than running the machine out of memory.
  The result is written to the clipboard as it is. A 10 000 × 10 000 image is
  400 MB of RGBA, which some apps won't take from a paste. The docs say so.
- **The readback** of a large frame happens per tile, and the matte runs per
  tile on the worker. Neither ever holds both full float renders at once.

## Steps

1. **Snapshot targets.** `SceneTarget` takes a format, with RGBA16F for the
   snapshot. Make a `SnapshotTargets` (scene and plain) owned by `RenderCore`,
   made on first use at the viewport's size and dropped when the size changes.
   Check that the gen3/H4 targets the frame uses (accumulation, bloom) are
   sized and owned per size, not per target, so a snapshot pass at the same
   size reuses them without disturbing the viewport's.

2. **Draw a frame into a given target.**
   - Split `ModelGlRenderer::paint` into the frame draw and where it lands: a
     `paint_into(gl, frame, target)` that both the viewport and the snapshot
     use, so there is one draw path and no copy of it.
   - The viewport's landing stays `begin_scene`/`finish_scene`. The snapshot's
     is its own target, cleared to `(bg, bg, bg, 0)`.
   - With Game Shaders off, the snapshot draws into its plain target, as the
     viewport draws into its rectangle.

3. **Request and result.**
   - The state keeps `last_frame: Option<ModelGpuFrame>`, set when a frame is
     built for paint.
   - The camera button sets `snapshot: Some(SnapshotRequest { frame,
     result: Arc<Mutex<Option<…>>> })`.
   - The paint callback, given the request with the frame, draws it twice
     (backgrounds 0 and 1), reads both back as `f32` RGBA (`glReadPixels`,
     `GL_FLOAT`), and puts them in `result`.
   - The UI pass takes the result next frame and hands it to a worker.

4. **Matte (worker).** Compute `(C, T)` → straight RGBA8, as above. Flip rows,
   since GL reads bottom-up. A pure function, unit-tested on synthetic pixels:
   - background only → transparent;
   - opaque → `alpha = 1`;
   - half-covered;
   - additive glow over nothing → `alpha = max(C)`;
   - multiply blend;
   - an alpha that rounds to 0 → `(0,0,0,0)`.

5. **Clipboard.** Write the straight RGBA through `arboard` directly, on the UI
   thread, rather than `copy_image`. Going through `Color32` stores
   premultiplied 8-bit, which leaves a faint glow at alpha 10 eleven colour
   levels: banding in exactly the halos this is for.
   - Keep one `arboard::Clipboard` alive in the app. On X11/Wayland the copy
     lasts only as long as its owner.
   - Report "Copied the preview to the clipboard (W × H)" in the status bar, or
     why it failed.
   - Enable `image-data` on Baboon's own `arboard` dependency. It is already on
     through egui-winit's, but should be stated where it's used.

6. **The button and its menu.** `ButtonIcon::Camera` (new asset, ink colour),
   right of **View** in the preview header (`panel/layout.rs`
   `draw_preview_header`), with a dropdown arrow beside it for the scale menu
   (above).
   - Tooltip: "Copy the frame to the clipboard, without its background
     (W × H)", where W × H is the size the scale gives.
   - Disabled, with the reason, while the preview has no frame, or while a
     snapshot is pending.
   - The number box takes a scale on Enter or when it loses focus, and refuses
     0, negatives and non-numbers, keeping what it had.
   - The slider and the box show the same scale. A box value past 10× leaves
     the slider at its end.

7. **Docs.** `docs/model-preview.md` and `docs/help.json`.

## Testing

All on the headless GL harness, which renders real kit models:

- **The matte's premise, per game.** For each game's sample models, with Game
  Shaders on and off, render at `bg = 0, 0.5, 1`. Check that the middle equals
  the average of the other two, to float precision. Any pass that reads the
  background non-linearly fails here, by name, before it can make a wrong
  snapshot.
  - It must catch something. A deliberately non-linear test pass (squaring the
    scene after the background) fails it.
- **No background pixels.** A snapshot of each sample has `alpha = 0`, and
  colour 0, wherever both renders show nothing but background, judged from the
  renders themselves, not the matte's output.
- **Glow survives the outline.** On a model with bloom (Halo 3 or Halo 4 Game
  Shaders, `bloom` on), snapshot pixels outside the model's covered area have
  `alpha > 0`.
  - Against the same model with `bloom` off, those pixels are transparent.
- **The frame on screen.**
  - Snapshot a model with an animated shader (the scorpion's treads, Halo CE)
    and live particles at time `t`. Its black-background render equals the
    viewport's own frame at `t` drawn on black.
  - The viewport frame after it equals the one it would have drawn without
    the snapshot, which shows the simulations didn't step twice.
  - Run for each game's particle and effect systems.
- **Scale.**
  - At 2×, the snapshot is twice the viewport's size each way, framed the
    same: its 2× box-downsample matches the 1× snapshot within a tolerance.
  - At 2×, with bloom on, the bloom is as wide relative to the picture as at
    1×.
- **Tiles.** With the tile limit forced small in a test (say 256), a 1024²
  snapshot drawn in tiles matches one drawn whole, bloom included. Nothing is
  seamed at tile edges.
- **Grid and highlight absent.** With the grid shown and a material hovered,
  the snapshot has neither.
- **UI.** In the frame harness:
  - the button's place (right of **View**), its tooltip and its disabled
    states;
  - the scale menu: slider and box agree; a typed 25× is taken; 0 and "abc"
    are refused; the resolution preview follows the viewport's size;
  - the status line after a copy. The
  clipboard write is behind a trait, so the test reads what would have been
  written.

## Platforms

- **macOS:** arboard writes an `NSImage` with alpha. Pastes into Preview,
  Photoshop and Discord keep transparency. To be confirmed by hand.
- **Windows:** arboard writes a DIB with alpha, and some apps drop the alpha of
  a pasted DIB. Check which formats arboard 3.6 puts on the clipboard, by
  reading its source; nothing here can run it on Windows. If there is no PNG
  format, add one (`RegisterClipboardFormat("PNG")`) through `clipboard-win`,
  which arboard already uses.
- **Linux:** the kept-alive clipboard above.

## Decided (owner, 2026-11-26)

1. **Viewport helpers** stay in the snapshot as shown: markers, skeleton,
   collision/physics overlays, wireframe. Left out are the grid, the
   background and the Material Editor's hover highlight.
2. **Size** is the viewport's times a scale. The menu has a 0.1×–10× slider,
   an uncapped number box and a resolution preview (see *Size*).
3. **Distortion** (heat haze, refraction) vanishes over a uniform background
   and isn't carried. The docs say so; nothing more.
