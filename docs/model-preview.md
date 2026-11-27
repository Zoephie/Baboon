Open a model, object, render model, collision model, physics model, structure BSP or scenario and choose **Model Preview** to see it drawn. The page has the viewport, the **Model Setup** sidebar beside it, and the **Animation Player** along the bottom.

## Keeping up with saved tags

The preview draws its shaders, bitmaps, render model and the rest from the files on disk. The shaders open in its [Materials](#materials) card are the exception: their edits always show, as they are made. When one of them is saved (in another tab, say a shader you are tuning) the preview loads again on its own, keeping its camera, picks and clip, with the old picture up until the new one is ready. The same happens when one changes outside Baboon, for example a bitmap reimported by Tool. Unsaved edits to those other tags show only with **View > Show Unsaved Edits**; otherwise they appear once saved. A preview in a background tab catches up when it is shown. See [File Change Monitoring](file-change-monitoring.md).

## Moving around

- **Drag** to orbit around the model.
- **Middle-drag**, or **Shift-drag**, to pan.
- **Scroll** to zoom toward the pointer.
- **View > Camera** has the zoom, **Perspective Projection** (clear it for a flat orthographic view) and **Reset View**.

## The header


The buttons at the left of the header show or hide each part of the model: the **Render Model**, **Collision Model**, **Physics Model** and **Armature**. A button takes its tag icon's colour while its part is shown. A model, or an object such as a biped or vehicle, gets buttons for the parts its `.model` names; a part with nothing to draw is greyed out. Halo CE objects have no physics model, so they get no Physics Model button. Hover a joint of the armature to see its name.

Collision and physics draw in their own colours (pink and red) over the render model: faces at half strength, so the model shows through, with solid edges. The Shading menu changes only the render model; its Wireframe option leaves collision and physics as they are.

- **Click** a button to show or hide its part.
- **Alt-click** shows only that part, hiding the others.
- **Shift-click** hides only that part, showing the others.

A Reach or Halo 2 Anniversary model whose `.model` names an **imposter** (the low, untextured stand-in the game draws far away) gets an **Imposter** button. It draws the imposter where the model is, each triangle in the colour baked into its vertices, and it doesn't take the model's animation. It starts hidden. Tool names an imposter's regions and permutations by position (`region_00`, `permutation_00` and on), and the game matches them to the render model's the same way, so the imposter follows the sidebar's region and permutation picks. If the render model's regions or permutations have changed since the imposter was made, they no longer line up: the game then unloads the imposter, and the button is greyed out; it needs generating again. An `.imposter_model` opened on its own previews in its own Model Preview tab, with its own `region_00` names.

### Shading

The **Shading** menu picks how the surfaces are drawn. The last one picked is remembered: every preview opens on it, this session and the next.

- **Wireframe** — edges only.
- **Solid** — plain shaded surfaces.
- **Textured** — the materials' base textures.
- **Game Shaders** — Halo CE, Halo 2 and Halo 3 materials through the game's own shaders, lit as the game lights them, with the object's effects and functions.

Below the choices:

- **Wireframe** draws the render model's edges over Solid, Textured or Game Shaders.
- **Render Backfaces** draws the inside of surfaces too.
- **Default Lights**, **PRT**, **Light Follows Camera**, **Active Camo**, **Bloom** and **Self Shadow** change how Game Shaders lights the model. They are greyed out under the other choices; hover one to see what it does.

### Other

The **Other** menu (the **…** button beside the model buttons):

- **Show Markers** draws the model's markers. Type in **Filter Markers…** to show only the ones whose names match.
- **Show Errors** highlights the error and warning reports stored in the tag. Hover one to read its report.
- **Show Bounding Sphere** (objects only: bipeds, vehicles, weapons, scenery and the rest) outlines the object's **bounding radius** about its **bounding offset**, measured from the model's root node (a biped's pelvis, a vehicle's frame) as the game does. It moves with animations and follows edits to those fields at once.
- **Level of Detail** (Halo CE and Halo 2 models) picks which of the model's levels is drawn: **Super Low** to **Super High** in Halo CE, **L1 Super Low** to **L6 Hollywood** in Halo 2, listed highest first. The highest level with geometry of its own is drawn until another is picked. A permutation without the level picked draws its nearest higher level. A level that only repeats the geometry of the level below it (tags often fill a missing level that way) is greyed out. The choice only changes the preview, never the tag, and each tag starts at its highest.
- **LOD Model** (Halo 3 and ODST models) draws the `.model`'s **LOD render model** (the simpler model the game swaps in at a distance, as the Covenant cruiser's) in place of the render model, following the sidebar's picks by name and the model's animation. It is greyed out when the `.model` names none, when the tag it names can't be read, and while the Render Model is hidden; hover it to see why.

### View

- **Show Grid** draws a ground grid, spaced to the model's size.
- **Show Unsaved Edits** draws the unsaved edits of other open tags the model uses (a shader you are tuning, say) instead of their saved files, so the preview follows them as you edit: it loads again a moment after you stop changing them. Closing such a tag without saving, or undoing back to its saved version, puts the preview back on the file. The previewed tag's own edits always show. The choice is shared by every preview and remembered; it starts off.
- **Background Color** clears the viewport to the **Theme**'s own background, **White**, **Black** or **Green Screen**. The choice is kept for every tag and the next time Baboon opens.
- **Camera** has the zoom, projection and reset.

### Snapshot

The **camera**, right of **View**, is one button with two halves: the camera and its scale (`1×`) copy the frame the viewport is showing to the clipboard as an image with a transparent background and no grid. It is the frame on screen when you press it: animated shaders, function-driven values, particles and the animation pose are as they were then. Every pixel that only shows background is fully transparent. What the model gives off past its outline stays: bloom, glows and additive effects are kept as partly transparent light, and transparent surfaces keep the cover they have. Markers, the armature, the bounding sphere and collision or physics overlays are in it when they are shown; the Material Editor's highlight is not. The status bar says when it has been copied.

The arrow on its right opens a menu that sets the image's size as a **scale** of the viewport's: a slider from 0.1× to 10×, a box that takes any scale above 0, and the image's **width** and **height**, each a box of its own: type either and the other follows, at the viewport's shape. The scale is kept for every preview and the next time Baboon opens. A larger image is the same picture with more pixels: bloom is as wide and as bright as on screen, though lines a pixel wide (the wireframe, markers) are thinner relative to it. An image too large for the GPU to draw at once is drawn in tiles, which the menu says; one over a gigabyte asks first (press the camera again), since it takes several on its way to the clipboard, and some apps won't take that large a paste.

Heat haze, refraction and other effects that bend what is behind the model have nothing to bend over a transparent background, so they aren't in the image.

## Decorator sets

A Halo 3, ODST or Reach `.decorator_set` opens in a **Decorator Set** preview. A level bakes where its decorators stand and how they are lit, so a set on its own has neither: the preview stands its types on a patch of flat ground, filled the way the game's brush fills one. Each decorator gets a type (by its placement weight, where the game has one), a spot kept its type's minimum distance from the others of its type, a random spin if the type rotates at random, and a tilt, hover, scale, wind motion and mix of the type's three colours from its ranges. The ground is flat, so *align to normal* changes nothing, and *align random* and *rotate x axis down* aren't applied yet.

With **Game Shaders**, the set draws through the explicit shader its `render shader` names (Halo 3's `decorator_default`, `decorator_wavy` and the rest, Reach's `decorator_wind_billboard`, `decorator_mesh` and the rest), translated from the kit's own source, each type's mesh drawn once per decorator as the game draws it. The wind is the globals' **default wind settings**, run on the preview's clock as the game runs it, so a set that sways in the wind moves, and a wavy one ripples. The light a level would bake into each decorator comes from the preview's own light probe instead, its colour leaning toward a mid-grey ground by its ground tint; Reach's decorators also stand in full sun, the preview's own light, under a clear sky. A set whose `shaded bright` is high, as Reach's skybox rocks', can wash out under that light. Without Game Shaders the set draws its texture, cut out by its alpha, and a two-sided set draws each face's back too.

The sidebar has three cards in place of Permutations:

- **Decorator Set**: its render shader, whether it draws both sides, its texture, and the wind it sways in.
- **Types**: a row per type, ticked to show it, with how many the patch stands. A type whose mesh its render model no longer has draws nothing, as in the game, and says so.
- **Layout**: the patch's edge in world units, how many decorators it tries to stand (any number; past 100,000 it warns that it's slow), and its seed. A change lays the patch out again at once. Fewer fit than asked where types keep their distance, and the card says how many did.

The game fades decorators out with distance; the preview doesn't yet. See `docs/decorator-preview-plan.md` for what comes next.

## The footer

The footer counts the model's own vertices and triangles, leaving out the collision, physics and imposter geometry drawn with it. While **LOD Model** is ticked it counts the LOD model's instead. When part of the preview was left out, or a material drew without its textures, it says so there; hover the note for why.

## The Model Setup sidebar

The sidebar's cards sit together in one bordered panel, and scroll together. Each card shows only when the object has something for it, and its **Reset** button (the arrow around a dot, at the right of its header) puts back what the card changed.

A change that loads the model again (a level of detail, the shading, a weapon) changes only the viewport. The sidebar and the Animation Player stay as they were, not taking clicks, until the new model and everything that follows it (its materials, collision and physics, weapons and clip) have arrived, and then change at once.

- **Permutations** starts with the variant picker, its arrows stepping through the model's variants, and, for a `.model`, **Save** (save the picks as a new variant, or update the selected one) and the bin, which deletes the selected variant. In a narrow sidebar, Save shows its icon alone. A row per region follows: tick it to draw the region, and pick its permutation from the dropdown, or step through them with the arrows either side.
- **Weapons** picks what a unit holds. The held weapon's own regions follow its dropdown, as rows like the Permutations card's.
- **Effects** (Game Shaders) has a row per effect an event plays, with a button to play it. In Halo CE it starts with Scale A and Scale B, what the effects are played at, and a Fire row per trigger of a weapon.
- **Object Functions** (Game Shaders) has a slider per input the object's functions read, and per function it makes, its value in the box at the right. A function's slider shows what it makes until you tick its box, which sets it as a script would. A greyed-out row is one nothing in the preview reads. In Halo CE the function rows are its four outputs, A out to D out, named for their use; they show what the function makes. A held object's functions follow under its name.

## Materials

The **Materials** card in the sidebar, after **Permutations** and before **Weapons**, edits the shaders the model draws with, and shows each change at once. It is there for a game whose materials the preview draws (Halo CE, Halo 2, Halo 3, ODST, Reach and Halo 4), closed at first; close it to put it away, and it opens again as it was left. It is as tall as the shader's fields, which scroll with the sidebar. Its dropdown starts on **None**, which shows the dropdown alone and opens no shader. While a shader is picked the sidebar widens to fit its grid, as the page allows, and goes back to its own width when it is not.

- **The dropdown** lists **None**, then each shader once, however many parts draw it: those of the model, its LOD model and what it holds. Where else a shader draws follows its name, as `visor (assault_rifle)`; an overlay, a shader drawn over its object rather than on parts of its own (Halo CE's modifier shader, Halo 2's shield and hologram shaders), says `overlay`. Type to search, or step through with the arrows either side. **While the pointer is over a row, or over the dropdown itself, the viewport lights every part that shader covers** in amber with white edges, over whatever shading is picked.
- **Under it** is the shader reference of the model's render model, as its own tab shows that field; its **Open** button opens the shader in a tab of its own, with its edits. **Switching it** points every slot of the render model that names this shader at the new one, as switching each in the render model's tab would: an unsaved edit to the render model (a `gbxmodel` for Halo CE), which loads the preview again and then shows the new shader. The render model is held as the shaders are, and asked about with them. A shader only the LOD model or a held object names shows its path instead: switch it in that tag.
- **Below that** the picked shader is edited as in its own tab: the parameter grid (or, for a Halo CE shader, its fields), with the same colour picker, function editor and undo.
- **Edits show at once.** Each one redraws that shader's parts in the preview, without loading the model again. A colour picker or function editor opened from the window shows what it is picking as it is dragged, before **OK** writes it; **Cancel** puts the preview back. Edits show under **Textured** and **Game Shaders** shading.
- **Nothing is written until you save.** The shader stays open, unsaved, until you save it or discard its edits; it is the same tag its own tab shows, so edits made in either place are in both. A shader with unsaved edits is marked in the dropdown with a dot before its name, on the theme's modified colour. The icons right of the dropdown are **Save** and **Reset** for the shader picked, then **Save All** and **Reset All** for every marked one. Reset asks first, under the dropdown, and says so when an open tab of the shader loses its edits too. A shader that can't be saved here (a read-only build's) says why on its Save.
- Closing the model's tab, the workspace or Baboon asks about the shaders with unsaved edits, as it asks about open tabs.

