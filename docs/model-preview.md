Open a model, object, render model, collision model, physics model, structure BSP or scenario and choose **Model Preview** to see it drawn. The page has the viewport, the **Model Setup** sidebar beside it, and the **Animation Player** along the bottom.

## Moving around

- **Drag** to orbit around the model.
- **Middle-drag**, or **Shift-drag**, to pan.
- **Scroll** to zoom toward the pointer.
- **View > Camera** has the zoom, **Perspective Projection** (clear it for a flat orthographic view) and **Reset View**.

## The header

The buttons at the left of the header show or hide each part of the model: the **Render Model**, **Collision Model**, **Physics Model** and **Armature**. A button takes its tag icon's colour while its part is shown. A model, or an object such as a biped or vehicle, gets buttons for the parts its `.model` names; a part with nothing to draw is greyed out. Halo CE objects have no physics model, so they get no Physics Model button. Hover a joint of the armature to see its name.

- **Click** a button to show or hide its part.
- **Alt-click** shows only that part, hiding the others.
- **Shift-click** hides only that part, showing the others.

A Halo 3 or ODST model whose `.model` names an **LOD render model** (the simpler model the game draws at a distance, as the Covenant cruiser's) gets an **LOD Model** button. It draws that model where the model is, following the sidebar's region and permutation picks by name and the model's animation. It starts hidden; Alt-click it to see it alone. If the tag it names can't be read, the button is greyed out; hover it to see why.

### Shading

The **Shading** menu picks how the surfaces are drawn:

- **Wireframe** — edges only.
- **Solid** — plain shaded surfaces.
- **Textured** — the materials' base textures.
- **Game Shaders** — Halo CE, Halo 2 and Halo 3 materials through the game's own shaders, lit as the game lights them, with the object's effects and functions.

Below the choices:

- **Wireframe** draws the edges over Solid, Textured or Game Shaders.
- **Render Backfaces** draws the inside of surfaces too.
- **Default Lights**, **PRT**, **Light Follows Camera**, **Active Camo**, **Bloom** and **Self Shadow** change how Game Shaders lights the model. They are greyed out under the other choices; hover one to see what it does.

### Other

The **Other** menu (the **…** button beside the model buttons):

- **Show Markers** draws the model's markers. Type in **Filter Markers…** to show only the ones whose names match.
- **Show Errors** highlights the error and warning reports stored in the tag. Hover one to read its report.
- **Show Bounding Sphere** (objects only: bipeds, vehicles, weapons, scenery and the rest) outlines the object's **bounding radius** about its **bounding offset**, measured from the model's root node (a biped's pelvis, a vehicle's frame) as the game does. It moves with animations and follows edits to those fields at once.
- **Level of Detail** (Halo CE and Halo 2 models) picks which of the model's levels is drawn: **Super Low** to **Super High** in Halo CE, **L1 Super Low** to **L6 Hollywood** in Halo 2. The highest is drawn until another is picked. A permutation without the level picked draws its nearest higher level, and a level no permutation has is greyed out. The choice only changes the preview, never the tag, and each tag starts at its highest.

### View

- **Show Grid** draws a ground grid, spaced to the model's size.
- **Background Color** clears the viewport to the **Theme**'s own background, **White**, **Black** or **Green Screen**. The choice is kept for every tag and the next time Baboon opens.
- **Camera** has the zoom, projection and reset.

## The footer

The footer counts the model's vertices and triangles, leaving out an LOD model drawn with it. When part of the preview was left out, or a material drew without its textures, it says so there; hover the note for why.
