//! Small shared geometry helpers (no Bevy systems).

use bevy::math::Vec3;

/// The candidate whose direction has the greatest dot product with `target`.
/// Ties resolve to the EARLIER candidate (strict `>`), so callers control
/// tie-breaks via candidate order. Panics if `candidates` is empty.
pub fn best_by_dot<T>(target: Vec3, candidates: impl IntoIterator<Item = (T, Vec3)>) -> T {
    let mut iter = candidates.into_iter();
    let (mut best, first_dir) = iter.next().expect("best_by_dot: no candidates");
    let mut best_dot = target.dot(first_dir);
    for (value, dir) in iter {
        let d = target.dot(dir);
        if d > best_dot {
            best_dot = d;
            best = value;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_max_dot() {
        let cands = [(1, Vec3::X), (2, Vec3::Y), (3, Vec3::Z)];
        assert_eq!(best_by_dot(Vec3::new(0.1, 0.9, 0.2), cands), 2);
    }

    #[test]
    fn ties_go_to_first() {
        // X and Y tie for the dot with (1,1,0); the earlier candidate wins.
        let cands = [('x', Vec3::X), ('y', Vec3::Y)];
        assert_eq!(best_by_dot(Vec3::new(1.0, 1.0, 0.0), cands), 'x');
    }
}
