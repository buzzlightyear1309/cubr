use glam::IVec3;
use serde::{Deserialize, Serialize};

/// Sticker color. Serializes to its single-letter name ("W","Y","R","O","B","G").
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum StickerColor {
    W,
    Y,
    R,
    O,
    B,
    G,
}

impl StickerColor {
    /// All six colors, used for sanity checks.
    pub const ALL: [StickerColor; 6] = [
        StickerColor::W,
        StickerColor::Y,
        StickerColor::R,
        StickerColor::O,
        StickerColor::B,
        StickerColor::G,
    ];
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Face {
    U,
    D,
    L,
    R,
    F,
    B,
}

impl Face {
    pub const ALL: [Face; 6] = [Face::U, Face::D, Face::L, Face::R, Face::F, Face::B];

    /// Outward normal in world space: U=+Y, D=-Y, R=+X, L=-X, F=+Z, B=-Z  (see README coords).
    pub fn normal(self) -> IVec3 {
        match self {
            Face::U => IVec3::new(0, 1, 0),
            Face::D => IVec3::new(0, -1, 0),
            Face::R => IVec3::new(1, 0, 0),
            Face::L => IVec3::new(-1, 0, 0),
            Face::F => IVec3::new(0, 0, 1),
            Face::B => IVec3::new(0, 0, -1),
        }
    }

    /// Solved color: U=W, D=Y, F=G, B=B, R=R, L=O  (see README face table).
    pub fn solved_color(self) -> StickerColor {
        match self {
            Face::U => StickerColor::W,
            Face::D => StickerColor::Y,
            Face::F => StickerColor::G,
            Face::B => StickerColor::B,
            Face::R => StickerColor::R,
            Face::L => StickerColor::O,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Turn {
    Cw,
    Ccw,
    Double,
} // (none), ', 2

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Move {
    pub face: Face,
    pub turn: Turn,
}

impl Move {
    /// All 18 standard moves, in README order: U U' U2 D D' D2 L L' L2 R R' R2 F F' F2 B B' B2.
    pub const ALL: [Move; 18] = [
        Move {
            face: Face::U,
            turn: Turn::Cw,
        },
        Move {
            face: Face::U,
            turn: Turn::Ccw,
        },
        Move {
            face: Face::U,
            turn: Turn::Double,
        },
        Move {
            face: Face::D,
            turn: Turn::Cw,
        },
        Move {
            face: Face::D,
            turn: Turn::Ccw,
        },
        Move {
            face: Face::D,
            turn: Turn::Double,
        },
        Move {
            face: Face::L,
            turn: Turn::Cw,
        },
        Move {
            face: Face::L,
            turn: Turn::Ccw,
        },
        Move {
            face: Face::L,
            turn: Turn::Double,
        },
        Move {
            face: Face::R,
            turn: Turn::Cw,
        },
        Move {
            face: Face::R,
            turn: Turn::Ccw,
        },
        Move {
            face: Face::R,
            turn: Turn::Double,
        },
        Move {
            face: Face::F,
            turn: Turn::Cw,
        },
        Move {
            face: Face::F,
            turn: Turn::Ccw,
        },
        Move {
            face: Face::F,
            turn: Turn::Double,
        },
        Move {
            face: Face::B,
            turn: Turn::Cw,
        },
        Move {
            face: Face::B,
            turn: Turn::Ccw,
        },
        Move {
            face: Face::B,
            turn: Turn::Double,
        },
    ];

    /// Parse one of the 18 notation strings; None for anything else.
    pub fn parse(s: &str) -> Option<Move> {
        let face = match s.chars().next()? {
            'U' => Face::U,
            'D' => Face::D,
            'L' => Face::L,
            'R' => Face::R,
            'F' => Face::F,
            'B' => Face::B,
            _ => return None,
        };
        let turn = match &s[1..] {
            "" => Turn::Cw,
            "'" => Turn::Ccw,
            "2" => Turn::Double,
            _ => return None,
        };
        Some(Move { face, turn })
    }

    /// Notation string, e.g. "R", "R'", "R2".
    pub fn notation(self) -> String {
        let face = match self.face {
            Face::U => 'U',
            Face::D => 'D',
            Face::L => 'L',
            Face::R => 'R',
            Face::F => 'F',
            Face::B => 'B',
        };
        let suffix = match self.turn {
            Turn::Cw => "",
            Turn::Ccw => "'",
            Turn::Double => "2",
        };
        format!("{face}{suffix}")
    }

    /// Rotation axis = self.face.normal().
    pub fn axis(self) -> IVec3 {
        self.face.normal()
    }

    /// Number of clockwise (looking at the face from outside) quarter-turns: Cw=1, Ccw=3, Double=2.
    pub fn quarter_turns_cw(self) -> u8 {
        match self.turn {
            Turn::Cw => 1,
            Turn::Ccw => 3,
            Turn::Double => 2,
        }
    }

    /// The move that undoes this one: same face, opposite quarter-turn direction
    /// (Cw↔Ccw); a double turn is its own inverse. So `m` followed by `m.inverse()`
    /// is the identity on the cube.
    pub fn inverse(self) -> Move {
        let turn = match self.turn {
            Turn::Cw => Turn::Ccw,
            Turn::Ccw => Turn::Cw,
            Turn::Double => Turn::Double,
        };
        Move {
            face: self.face,
            turn,
        }
    }
}

/// The exact JSON shape of POST /state (see README "Cube-state JSON contract").
/// Field order is irrelevant to serde; keys are U R F D L B.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct CubeState {
    pub U: [StickerColor; 9],
    pub R: [StickerColor; 9],
    pub F: [StickerColor; 9],
    pub D: [StickerColor; 9],
    pub L: [StickerColor; 9],
    pub B: [StickerColor; 9],
}

impl CubeState {
    /// All 9 of each face = that face's solved color.
    pub fn solved() -> Self {
        CubeState {
            U: [Face::U.solved_color(); 9],
            R: [Face::R.solved_color(); 9],
            F: [Face::F.solved_color(); 9],
            D: [Face::D.solved_color(); 9],
            L: [Face::L.solved_color(); 9],
            B: [Face::B.solved_color(); 9],
        }
    }

    pub fn face(&self, f: Face) -> &[StickerColor; 9] {
        match f {
            Face::U => &self.U,
            Face::D => &self.D,
            Face::L => &self.L,
            Face::R => &self.R,
            Face::F => &self.F,
            Face::B => &self.B,
        }
    }

    /// Non-fatal sanity check per README "Validation notes": 6 faces × 9, each color ×9.
    /// Returns warnings; never rejects (impossible states are allowed).
    pub fn sanity_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut counts = [0usize; 6];
        for f in Face::ALL {
            for &c in self.face(f) {
                let idx = StickerColor::ALL.iter().position(|&x| x == c).unwrap();
                counts[idx] += 1;
            }
        }
        for (i, &color) in StickerColor::ALL.iter().enumerate() {
            if counts[i] != 9 {
                warnings.push(format!(
                    "color {color:?} appears {} time(s), expected 9",
                    counts[i]
                ));
            }
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_normals_and_colors() {
        assert_eq!(Face::U.normal(), IVec3::new(0, 1, 0));
        assert_eq!(Face::D.normal(), IVec3::new(0, -1, 0));
        assert_eq!(Face::R.normal(), IVec3::new(1, 0, 0));
        assert_eq!(Face::L.normal(), IVec3::new(-1, 0, 0));
        assert_eq!(Face::F.normal(), IVec3::new(0, 0, 1));
        assert_eq!(Face::B.normal(), IVec3::new(0, 0, -1));

        assert_eq!(Face::U.solved_color(), StickerColor::W);
        assert_eq!(Face::D.solved_color(), StickerColor::Y);
        assert_eq!(Face::F.solved_color(), StickerColor::G);
        assert_eq!(Face::B.solved_color(), StickerColor::B);
        assert_eq!(Face::R.solved_color(), StickerColor::R);
        assert_eq!(Face::L.solved_color(), StickerColor::O);
    }

    #[test]
    fn quarter_turns() {
        assert_eq!(
            Move {
                face: Face::R,
                turn: Turn::Cw
            }
            .quarter_turns_cw(),
            1
        );
        assert_eq!(
            Move {
                face: Face::R,
                turn: Turn::Ccw
            }
            .quarter_turns_cw(),
            3
        );
        assert_eq!(
            Move {
                face: Face::R,
                turn: Turn::Double
            }
            .quarter_turns_cw(),
            2
        );
    }

    #[test]
    fn inverse_undoes_each_move() {
        for &m in &Move::ALL {
            // Cw↔Ccw, Double→Double; the face never changes.
            assert_eq!(m.inverse().face, m.face);
            match m.turn {
                Turn::Cw => assert_eq!(m.inverse().turn, Turn::Ccw),
                Turn::Ccw => assert_eq!(m.inverse().turn, Turn::Cw),
                Turn::Double => assert_eq!(m.inverse().turn, Turn::Double),
            }
            // Inverting twice is the identity.
            assert_eq!(m.inverse().inverse(), m);
            // The two quarter-turn counts sum to a full (no-op) rotation.
            assert_eq!(
                (m.quarter_turns_cw() + m.inverse().quarter_turns_cw()) % 4,
                0
            );
        }
    }

    // Test 7 (part 1): parse/notation round-trip over all 18; parse rejects junk;
    // serde_json::to_string(&StickerColor::W) == "\"W\"".
    #[test]
    fn parse_notation_round_trip() {
        assert_eq!(Move::ALL.len(), 18);
        for &m in &Move::ALL {
            assert_eq!(Move::parse(&m.notation()), Some(m));
        }
        // Exact expected forms for a representative face.
        assert_eq!(
            Move {
                face: Face::R,
                turn: Turn::Cw
            }
            .notation(),
            "R"
        );
        assert_eq!(
            Move {
                face: Face::R,
                turn: Turn::Ccw
            }
            .notation(),
            "R'"
        );
        assert_eq!(
            Move {
                face: Face::R,
                turn: Turn::Double
            }
            .notation(),
            "R2"
        );
    }

    #[test]
    fn parse_rejects_junk() {
        assert_eq!(Move::parse(""), None);
        assert_eq!(Move::parse("x"), None);
        assert_eq!(Move::parse("RR"), None);
        assert_eq!(Move::parse("R3"), None);
        assert_eq!(Move::parse("u"), None);
    }

    #[test]
    fn sticker_color_serializes_to_single_letter() {
        assert_eq!(serde_json::to_string(&StickerColor::W).unwrap(), "\"W\"");
        assert_eq!(serde_json::to_string(&StickerColor::Y).unwrap(), "\"Y\"");
        assert_eq!(serde_json::to_string(&StickerColor::R).unwrap(), "\"R\"");
        assert_eq!(serde_json::to_string(&StickerColor::O).unwrap(), "\"O\"");
        assert_eq!(serde_json::to_string(&StickerColor::B).unwrap(), "\"B\"");
        assert_eq!(serde_json::to_string(&StickerColor::G).unwrap(), "\"G\"");
        assert_eq!(
            serde_json::from_str::<StickerColor>("\"O\"").unwrap(),
            StickerColor::O
        );
    }

    // Test 7 (part 2): CubeState serde round-trips.
    #[test]
    fn cube_state_serde_round_trip() {
        let s = CubeState::solved();
        let json = serde_json::to_string(&s).unwrap();
        let back: CubeState = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn cube_state_solved_matches_readme_example() {
        let s = CubeState::solved();
        assert_eq!(s.U, [StickerColor::W; 9]);
        assert_eq!(s.R, [StickerColor::R; 9]);
        assert_eq!(s.F, [StickerColor::G; 9]);
        assert_eq!(s.D, [StickerColor::Y; 9]);
        assert_eq!(s.L, [StickerColor::O; 9]);
        assert_eq!(s.B, [StickerColor::B; 9]);
    }

    #[test]
    fn sanity_warnings_clean_for_solved_and_warns_for_impossible() {
        assert!(CubeState::solved().sanity_warnings().is_empty());
        let all_white = CubeState {
            U: [StickerColor::W; 9],
            R: [StickerColor::W; 9],
            F: [StickerColor::W; 9],
            D: [StickerColor::W; 9],
            L: [StickerColor::W; 9],
            B: [StickerColor::W; 9],
        };
        // 5 non-white colors should each be flagged (count 0), and white (count 54).
        assert!(!all_white.sanity_warnings().is_empty());
    }
}
