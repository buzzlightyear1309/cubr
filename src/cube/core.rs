use crate::cube::model::{CubeState, Face, Move, StickerColor};
use bevy::math::IVec3;

/// 26 cubies (the hidden core at (0,0,0) is omitted). Integer rotation math only.
pub struct CubeCore {
    cubies: Vec<CoreCubie>,
}

/// Apply one clockwise (looking from outside) quarter-turn about the outward
/// normal `axis` to the integer vector `v`.
///
/// Clockwise-as-seen-from-outside is a right-handed rotation by -90° about the
/// outward normal `n`. For a 90° turn Rodrigues' formula reduces to
/// `R(v) = (n·v) n - (n × v)` (with cosθ = 0, sinθ = -1). Sanity: for n = +Y
/// this sends (x,y,z) -> (-z, y, x), the README `U` convention.
fn rot_cw(axis: IVec3, v: IVec3) -> IVec3 {
    let dot = axis.dot(v);
    axis * dot - axis.cross(v)
}

impl CubeCore {
    pub fn solved() -> Self {
        let mut cubies = Vec::with_capacity(26);
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    if x == 0 && y == 0 && z == 0 {
                        continue; // hidden core
                    }
                    let home = IVec3::new(x, y, z);
                    let stickers = solved_stickers(home);
                    cubies.push(CoreCubie {
                        home,
                        pos: home,
                        orient: identity(),
                        stickers,
                    });
                }
            }
        }
        CubeCore { cubies }
    }

    /// Apply a move as an integer permutation+reorientation of the affected layer.
    /// Quarter turns applied quarter_turns_cw times. Pure geometry — colors ride along.
    pub fn apply(&mut self, m: Move) {
        let axis = m.face.normal();
        let turns = m.quarter_turns_cw();
        for idx in self.layer(m) {
            let cubie = &mut self.cubies[idx];
            for _ in 0..turns {
                cubie.pos = rot_cw(axis, cubie.pos);
                cubie.orient = [
                    rot_cw(axis, cubie.orient[0]),
                    rot_cw(axis, cubie.orient[1]),
                    rot_cw(axis, cubie.orient[2]),
                ];
            }
        }
    }

    /// Repaint to an arbitrary state for POST /state: reset all cubies to home pose,
    /// then assign each visible sticker the given color. Represents impossible states fine.
    pub fn paint(&mut self, state: &CubeState) {
        for cubie in &mut self.cubies {
            cubie.pos = cubie.home;
            cubie.orient = identity();
            // In the home (identity) pose, each sticker's local outward normal
            // equals its world normal, which is a face normal. Read the facelet
            // color for that face at the home position's facelet index.
            for (local_normal, color) in &mut cubie.stickers {
                let face = face_for_normal(*local_normal);
                let idx = world_pos_to_index(face, cubie.home);
                *color = state.face(face)[idx];
            }
        }
    }

    /// Read the current facelets in README per-face orientation (row-major, the index
    /// layout and per-face viewing rules in README "Per-face viewing orientation").
    // §1 public contract: state read-back for downstream stages (CV/solver); the
    // Stage-1 binary never reads it back, so allow it to be unused here.
    #[allow(dead_code)]
    pub fn to_state(&self) -> CubeState {
        let mut state = CubeState::solved();
        for face in Face::ALL {
            let normal = face.normal();
            let arr = state_face_mut(&mut state, face);
            for (idx, slot) in arr.iter_mut().enumerate() {
                let world_pos = index_to_world_pos(face, idx);
                // Find the cubie currently at that world position and read the
                // sticker whose *world* normal equals this face's outward normal.
                let cubie = self
                    .cubies
                    .iter()
                    .find(|c| c.pos == world_pos)
                    .expect("a cubie must occupy every surface grid position");
                let color = cubie
                    .stickers
                    .iter()
                    .find(|(local, _)| world_normal(&cubie.orient, *local) == normal)
                    .map(|(_, c)| *c)
                    .expect("cubie on a face must carry a sticker facing that face");
                *slot = color;
            }
        }
        state
    }

    /// For the renderer: snapshot of each cubie's current pose + visible stickers, so the
    /// Bevy layer can build/sync entities. `home` identifies the entity across moves.
    pub fn cubies(&self) -> &[CoreCubie] {
        &self.cubies
    }

    /// Indices into cubies() that lie in the layer this move turns (the 9 moving pieces).
    pub fn layer(&self, m: Move) -> Vec<usize> {
        let axis = m.face.normal();
        // The face's outward sign along its single nonzero axis component; the
        // moving layer is the set of cubies whose pos projects to +1 on that axis.
        self.cubies
            .iter()
            .enumerate()
            .filter(|(_, c)| c.pos.dot(axis) == 1)
            .map(|(i, _)| i)
            .collect()
    }
}

/// Read-only view the renderer consumes.
pub struct CoreCubie {
    pub home: IVec3,        // solved position; stable id for the entity
    pub pos: IVec3,         // current grid position, components in {-1,0,1}
    pub orient: [IVec3; 3], // integer rotation matrix columns (local->world basis)
    // visible stickers: which outward local face shows which color
    pub stickers: Vec<(IVec3 /*local outward normal*/, StickerColor)>,
}

/// 3×3 integer identity as column vectors.
fn identity() -> [IVec3; 3] {
    [
        IVec3::new(1, 0, 0),
        IVec3::new(0, 1, 0),
        IVec3::new(0, 0, 1),
    ]
}

/// Transform a local vector by the orientation matrix (columns) into world space.
fn world_normal(orient: &[IVec3; 3], local: IVec3) -> IVec3 {
    orient[0] * local.x + orient[1] * local.y + orient[2] * local.z
}

/// The face whose outward normal equals this axis-unit vector.
fn face_for_normal(n: IVec3) -> Face {
    Face::ALL
        .into_iter()
        .find(|f| f.normal() == n)
        .expect("normal must be one of the six face normals")
}

/// Stickers for a solved cubie at `home`: one per axis it touches, colored by the
/// face whose normal is that outward axis.
fn solved_stickers(home: IVec3) -> Vec<(IVec3, StickerColor)> {
    let mut stickers = Vec::new();
    let axes = [
        (IVec3::new(1, 0, 0), home.x),
        (IVec3::new(0, 1, 0), home.y),
        (IVec3::new(0, 0, 1), home.z),
    ];
    for (unit, comp) in axes {
        if comp != 0 {
            let normal = unit * comp; // outward direction along this axis
            let face = face_for_normal(normal);
            stickers.push((normal, face.solved_color()));
        }
    }
    stickers
}

/// Mutable access to a face array inside a CubeState.
fn state_face_mut(state: &mut CubeState, f: Face) -> &mut [StickerColor; 9] {
    match f {
        Face::U => &mut state.U,
        Face::D => &mut state.D,
        Face::L => &mut state.L,
        Face::R => &mut state.R,
        Face::F => &mut state.F,
        Face::B => &mut state.B,
    }
}

/// The single shared per-face index <-> world-position mapping (derived from the
/// README "Per-face viewing orientation" table). `paint` and `to_state` both go
/// through this so they cannot disagree.
///
/// With row r = idx/3, col c = idx%3, components in {-1,0,1}:
///   U (y=+1): x = c-1, z = r-1
///   D (y=-1): x = c-1, z = 1-r
///   F (z=+1): x = c-1, y = 1-r
///   B (z=-1): x = 1-c, y = 1-r   (mirrored)
///   R (x=+1): z = 1-c, y = 1-r
///   L (x=-1): z = c-1, y = 1-r
fn index_to_world_pos(face: Face, idx: usize) -> IVec3 {
    let r = (idx / 3) as i32;
    let c = (idx % 3) as i32;
    match face {
        Face::U => IVec3::new(c - 1, 1, r - 1),
        Face::D => IVec3::new(c - 1, -1, 1 - r),
        Face::F => IVec3::new(c - 1, 1 - r, 1),
        Face::B => IVec3::new(1 - c, 1 - r, -1),
        Face::R => IVec3::new(1, 1 - r, 1 - c),
        Face::L => IVec3::new(-1, 1 - r, c - 1),
    }
}

/// Inverse of `index_to_world_pos`: which facelet index on `face` does this world
/// position carry.
fn world_pos_to_index(face: Face, pos: IVec3) -> usize {
    let (r, c) = match face {
        Face::U => (pos.z + 1, pos.x + 1),
        Face::D => (1 - pos.z, pos.x + 1),
        Face::F => (1 - pos.y, pos.x + 1),
        Face::B => (1 - pos.y, 1 - pos.x),
        Face::R => (1 - pos.y, 1 - pos.z),
        Face::L => (1 - pos.y, pos.z + 1),
    };
    (r * 3 + c) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cube::model::StickerColor::{B, G, O, R, W, Y};

    // Self-checks on the README mapping (asserted in the task brief).
    #[test]
    fn index_mapping_anchor_points() {
        assert_eq!(index_to_world_pos(Face::U, 0), IVec3::new(-1, 1, -1));
        assert_eq!(index_to_world_pos(Face::U, 8), IVec3::new(1, 1, 1));
        assert_eq!(index_to_world_pos(Face::F, 0), IVec3::new(-1, 1, 1));
        assert_eq!(index_to_world_pos(Face::B, 0), IVec3::new(1, 1, -1));
        assert_eq!(index_to_world_pos(Face::R, 0), IVec3::new(1, 1, 1));
        assert_eq!(index_to_world_pos(Face::L, 0), IVec3::new(-1, 1, -1));
    }

    #[test]
    fn index_mapping_is_a_bijection_inverse() {
        for face in Face::ALL {
            for idx in 0..9 {
                let pos = index_to_world_pos(face, idx);
                assert_eq!(
                    world_pos_to_index(face, pos),
                    idx,
                    "face {face:?} idx {idx}"
                );
            }
        }
    }

    // rot_cw sanity: the README U convention (x,y,z) -> (-z, y, x).
    #[test]
    fn rot_cw_u_convention() {
        let axis = Face::U.normal();
        assert_eq!(rot_cw(axis, IVec3::new(1, 0, 0)), IVec3::new(0, 0, 1));
        assert_eq!(rot_cw(axis, IVec3::new(0, 0, 1)), IVec3::new(-1, 0, 0));
        assert_eq!(rot_cw(axis, IVec3::new(2, 5, 3)), IVec3::new(-3, 5, 2));
    }

    // Test 1: solved core == solved state == README example.
    #[test]
    fn solved_core_matches_solved_state_and_readme() {
        let core = CubeCore::solved();
        let state = core.to_state();
        assert_eq!(state, CubeState::solved());

        assert_eq!(state.U, [W; 9]);
        assert_eq!(state.R, [R; 9]);
        assert_eq!(state.F, [G; 9]);
        assert_eq!(state.D, [Y; 9]);
        assert_eq!(state.L, [O; 9]);
        assert_eq!(state.B, [B; 9]);
    }

    #[test]
    fn solved_core_has_26_cubies() {
        assert_eq!(CubeCore::solved().cubies().len(), 26);
        // sticker counts: 6 centers (1), 12 edges (2), 8 corners (3) = 6+24+24 = 54
        let total: usize = CubeCore::solved()
            .cubies()
            .iter()
            .map(|c| c.stickers.len())
            .sum();
        assert_eq!(total, 54);
    }

    // Test 2: direction anchor — pins "clockwise".
    #[test]
    fn u_move_direction_anchor() {
        let mut core = CubeCore::solved();
        core.apply(Move::parse("U").unwrap());
        let state = core.to_state();
        assert_eq!(&state.F[0..3], &[R, R, R]);
        assert_eq!(&state.L[0..3], &[G, G, G]);
    }

    // Test 3: every quarter move has order 4; every double has order 2.
    #[test]
    fn move_orders() {
        let solved = CubeCore::solved().to_state();
        for &m in &Move::ALL {
            let order = m.quarter_turns_cw();
            let mut core = CubeCore::solved();
            if order == 2 {
                core.apply(m);
                core.apply(m);
                assert_eq!(
                    core.to_state(),
                    solved,
                    "double move {} order 2",
                    m.notation()
                );
            } else {
                for _ in 0..4 {
                    core.apply(m);
                }
                assert_eq!(
                    core.to_state(),
                    solved,
                    "quarter move {} order 4",
                    m.notation()
                );
            }
        }
    }

    // Test 4: X then X' returns to solved, for all six faces.
    #[test]
    fn move_then_inverse_is_identity() {
        let solved = CubeCore::solved().to_state();
        for face in Face::ALL {
            let mut core = CubeCore::solved();
            let cw = Move {
                face,
                turn: crate::cube::model::Turn::Cw,
            };
            let ccw = Move {
                face,
                turn: crate::cube::model::Turn::Ccw,
            };
            core.apply(cw);
            core.apply(ccw);
            assert_eq!(core.to_state(), solved, "face {face:?} X then X'");
        }
    }

    // Test 5: sexy move (R U R' U') ×6 returns to solved.
    #[test]
    fn sexy_move_order_six() {
        let solved = CubeCore::solved().to_state();
        let mut core = CubeCore::solved();
        let seq = [
            Move::parse("R").unwrap(),
            Move::parse("U").unwrap(),
            Move::parse("R'").unwrap(),
            Move::parse("U'").unwrap(),
        ];
        for _ in 0..6 {
            for &m in &seq {
                core.apply(m);
            }
        }
        assert_eq!(core.to_state(), solved);
    }

    // Test 6: paint round-trip for solved and for an impossible (all-white) state.
    #[test]
    fn paint_round_trip() {
        let mut core = CubeCore::solved();

        let solved = CubeState::solved();
        core.paint(&solved);
        assert_eq!(core.to_state(), solved);

        let all_white = CubeState {
            U: [W; 9],
            R: [W; 9],
            F: [W; 9],
            D: [W; 9],
            L: [W; 9],
            B: [W; 9],
        };
        core.paint(&all_white);
        assert_eq!(core.to_state(), all_white);
    }

    // Extra: paint correctly absorbs an arbitrary scramble-shaped (still valid) state,
    // proving the shared mapping is consistent both directions.
    #[test]
    fn paint_round_trip_after_scramble() {
        let mut scrambled = CubeCore::solved();
        for m in [
            "R", "U", "F", "L", "D", "B", "R'", "U2", "F'", "L2", "D'", "B2",
        ] {
            scrambled.apply(Move::parse(m).unwrap());
        }
        let target = scrambled.to_state();

        let mut core = CubeCore::solved();
        core.paint(&target);
        assert_eq!(core.to_state(), target);
    }
}
