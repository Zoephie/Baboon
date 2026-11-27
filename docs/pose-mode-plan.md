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
   nodes to begin with, as the graphs' limb chains run effector to grandparent. Follow in the pose
   strip sets how many (− and +, up to the deepest chain the skeleton has); while dragging, `]` or
   the wheel up adds one and `[` or the wheel down takes one, the keys for a trackpad, where
   scrolling while holding a drag is awkward. A drag shows no label (it once named the joint and
   the count beside it). FABRIK places the chain, straightening it toward a target out of reach;
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
   range negated. The tag holds degrees. The preview measures a hinge's angle the right-handed way
   about up, which runs opposite to Havok's, so in the preview's terms the limit is the tag's own
   range: the Chief's elbow bends to 75° and his knee to 60° the way they bend, 15° the other (a
   first reading with Havok's sign took for a forward bend what was the elbow bent backward). The ragdolls' `node a` is the pelvis (the torso is one rigid
   body in a ragdoll), so a bent spine counts against the shoulders and head. Joints with no
   constraint (spine, wrists, fingers) stay free.
4. **Pins, twist and the root (done).** Double-click a joint to pin it where it stands (an orange
   ring); after every drag each pin's own chain is turned to bring it back, so the feet stay planted
   while the pelvis is dragged down into a crouch. Alt-drag twists a joint about its bone (the line to
   its first child). Dragging the root moves the whole skeleton. Reset pose clears pins too. Two
   additions made limited chains land: a limited hinge is first bent a little into its larger side
   (FABRIK bends a chain the way it is already bent, and the Chief's knees stand a hair back in the
   bind pose), and a finishing pass turns each node only as it can turn, a hinge about its own axis,
   since FABRIK bends in the plane of the joints, which a hinge's plane need not be.

   The plane angle's sign was settled the same way as the hinge's: the preview's angle toward left
   runs opposite to Havok's, so in the preview's terms the limit is the tag's own range. The Chief's
   hip and shoulder plane ranges are one-sided (−60° to 0° left, 0° to 60° right): read that way, each
   leg splays outward but can't cross the other and each arm lifts away from his body, as a ragdoll is
   built; read the other way, his legs could only cross and his arms couldn't lift.
5. **A move gizmo (done).** Clicking a joint selects it and puts a gizmo on it in the model's axes
   (x forward red, y left green, z up blue). Dragging an arrow moves the joint along that axis alone,
   to the axis point nearest the pointer's line of sight; dragging a square moves it within that plane
   (xy, xz or yz), where the line of sight meets it; the joint's own dot still drags across the plane
   facing the camera. A locked drag goes only as far toward the pointer along its line or plane as
   the chain and its limits can take the joint exactly (a search between the last point reached and
   the target), so a hand pulled up past its reach stops on the axis rather than swinging off it.

6. **Rotate mode (done).** Move and Rotate beside Pose switch the selected joint's gizmo. In
   Rotate it shows a ring per model axis (near halves over far); dragging round one turns the joint
   about that axis, carrying what hangs below, its limit held and the pins after. The turn follows
   the pointer's run along the ring's way round where it was taken, over the ring's radius, so a ring
   seen edge on still turns. Alt-drag still twists about the bone.

7. **The pose tools in the viewport (done).** Pose's buttons (Limits, Move, Rotate, Reset pose)
   ran the header into its right-hand menus in a narrow card. Only the Pose toggle stays in the
   header; the rest ride in a strip in the viewport's top-left corner while posing. The header
   now measures its two groups before drawing them and, too narrow for both, gives the right
   group's menus up to icons, then folds the model toggles into the Other menu.

8. **Scale mode (done).** Scale beside Move and Rotate puts a circle on the selected joint, flat
   to the screen; dragging it outward grows the joint, inward shrinks it, by the pointer's distance
   from the joint over its distance at the press (×0.05 to ×20). The scale is one factor for all
   three axes, as a Halo node transform and its animations carry, laid over the scale beneath:
   the joint stays where it stands, and what hangs below it scales with it. The solver reads bone
   lengths from the posed skeleton, so a drag below a scaled joint reaches as far as its longer
   bones now do. Reset pose undoes scales too.

9. **Selecting several joints (done).** Shift+click adds a joint to the selection or takes it
   out; the last one clicked is active and carries the gizmo, the active joint drawn larger.
   Shift+double-click selects a joint and everything below it; a plain click selects one alone,
   and a click on empty space lets them go. Rotate, Scale and Alt-twist take every selected joint
   by the same amount, each about its own origin (Blender's individual origins), so nested ones
   add up: a hand's finger joints curl together. A move takes the others along by as far as the
   active joint actually went, each through its own chain, leaving out any above or below the
   active joint, and of two on one limb all but the upper, whose chains would fight over the same
   bones. Dragging the root still moves the whole skeleton. No box select: a plain drag on empty
   space orbits.

10. **Global or local axes (done).** Global and Local in the pose strip (not shown for Scale,
    which has no axes) pick whether the gizmo runs along the model's axes or the active joint's
    own, its posed world transform's x, y and z. A move keeps to the axes it began with while the
    chain moves the joint under them; a ring turns each selected joint about that axis of its
    own, so fingers curl about their own hinges. A locked move also now keeps the joint as far
    behind the pointer as the press took the handle: it used to jump to the point under the
    pointer, so taking an arrow near its tip threw the joint most of an arrow's length.

## Research

FABRIK (Aristidou & Lasenby) works in joint positions and needs a twist rule when turned back into
rotations; CCD favours the joints nearest the effector; damped least squares (Buss) handles several
effectors and unreachable targets stably and takes per-joint weights. Blender's Auto IK sets the
chain by length while dragging; Unreal's Full Body IK is position-based with per-bone stiffness.

## Joint limits in each game (settled 2026-11-27)

Read in Reach's and Halo 4's symbolled Xbox 360 debug builds beside Halo 3's:

- **Reach and Halo 4 hand Havok every range as the tag has it** (`create_constraint`, Reach
  0x83178580, Halo 4 0x83322CC0), the twist and plane each held within a quarter turn either side
  of straight, the cone the larger of its ends' sizes, short of a half turn. Halo 3 negates the
  plane and hinge ranges. Their tags carry the same signs (elbows −75° to 15° in Reach, −70° to
  15° in Halo 3), so read Halo 3's way, Reach's and Halo 4's elbows and knees bent backward and
  their legs splayed inward. A limited hinge takes forward rather than left as its reference in
  Reach and Halo 4, which leaves the angle as it was.
- **The other games, by trying their characters**: Halo 2's elbows and knees, and ODST's elbows,
  knees and hips, bend the way they bend read Halo 3's way. Halo 2's plane ranges are read so too
  but unproven: its hips' frames don't show a leg's splay plainly in the plane angle.
- **Halo 2's ragdolls and all of Halo 4's constraints name the child first** (thigh, then pelvis;
  calf, then thigh). Havok's body A is always `node a`, and the limit is on B's frame against A's
  either way; the preview now turns whichever of the two hangs below the other.
- **The twist's sign**: Havok measures the twist as it measures the hinge
  (`hkInternalConstraintUtils_inlineCalcRelativeAngle` beside `buildJacobianFromAngLimitAtom`:
  both the other way about the axis from the right hand). The twist had been held in the
  right-handed measure, mirroring it; Halo 4's twists run 0° to 10° or 15°, one-sided.
- Kit tests try every game's stock character: elbows and knees bend 50° their way unhindered and
  are held back bent 40° the other; legs splay outward but not across, the pelvis unmoved.

## Not settled

- Halo 2 Anniversary's multiplayer (Halo 4's engine) and Campaign Evolved (Reach's tags) are read
  Reach's and Halo 4's way, unread and untried: no kit here has their characters.
- Halo 2's plane ranges, as above.
