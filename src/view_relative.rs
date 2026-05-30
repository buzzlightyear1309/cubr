//! View-relative move mapping: turn a face named relative to the current camera
//! view into one of the 18 absolute `Move`s. Pure (no Bevy systems) so it is
//! unit-tested directly. Consumed by the Beginner panel in `ui.rs` (next unit).

use bevy::math::Vec3;

use crate::cube::model::{Face, Move, Turn};

/// A cube face named relative to where the camera is currently looking.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RelFace {
    Front,
    Back,
    Up,
    Down,
    Left,
    Right,
}

/// Resolve a view-relative face + turn into an absolute `Move`.
///
/// `basis` is `OrbitCamera::basis()` = `(forward, right, up)` (forward is the
/// look direction camera->origin). The relative face picks a world direction;
/// we resolve it to the absolute `Face` whose outward normal aligns best with
/// that direction. The turn (CW/CCW/180 "looking at that face from outside") is
/// already viewpoint-independent, so it passes through unchanged.
pub fn relative_move(basis: (Vec3, Vec3, Vec3), rel: RelFace, turn: Turn) -> Move {
    Move {
        face: resolve_face(rel_dir(basis, rel)),
        turn,
    }
}

/// The six relative faces, primary-faces-first. This is the iteration order
/// [`describe`] uses, so the default view names moves the way a user reads it.
const REL_ORDER: [RelFace; 6] = [
    RelFace::Front,
    RelFace::Up,
    RelFace::Right,
    RelFace::Back,
    RelFace::Down,
    RelFace::Left,
];

/// Inverse of [`relative_move`]: describe an absolute `Move` in view-relative terms
/// for the current camera `basis`. Returns the `RelFace` a user would name for that
/// move's face from this viewpoint, plus the (viewpoint-independent) `Turn`, which
/// passes through unchanged. Used by the Beginner-mode steps panel.
///
/// We return the FIRST relative face (in [`REL_ORDER`]) whose resolved absolute face
/// equals `m.face`. When such a face exists this guarantees the round-trip
/// `relative_move(basis, rel, m.turn).face == m.face` by construction, and the
/// primary-first order names the visible faces the way a user reads the default view
/// (Front->F, Up->U, Right->R).
///
/// Note `relative_move` is *not* surjective at 45°-corner views: independent
/// nearest-face resolution can map two relative faces onto the same absolute face,
/// leaving its opposite with no exact relative representative (e.g. at the default
/// view both `Left` and `Front` resolve to `F`, so no relative face resolves to `L`).
/// For such a `m.face` there is no exact round-trip; we fall back to the relative
/// face whose world direction aligns best with `m.face`'s outward normal — the right
/// thing to *tell the user* ("turn the Left face") since the panel enqueues the
/// absolute move directly. `describe` is therefore total and never panics.
pub fn describe(basis: (Vec3, Vec3, Vec3), m: Move) -> (RelFace, Turn) {
    if let Some(rel) = REL_ORDER
        .into_iter()
        .find(|&rel| relative_move(basis, rel, m.turn).face == m.face)
    {
        return (rel, m.turn);
    }
    let target = m.face.normal().as_vec3();
    let rel = crate::geom::best_by_dot(target, REL_ORDER.map(|r| (r, rel_dir(basis, r))));
    (rel, m.turn)
}

/// Full-word name for a relative face (Beginner-mode UI wording).
pub(crate) fn rel_word(rel: RelFace) -> &'static str {
    match rel {
        RelFace::Front => "Front",
        RelFace::Back => "Back",
        RelFace::Up => "Up",
        RelFace::Down => "Down",
        RelFace::Left => "Left",
        RelFace::Right => "Right",
    }
}

/// Spelled-out turn for a relative label. Plain ASCII: the stock Bevy font ships a
/// minimal glyph set (rotation arrows ↻/↺ and the degree sign render as tofu), so
/// the turn is text and the half turn is "180" (no ° symbol).
pub(crate) fn turn_word(turn: Turn) -> &'static str {
    match turn {
        Turn::Cw => "CW",
        Turn::Ccw => "CCW",
        Turn::Double => "180",
    }
}

/// The view-relative label for a face + turn, e.g. "Front CW", "Up 180". Shared by
/// the Beginner move buttons (`ui.rs`) and the Beginner step list (`solve_ui.rs`).
pub(crate) fn rel_label(rel: RelFace, turn: Turn) -> String {
    format!("{} {}", rel_word(rel), turn_word(turn))
}

/// World direction a relative face points along, for the given `basis`.
fn rel_dir(basis: (Vec3, Vec3, Vec3), rel: RelFace) -> Vec3 {
    let (forward, right, up) = basis;
    match rel {
        RelFace::Front => -forward,
        RelFace::Back => forward,
        RelFace::Up => up,
        RelFace::Down => -up,
        RelFace::Right => right,
        RelFace::Left => -right,
    }
}

/// Face priority for resolving 45°-corner ties deterministically. Chosen so the
/// default view (yaw π/4, pitch π/6) resolves Front->F, Up->U, Right->R.
const FACE_PRIORITY: [Face; 6] = [Face::F, Face::U, Face::R, Face::B, Face::D, Face::L];

/// The absolute face whose outward normal best aligns with `dir`; ties broken by
/// FACE_PRIORITY order (strict `>` keeps the earlier-listed face).
fn resolve_face(dir: Vec3) -> Face {
    crate::geom::best_by_dot(dir, FACE_PRIORITY.map(|f| (f, f.normal().as_vec3())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::basis_from_yaw_pitch;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_3, FRAC_PI_4, FRAC_PI_6, PI};

    /// At the default view (yaw π/4, pitch π/6) the relative faces resolve to
    /// the README scheme: Front->F, Up->U, Right->R (and the opposites Back->B,
    /// Down->D).
    #[test]
    fn default_view_resolves_to_readme_scheme() {
        let basis = basis_from_yaw_pitch(FRAC_PI_4, FRAC_PI_6);
        assert_eq!(relative_move(basis, RelFace::Front, Turn::Cw).face, Face::F);
        assert_eq!(relative_move(basis, RelFace::Up, Turn::Cw).face, Face::U);
        assert_eq!(relative_move(basis, RelFace::Right, Turn::Cw).face, Face::R);
        assert_eq!(relative_move(basis, RelFace::Back, Turn::Cw).face, Face::B);
        assert_eq!(relative_move(basis, RelFace::Down, Turn::Cw).face, Face::D);
    }

    /// Looking from +X (yaw 0, pitch 0) — a clean non-degenerate view. The
    /// camera faces -X, so screen-front is the L face's outward direction... no:
    /// forward = -pos = (-1,0,0), so Front = -forward = +X = R, etc.
    #[test]
    fn looking_from_plus_x() {
        let basis = basis_from_yaw_pitch(0.0, 0.0);
        assert_eq!(relative_move(basis, RelFace::Front, Turn::Cw).face, Face::R);
        assert_eq!(relative_move(basis, RelFace::Right, Turn::Cw).face, Face::B);
        assert_eq!(relative_move(basis, RelFace::Up, Turn::Cw).face, Face::U);
        assert_eq!(relative_move(basis, RelFace::Left, Turn::Cw).face, Face::F);
        assert_eq!(relative_move(basis, RelFace::Back, Turn::Cw).face, Face::L);
        assert_eq!(relative_move(basis, RelFace::Down, Turn::Cw).face, Face::D);
    }

    /// A +90° yaw from the default remaps which absolute face is "Right": it is
    /// no longer R.
    #[test]
    fn yaw_quarter_turn_remaps_right() {
        let basis = basis_from_yaw_pitch(FRAC_PI_4 + FRAC_PI_2, FRAC_PI_6);
        assert_ne!(relative_move(basis, RelFace::Right, Turn::Cw).face, Face::R);
    }

    /// The turn passes through unchanged regardless of the relative face.
    #[test]
    fn turn_passes_through() {
        let basis = basis_from_yaw_pitch(FRAC_PI_4, FRAC_PI_6);
        assert_eq!(
            relative_move(basis, RelFace::Front, Turn::Cw).turn,
            Turn::Cw
        );
        assert_eq!(
            relative_move(basis, RelFace::Right, Turn::Ccw).turn,
            Turn::Ccw
        );
        assert_eq!(
            relative_move(basis, RelFace::Up, Turn::Double).turn,
            Turn::Double
        );
    }

    /// The invariant `describe` must satisfy for one move at one basis:
    ///   - the turn always passes through unchanged, and
    ///   - the named relative face is the BEST name for `m.face` — either it
    ///     round-trips exactly (`relative_move(rel).face == m.face`), or (when
    ///     `relative_move` is non-surjective at this corner view and no relative
    ///     face resolves to `m.face`) its world direction is the closest of the six
    ///     to `m.face`'s outward normal.
    ///
    /// The exact round-trip is the common case; the fallback only fires for the one
    /// or two opposite faces stranded by a 45°-corner tie.
    fn assert_describe_names_best(basis: (Vec3, Vec3, Vec3), m: Move) {
        let (rel, turn) = describe(basis, m);
        assert_eq!(turn, m.turn, "turn changed for {m:?}");

        let reachable = REL_ORDER
            .into_iter()
            .any(|r| relative_move(basis, r, m.turn).face == m.face);
        if reachable {
            assert_eq!(
                relative_move(basis, rel, turn).face,
                m.face,
                "exact round-trip expected for reachable {m:?}"
            );
        } else {
            let normal = m.face.normal().as_vec3();
            let best = crate::geom::best_by_dot(normal, REL_ORDER.map(|r| (r, rel_dir(basis, r))));
            assert_eq!(rel, best, "fallback name mismatch for unreachable {m:?}");
        }
    }

    /// At the default view, `describe` names every one of the 18 moves correctly
    /// (exact round-trip where possible, best-dot fallback for stranded faces) and
    /// always preserves the turn.
    #[test]
    fn describe_names_all_moves_default_view() {
        let basis = basis_from_yaw_pitch(FRAC_PI_4, FRAC_PI_6);
        for &m in &Move::ALL {
            assert_describe_names_best(basis, m);
        }
    }

    /// Same naming invariant over all 18 moves at several other representative
    /// views: a flat front-on view, quarter/half yaw turns, and a steep pitch.
    #[test]
    fn describe_names_all_moves_other_views() {
        let views = [
            basis_from_yaw_pitch(0.0, 0.0),
            basis_from_yaw_pitch(FRAC_PI_4 + FRAC_PI_2, FRAC_PI_6),
            basis_from_yaw_pitch(PI, FRAC_PI_6),
            basis_from_yaw_pitch(FRAC_PI_4, FRAC_PI_3),
        ];
        for basis in views {
            for &m in &Move::ALL {
                assert_describe_names_best(basis, m);
            }
        }
    }

    /// At the default view, the absolute moves on the visible primary faces are
    /// named with the matching relative face (consistent with the README scheme).
    #[test]
    fn describe_default_view_naming() {
        let basis = basis_from_yaw_pitch(FRAC_PI_4, FRAC_PI_6);
        assert_eq!(describe(basis, Move::parse("F").unwrap()).0, RelFace::Front);
        assert_eq!(describe(basis, Move::parse("U").unwrap()).0, RelFace::Up);
        assert_eq!(describe(basis, Move::parse("R").unwrap()).0, RelFace::Right);
    }

    /// The turn passes straight through `describe`, unchanged.
    #[test]
    fn describe_turn_passes_through() {
        let basis = basis_from_yaw_pitch(FRAC_PI_4, FRAC_PI_6);
        assert_eq!(
            describe(
                basis,
                Move {
                    face: Face::R,
                    turn: Turn::Ccw
                }
            )
            .1,
            Turn::Ccw
        );
        assert_eq!(
            describe(
                basis,
                Move {
                    face: Face::R,
                    turn: Turn::Double
                }
            )
            .1,
            Turn::Double
        );
        assert_eq!(
            describe(
                basis,
                Move {
                    face: Face::R,
                    turn: Turn::Cw
                }
            )
            .1,
            Turn::Cw
        );
    }
}
