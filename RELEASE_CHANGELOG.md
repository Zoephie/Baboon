# Baboon Release Notes

## Added

- Renamed the application from **Genesis** to **Baboon**, including app title, package metadata, icon references, docs, and update URLs.
- Added a dedicated `.model` **Render model** tab for inspecting referenced `render_model` geometry without scrolling through the tag field tree.
- Added an embedded 3D model preview for `.model` tags with:
  - Region and permutation filtering
  - Variant selection
  - Marker display
  - Wireframe toggle
  - Backface toggle
  - Scale, rotate, pan, and zoom controls
- Added middle-mouse panning in the 3D model viewport.
- Added hover-only marker labels, with black marker points and high-contrast yellow labels.
- Added saved model viewport sizing:
  - Adjustable from `80%` to `260%`
  - Available in both `File > Settings > Appearance` and the render model tab
  - Persisted in user preferences
  - Layout adapts by moving variant controls below the viewport when space is limited
- Added `.model` variant editing from the render model tab:
  - Create new variant from current region/permutation selection
  - Update selected variant from current selection
  - Drop/delete selected variant
- Added documentation for dark mode and model viewport resizing in `Help > Doc...`.

## Changed

- Moved **Dark mode** out of the `View` menu and into `File > Settings > Appearance`.
- Improved model preview shading using decoded vertex normals for a Blender-like solid viewport look without requiring textures.
- Kept the app UI on the faster Glow/OpenGL backend after testing WGPU impact on responsiveness.
- Reduced unnecessary model preview state creation for non-model tags.

## Fixed

- Fixed exploded/broken render model geometry in the GUI preview.
- Fixed render model strip/index handling that caused invalid triangle output.
- Fixed visual line artifacts in the model viewport by switching filled mesh drawing to a single egui mesh path.
- Fixed variant/region loading regressions in the model preview.
- Preserved legacy Genesis preference/index fallback paths so existing users keep their settings.
