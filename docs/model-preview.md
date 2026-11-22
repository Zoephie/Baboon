Open a model, object, render model, collision model, physics model, structure BSP or scenario and choose **Model Preview** to see it drawn. The page has the viewport, the **Model Setup** sidebar beside it, and the **Animation Player** along the bottom.

## Keeping up with saved tags

The preview draws its shaders, bitmaps, render model and the rest from the files on disk. When one of them is saved (in another tab, say a shader you are tuning) the preview loads again on its own, keeping its camera, picks and clip, with the old picture up until the new one is ready. The same happens when one changes outside Baboon, for example a bitmap reimported by Tool. Unsaved edits to those other tags do not show until they are saved. A preview in a background tab catches up when it is shown. See [File Change Monitoring](file-change-monitoring.md).

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

The **Shading** menu picks how the surfaces are drawn:

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
- **Background Color** clears the viewport to the **Theme**'s own background, **White**, **Black** or **Green Screen**. The choice is kept for every tag and the next time Baboon opens.
- **Camera** has the zoom, projection and reset.

## The footer

The footer counts the model's own vertices and triangles, leaving out the collision, physics and imposter geometry drawn with it. While **LOD Model** is ticked it counts the LOD model's instead. When part of the preview was left out, or a material drew without its textures, it says so there; hover the note for why.

## The Model Setup sidebar

The sidebar's cards sit together in one bordered panel, and scroll together. Each card shows only when the object has something for it, and its **Reset** button (the arrow around a dot, at the right of its header) puts back what the card changed.

A change that loads the model again (a level of detail, the shading, a weapon) changes only the viewport. The sidebar and the Animation Player stay as they were, not taking clicks, until the new model and everything that follows it (its materials, collision and physics, weapons and clip) have arrived, and then change at once.

- **Permutations** starts with the variant picker, its arrows stepping through the model's variants, and, for a `.model`, **Save** (save the picks as a new variant, or update the selected one) and the bin, which deletes the selected variant. In a narrow sidebar, Save shows its icon alone. A row per region follows: tick it to draw the region, and pick its permutation from the dropdown, or step through them with the arrows either side.
- **Weapons** picks what a unit holds. The held weapon's own regions follow its dropdown, as rows like the Permutations card's.
- **Effects** (Game Shaders) has a row per effect an event plays, with a button to play it. In Halo CE it starts with Scale A and Scale B, what the effects are played at, and a Fire row per trigger of a weapon.
- **Object Functions** (Game Shaders) has a slider per input the object's functions read, and per function it makes, its value in the box at the right. A function's slider shows what it makes until you tick its box, which sets it as a script would. A greyed-out row is one nothing in the preview reads. In Halo CE the function rows are its four outputs, A out to D out, named for their use; they show what the function makes. A held object's functions follow under its name.
