//! Versioned, std-only disk cache for the three Korf pattern databases.
//!
//! The PDBs are large (~85 MB packed) and slow to generate, so we build once and cache
//! to `~/.cache/cube/korf-pdb.bin`. A stale cache written under a *different* edge split
//! / index formula / move convention would silently produce wrong (non-optimal or
//! incorrect) solutions, so the file carries a [`CACHE_VERSION`] that MUST be bumped on
//! any such change, plus exact-length validation. Any mismatch / short read / IO error
//! makes [`load`] return `None` so the caller regenerates.
//!
//! Not yet wired into the live solver (Unit K2 provides the primitives; `build_or_load`
//! and the public `solve` rewrite are Unit K4).

use super::pdb::{Pdbs, CORNER_SIZE, EDGE_SIZE};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// On-disk format version. **Bump this on any change to the edge split, the index
/// formulas, or the move convention** — a stale cache with a different convention is a
/// silent-wrong-result trap, and this field is the guard.
pub(crate) const CACHE_VERSION: u32 = 1;

/// File magic: 8 bytes. The trailing `\x01` is part of the magic (format identifier),
/// distinct from [`CACHE_VERSION`] which validates the encoding convention.
const MAGIC: &[u8; 8] = b"CUBEPDB\x01";

/// Cache filename within the cache directory.
const FILENAME: &str = "korf-pdb.bin";

/// Expected packed (nibble) byte length of the corner PDB blob.
fn corner_packed_len() -> usize {
    CORNER_SIZE.div_ceil(2)
}

/// Expected packed (nibble) byte length of each per-group edge PDB blob.
fn edge_packed_len() -> usize {
    EDGE_SIZE.div_ceil(2)
}

/// Resolve the cache file path (std-only, no `dirs` crate):
/// `$XDG_CACHE_HOME/cube/` else `$HOME/.cache/cube/` else `temp_dir()/cube/`,
/// with filename [`FILENAME`].
pub(crate) fn cache_path() -> PathBuf {
    let dir = if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME").filter(|s| !s.is_empty()) {
        PathBuf::from(xdg).join("cube")
    } else if let Some(home) = std::env::var_os("HOME").filter(|s| !s.is_empty()) {
        PathBuf::from(home).join(".cache").join("cube")
    } else {
        std::env::temp_dir().join("cube")
    };
    dir.join(FILENAME)
}

/// Load and validate the cached PDBs from `path`.
///
/// Parses the header `MAGIC (8) | version u32 LE | len_corner u64 | len_edge_a u64 |
/// len_edge_b u64`, then the three blobs. Returns `None` on any mismatch (bad magic,
/// wrong version, wrong blob lengths, short read, IO error) so the caller regenerates.
pub(crate) fn load(path: &Path) -> Option<Pdbs> {
    let mut file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;

    // Header: 8 + 4 + 8*3 = 36 bytes.
    const HEADER: usize = 8 + 4 + 8 * 3;
    if bytes.len() < HEADER {
        return None;
    }
    if &bytes[0..8] != MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
    if version != CACHE_VERSION {
        return None;
    }
    let len_corner = u64::from_le_bytes(bytes[12..20].try_into().ok()?) as usize;
    let len_edge_a = u64::from_le_bytes(bytes[20..28].try_into().ok()?) as usize;
    let len_edge_b = u64::from_le_bytes(bytes[28..36].try_into().ok()?) as usize;

    if len_corner != corner_packed_len()
        || len_edge_a != edge_packed_len()
        || len_edge_b != edge_packed_len()
    {
        return None;
    }

    // Remaining bytes must match the three blobs exactly (no trailing slack).
    let total = HEADER + len_corner + len_edge_a + len_edge_b;
    if bytes.len() != total {
        return None;
    }

    let mut off = HEADER;
    let corner = bytes[off..off + len_corner].to_vec();
    off += len_corner;
    let edge_a = bytes[off..off + len_edge_a].to_vec();
    off += len_edge_a;
    let edge_b = bytes[off..off + len_edge_b].to_vec();

    Some(Pdbs {
        corner,
        edge_a,
        edge_b,
    })
}

/// Atomically write `pdbs` to `path`: create the parent dir, write the header + three
/// blobs to `<path>.tmp`, then rename over `path`.
pub(crate) fn save(path: &Path, pdbs: &Pdbs) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(MAGIC)?;
        file.write_all(&CACHE_VERSION.to_le_bytes())?;
        file.write_all(&(pdbs.corner.len() as u64).to_le_bytes())?;
        file.write_all(&(pdbs.edge_a.len() as u64).to_le_bytes())?;
        file.write_all(&(pdbs.edge_b.len() as u64).to_le_bytes())?;
        file.write_all(&pdbs.corner)?;
        file.write_all(&pdbs.edge_a)?;
        file.write_all(&pdbs.edge_b)?;
        file.flush()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake `Pdbs` with correctly-sized blobs filled with a deterministic pattern.
    /// Does NOT call the slow `Pdbs::generate()`.
    fn fake_pdbs() -> Pdbs {
        let mut corner = vec![0u8; corner_packed_len()];
        let mut edge_a = vec![0u8; edge_packed_len()];
        let mut edge_b = vec![0u8; edge_packed_len()];
        for (i, b) in corner.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(31).wrapping_add(7);
        }
        for (i, b) in edge_a.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(13).wrapping_add(1);
        }
        for (i, b) in edge_b.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(17).wrapping_add(3);
        }
        Pdbs {
            corner,
            edge_a,
            edge_b,
        }
    }

    /// Unique temp path per test (no collisions across parallel runs).
    fn temp_path(tag: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("cube-cache-test-{tag}-{pid}-{nanos}.bin"))
    }

    #[test]
    fn save_load_roundtrip() {
        let path = temp_path("roundtrip");
        let pdbs = fake_pdbs();
        save(&path, &pdbs).expect("save failed");
        let loaded = load(&path).expect("load failed");
        assert_eq!(loaded.corner, pdbs.corner);
        assert_eq!(loaded.edge_a, pdbs.edge_a);
        assert_eq!(loaded.edge_b, pdbs.edge_b);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rejects_bad_magic() {
        let path = temp_path("magic");
        save(&path, &fake_pdbs()).expect("save failed");
        let mut bytes = fs::read(&path).unwrap();
        bytes[0] = b'X'; // corrupt magic
        fs::write(&path, &bytes).unwrap();
        assert!(load(&path).is_none(), "bad magic must reject");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rejects_wrong_version() {
        let path = temp_path("version");
        save(&path, &fake_pdbs()).expect("save failed");
        let mut bytes = fs::read(&path).unwrap();
        // Bump the version field (bytes 8..12, LE).
        let v = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) + 1;
        bytes[8..12].copy_from_slice(&v.to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        assert!(load(&path).is_none(), "wrong version must reject");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rejects_truncated_blob() {
        let path = temp_path("trunc");
        save(&path, &fake_pdbs()).expect("save failed");
        let mut bytes = fs::read(&path).unwrap();
        bytes.truncate(bytes.len() - 1); // drop a byte from the last blob
        fs::write(&path, &bytes).unwrap();
        assert!(load(&path).is_none(), "truncated blob must reject");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rejects_missing_file() {
        let path = temp_path("missing");
        let _ = fs::remove_file(&path);
        assert!(load(&path).is_none(), "missing file must return None");
    }
}
