# Pose mode: plan

Posing a model's skeleton by hand in the Model Preview, for viewing: drag a joint and the skeleton
follows like a puppet, over the bind pose or over an animation.

## Decided (2026-11-27)

- Viewing only: no export, nothing saved.
- Posing works over the bind pose and over animations, playing or paused.
- No name-based guesses: limbs and limits come from tag data or not at all.
- Hard joint limits only where the tags give them; elsewhere, do what we can.

## What the tags give

- **The pose path.** Local node transforms → world (`world_pose`) → the game's IK → skinning rows.
  The skinned mesh and the armature now share one function (`posed_node_matrices`), and the hand
  pose is laid over it last, so the game's IK can't undo it.
- **Joints on screen.** The armature overlay projects every node and finds the one under the pointer.
- **No stretching by construction.** A bone is a node's offset from its parent; the hand pose only
  turns nodes, so lengths never change.
- **Joint limits: the physics model's constraints.** Havok ragdoll constraints (`min/max twist`,
  `min/max cone`, `min/max plane`) and limited hinges (`limit min/max angle`) between two nodes, each
  side with a frame (`forward`, `left`, `up`, `position`) in its node's space. Halo 2, Halo 3, Reach
  (and Halo 4's physics models) carry them; the stock Chief, Elite and Spartan each have 5 ragdolls
  (head, shoulders, hips) and 4 limited hinges (elbows, knees). blam-tags already reads them
  (`JmsRagdoll`, `JmsHinge`). Their nodes map to the render model's by name, as the game's model node
  maps do.
- **IK chains: Reach's and Halo 4's graphs** name their own (`right_arm`, `left_arm`, `right_leg`,
  `left_leg`, start node to effector node). Halo 2's and Halo 3's graphs name none.
- **The two-bone solver** the preview already runs for weapon IK (`two_bone_ik`, the game's).

## Steps

1. **Drag a joint (done).** A Pose toggle beside the armature toggle; a drag that starts on a joint
   turns the joint's parent so the joint follows the pointer across the plane through it facing the
   camera; Reset pose. Turns are kept in each node's parent frame, so they ride an animation.
2. **Drag a chain (done).** A drag moves the joint with the chain above it (Blender's Auto IK): two
   nodes to begin with, as the graphs' limb chains run effector to grandparent, the wheel adding or
   taking one while dragging. FABRIK places the chain, straightening it toward a target out of reach;
   each node then takes the least swing that lands its child, root first, adding no twist. Damped
   least squares was tried first: it crawled near full extension, and with lighter damping it folded
   the chain backward.
3. **Joint limits (done).** The physics model's ragdoll and limited hinge constraints, read by name
   and their nodes matched to the render model's by name, held after each placing (a few rounds of
   placing and holding each drag event); a Limits toggle beside Pose. As
   `c_havok_component::create_constraint` (Halo 3 0x1803206F0, Ares `havok_component.cpp`) builds
   them: a frame's columns are forward, left, up (`hkTransform_from_matrix4x3`); a ragdoll's twist
   axis is forward and its plane axis left, its twist range as the tag has it, its cone the larger of
   `min cone` and `max cone`, its plane range negated; a limited hinge turns about up from left, its
   range negated. The tag holds degrees. The ragdolls' `node a` is the pelvis (the torso is one rigid
   body in a ragdoll), so a bent spine counts against the shoulders and head. Joints with no
   constraint (spine, wrists, fingers) stay free.
4. **Pins and twist.** Pin joints the solve must hold (hands and feet planted while the body moves),
   a ring to twist a joint about its bone, and moving the root.

## Research

FABRIK (Aristidou & Lasenby) works in joint positions and needs a twist rule when turned back into
rotations; CCD favours the joints nearest the effector; damped least squares (Buss) handles several
effectors and unreachable targets stably and takes per-joint weights. Blender's Auto IK sets the
chain by length while dragging; Unreal's Full Body IK is position-based with per-bone stiffness.

## Not settled

- The sign of a ragdoll joint's plane angle in Havok's terms: the preview measures it as the twist
  axis's angle toward the left axis, its range negated as the game negates it. The hinge's sign is
  checked against the Chief's elbow, which bends forward to its limit; the plane's has no such check.
- Halo 2's, Reach's and Halo 4's constraint builds: only Halo 3's was read. Their tags share the
  field names and hold the same degree ranges.
