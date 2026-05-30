// Swipe/flick direct manipulation: drag a visible layer to turn it.
//
// This is a pure presentation/input layer. It observes the picking events for
// the sticker meshes, resolves a screen drag into one of the 18 absolute moves,
// and feeds it to the frozen engine through `MoveQueue` — exactly like the move
// buttons. It never reaches into `CubeCore`, the animation system, or the JSON
// contract beyond `queue.0.push_back(<Move>)`.
//
// Geometry note (matches `animation::total_angle`: Cw = -FRAC_PI_2 about the
// outward normal): a grabbed layer is rotated +90° right-handed about
// `n × drag_axis`, where `n` is the grabbed face's outward normal and
// `drag_axis` is the dominant in-plane drag direction. Relative to the *turned*
// face's outward normal that reads as Ccw on the +axis side and Cw on the
// -axis side — see `swipe_to_move`.

use bevy::prelude::*;
use bevy::picking::events::{Cancel, DragEnd, Pointer, Press, Release};

use crate::cube::model::{Face, Move, Turn};
use crate::cube::spawn::{Cubie, Sticker};
use crate::cube::{Cube, MoveQueue};

/// Swipe/flick a visible layer to turn it. Feeds the same `MoveQueue` as the
/// buttons; never touches the frozen engine beyond `queue.0.push_back(<Move>)`.
pub struct SwipePlugin;

/// Tracks an in-flight press that began on the cube.
#[derive(Resource, Default)]
pub struct DragState {
    /// Captured grab, set on a cube `Press`, consumed (`take`) on `DragEnd`.
    grab: Option<CubeGrab>,
    /// True between a cube `Press` and its end; gates the orbit camera so a drag
    /// that started on the cube turns a layer instead of orbiting. Cleared on
    /// BOTH `Release` and `DragEnd` (Release fires before DragEnd), so it never
    /// depends on `grab` and never sticks.
    pub pressing_cube: bool,
}

struct CubeGrab {
    face: Face,        // world face the sticker faces
    cubie_pos: IVec3,  // grabbed cubie's current grid cell
    hit_world: Vec3,   // world hit point (for screen projection)
}

/// Minimum screen drag (pixels) to count as a swipe rather than a click.
const MIN_DRAG_PX: f32 = 8.0;

impl Plugin for SwipePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DragState>()
            .add_observer(on_press)
            .add_observer(on_release)
            .add_observer(on_cancel)
            .add_observer(on_drag_end);
    }
}

/// Press on the cube: capture the grabbed face + cubie + hit point, and mark
/// that a cube press is in flight (suppresses orbit). Presses that miss a
/// sticker (body gaps / empty space) leave `pressing_cube` false so the camera
/// orbits as before.
fn on_press(
    press: On<Pointer<Press>>,
    stickers: Query<(&Sticker, &ChildOf)>,
    cubies: Query<(&GlobalTransform, &Cubie)>,
    cube: Res<Cube>,
    mut drag: ResMut<DragState>,
) {
    let target = press.original_event_target();
    let Ok((sticker, child_of)) = stickers.get(target) else {
        return;
    };
    let Ok((cubie_xf, cubie)) = cubies.get(child_of.parent()) else {
        return;
    };
    // The grabbed face = the sticker's outward normal in world space, snapped to a
    // face. Uses the live rotation so it tracks the cubie even mid-animation.
    let world_n = cubie_xf.rotation() * sticker.local_normal.as_vec3();
    let face = nearest_face(world_n);
    // The grabbed cubie's CURRENT grid cell, read from the source of truth — not
    // `round(translation)`, which is mid-rotation while a prior swipe is still
    // animating and would mis-identify the layer. Exact for idle, scrambled, and
    // in-flight states alike.
    let Some(core) = cube.0.cubies().iter().find(|c| c.home == cubie.home) else {
        return;
    };
    let cubie_pos = core.pos;
    let p = press.event();
    let hit_world = p
        .event
        .hit
        .position
        .unwrap_or(cubie_xf.translation() + world_n);
    drag.grab = Some(CubeGrab {
        face,
        cubie_pos,
        hit_world,
    });
    drag.pressing_cube = true;
}

/// Any release ends the press (re-enables orbit). Fires before DragEnd.
fn on_release(_release: On<Pointer<Release>>, mut drag: ResMut<DragState>) {
    drag.pressing_cube = false;
}

/// A cancelled pointer (window focus loss / touch cancel mid-drag) emits no
/// Release or DragEnd, so clear the state here too — otherwise `pressing_cube`
/// would stay set and orbit would be suppressed for the rest of the session.
fn on_cancel(_cancel: On<Pointer<Cancel>>, mut drag: ResMut<DragState>) {
    drag.pressing_cube = false;
    drag.grab = None;
}

/// Drag that began on the cube: project the screen drag onto the face's two
/// in-plane world axes, pick the dominant one, resolve to a quarter turn, enqueue.
fn on_drag_end(
    drag_end: On<Pointer<DragEnd>>,
    mut drag: ResMut<DragState>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut queue: ResMut<MoveQueue>,
) {
    drag.pressing_cube = false;
    let Some(grab) = drag.grab.take() else {
        return;
    };
    let screen = drag_end.event().event.distance;
    if screen.length_squared() < MIN_DRAG_PX * MIN_DRAG_PX {
        return;
    }
    let Ok((cam, cam_xf)) = camera.single() else {
        return;
    };
    let Some(axis) = dominant_inplane_axis(cam, cam_xf, &grab, screen) else {
        return;
    };
    if let Some(mv) = swipe_to_move(grab.face, grab.cubie_pos, axis) {
        queue.0.push_back(mv);
    }
}

/// The two positive unit world axes lying in `face`'s plane (perpendicular to its normal).
fn face_plane_axes(face: Face) -> [IVec3; 2] {
    match face {
        Face::U | Face::D => [IVec3::X, IVec3::Z],
        Face::L | Face::R => [IVec3::Y, IVec3::Z],
        Face::F | Face::B => [IVec3::X, IVec3::Y],
    }
}

/// Project the two in-plane axes to screen at the hit point; return the one whose
/// screen direction best matches the drag, signed by the drag direction.
fn dominant_inplane_axis(
    cam: &Camera,
    cam_xf: &GlobalTransform,
    grab: &CubeGrab,
    screen: Vec2,
) -> Option<IVec3> {
    let base = cam.world_to_viewport(cam_xf, grab.hit_world).ok()?;
    let mut best: Option<IVec3> = None;
    let mut best_dot = 0.0_f32;
    for axis in face_plane_axes(grab.face) {
        let tip = cam.world_to_viewport(cam_xf, grab.hit_world + axis.as_vec3()).ok()?;
        let d = screen.dot(tip - base);
        if d.abs() > best_dot.abs() {
            best_dot = d;
            best = Some(if d >= 0.0 { axis } else { -axis });
        }
    }
    best
}

/// Absolute face whose outward normal best aligns with `v`.
fn nearest_face(v: Vec3) -> Face {
    let mut best = Face::U;
    let mut best_dot = f32::NEG_INFINITY;
    for f in Face::ALL {
        let d = v.dot(f.normal().as_vec3());
        if d > best_dot {
            best_dot = d;
            best = f;
        }
    }
    best
}

/// The face whose outward normal is exactly this axis-unit vector.
fn face_from_normal(n: IVec3) -> Face {
    Face::ALL
        .into_iter()
        .find(|f| f.normal() == n)
        .expect("axis-unit normal")
}

/// PURE: the quarter turn a swipe produces. `drag_axis` is the dominant in-plane
/// drag as a signed world axis. Returns None when the grabbed layer is the middle
/// slice (no outer-face move exists). The turned layer rotates +90° right-handed
/// about `n × drag_axis`; relative to the turned face's OUTWARD normal that is
/// Ccw when the cubie is on the +axis side, Cw on the -axis side — matching
/// animation::total_angle (Cw = -FRAC_PI_2 about the outward normal).
fn swipe_to_move(face_hit: Face, cubie_pos: IVec3, drag_axis: IVec3) -> Option<Move> {
    let n = face_hit.normal();
    let axis_dir = n.cross(drag_axis);
    let sign = cubie_pos.dot(axis_dir);
    if sign == 0 {
        return None;
    }
    let turned = face_from_normal(axis_dir * sign);
    let turn = if sign > 0 { Turn::Ccw } else { Turn::Cw };
    Some(Move { face: turned, turn })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(face: Face, turn: Turn) -> Move {
        Move { face, turn }
    }

    // GOLDEN cases — physics-verified against README/core conventions.
    #[test]
    fn golden_cases() {
        // drag front-top row right -> U'
        assert_eq!(
            swipe_to_move(Face::F, IVec3::new(0, 1, 1), IVec3::X),
            Some(mv(Face::U, Turn::Ccw))
        );
        // drag left -> U
        assert_eq!(
            swipe_to_move(Face::F, IVec3::new(0, 1, 1), -IVec3::X),
            Some(mv(Face::U, Turn::Cw))
        );
        // drag front-right col up -> R
        assert_eq!(
            swipe_to_move(Face::F, IVec3::new(1, 0, 1), IVec3::Y),
            Some(mv(Face::R, Turn::Cw))
        );
        // down -> R'
        assert_eq!(
            swipe_to_move(Face::F, IVec3::new(1, 0, 1), -IVec3::Y),
            Some(mv(Face::R, Turn::Ccw))
        );
        // middle slice (M), no outer move
        assert_eq!(swipe_to_move(Face::F, IVec3::new(0, 0, 1), IVec3::Y), None);

        assert_eq!(
            swipe_to_move(Face::U, IVec3::new(1, 1, 0), IVec3::Z),
            Some(mv(Face::R, Turn::Ccw))
        );
        assert_eq!(
            swipe_to_move(Face::U, IVec3::new(1, 1, 0), -IVec3::Z),
            Some(mv(Face::R, Turn::Cw))
        );
        assert_eq!(
            swipe_to_move(Face::R, IVec3::new(1, 1, 0), IVec3::Z),
            Some(mv(Face::U, Turn::Cw))
        );
        assert_eq!(
            swipe_to_move(Face::R, IVec3::new(1, 1, 0), -IVec3::Z),
            Some(mv(Face::U, Turn::Ccw))
        );
    }

    /// For an outer-layer cubie, every face × in-plane axis × sign produces a
    /// valid outer move on a face perpendicular to the grabbed one, and
    /// reversing the drag flips the turn while keeping the same turned face.
    #[test]
    fn property_all_faces_axes_signs() {
        for face in Face::ALL {
            let n = face.normal();
            for axis in face_plane_axes(face) {
                // The turn pivots about `n × axis`; place the cubie on the +side
                // of that axis so `sign != 0` (an outer-layer cubie).
                let pivot = n.cross(axis);
                // Build an outer-layer cubie: on the grabbed face (+n side) and
                // on the +pivot side so the dragged-about coordinate is nonzero.
                let cubie_pos = n + pivot;

                let pos = swipe_to_move(face, cubie_pos, axis).expect("outer move exists");
                let neg = swipe_to_move(face, cubie_pos, -axis).expect("outer move exists");

                // Turned face is perpendicular to the grabbed face: not the same
                // face and not its opposite.
                assert_ne!(pos.face, face, "turned == grabbed for {face:?}/{axis:?}");
                assert_ne!(
                    pos.face.normal(),
                    -n,
                    "turned is opposite of grabbed for {face:?}/{axis:?}"
                );
                // Turned face's normal is perpendicular to the grabbed normal.
                assert_eq!(
                    pos.face.normal().dot(n),
                    0,
                    "turned not perpendicular for {face:?}/{axis:?}"
                );

                // Reversing the drag keeps the turned face but flips the turn.
                assert_eq!(
                    pos.face, neg.face,
                    "reversed drag changed face for {face:?}/{axis:?}"
                );
                assert_ne!(
                    pos.turn, neg.turn,
                    "reversed drag did not flip turn for {face:?}/{axis:?}"
                );
                let flipped = match pos.turn {
                    Turn::Cw => Turn::Ccw,
                    Turn::Ccw => Turn::Cw,
                    Turn::Double => Turn::Double,
                };
                assert_eq!(neg.turn, flipped, "turn not exactly flipped");
            }
        }
    }

    /// A cubie on the middle slice for the dragged-about axis returns None.
    #[test]
    fn middle_slice_per_face() {
        for face in Face::ALL {
            let n = face.normal();
            for axis in face_plane_axes(face) {
                let pivot = n.cross(axis);
                // On the grabbed face, but the dragged-about coordinate is 0
                // (middle slice along `pivot`), so no outer move exists.
                let cubie_pos = n; // pivot-component is zero by construction
                debug_assert_eq!(cubie_pos.dot(pivot), 0);
                assert_eq!(
                    swipe_to_move(face, cubie_pos, axis),
                    None,
                    "expected middle-slice None for {face:?}/{axis:?}"
                );
            }
        }
    }
}
