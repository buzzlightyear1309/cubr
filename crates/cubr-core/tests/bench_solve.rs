//! Throwaway discovery benchmark (NOT part of the gate). Times deep optimal solves
//! on the current single-threaded max-of-three solver to see whether the deepest
//! states genuinely take minutes (algorithmic) or whether something is wrong.
//!
//! Run with:
//!   cargo test -p cubr-core --release --test bench_solve -- --ignored --nocapture

use cubr_core::core::CubeCore;
use cubr_core::model::Move;
use cubr_core::solver::{build_or_load_pdbs, solve, SolveError, Solver};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Numerical-Recipes LCG; deterministic, no `rand`.
fn lcg(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *seed
}

#[test]
#[ignore = "discovery benchmark; run explicitly in release with --ignored --nocapture"]
fn bench_deep_solves() {
    let t0 = Instant::now();
    let pdbs = build_or_load_pdbs();
    println!("PDB load/build took {:?}", t0.elapsed());

    let cap = Duration::from_secs(30);
    let mut seed = 0xBEEF_0001u32;

    for trial in 0..10 {
        // Long scrambles to land on deep (often distance 17-20) states.
        let len = 24 + (lcg(&mut seed) as usize % 6); // 24..=29
        let mut core = CubeCore::solved();
        for _ in 0..len {
            core.apply(Move::ALL[(lcg(&mut seed) as usize) % 18]);
        }
        let state = core.to_state();

        // Watchdog: cancel the solve if it runs past `cap`, so the bench can't hang.
        let cancel = Arc::new(AtomicBool::new(false));
        let wd_flag = Arc::clone(&cancel);
        let wd = std::thread::spawn(move || {
            let start = Instant::now();
            while start.elapsed() < cap {
                if wd_flag.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            wd_flag.store(true, Ordering::Relaxed);
        });

        let t = Instant::now();
        let res = solve(&pdbs, &state, &cancel);
        let dt = t.elapsed();
        cancel.store(true, Ordering::Relaxed); // stop the watchdog
        let _ = wd.join();

        match res {
            Ok(sol) => println!(
                "trial {trial:2}: scramble_len={len} -> solved in {} moves, {:?}",
                sol.len(),
                dt
            ),
            Err(SolveError::Cancelled) => {
                println!("trial {trial:2}: scramble_len={len} -> TIMEOUT (>{cap:?})")
            }
            Err(SolveError::Unsolvable) => {
                println!("trial {trial:2}: scramble_len={len} -> UNSOLVABLE (unexpected)")
            }
        }
    }
}

/// Discovery benchmark for the **hybrid** path: builds a reusable [`Solver`] once, then
/// times ~10 deterministic deep scrambles through `Solver::solve`. It reads the ambient
/// `CUBR_KORF_BUDGET_MS`, so a runner can shrink the Korf budget (e.g.
/// `CUBR_KORF_BUDGET_MS=200`) to force the near-optimal two-phase fallback and see deep
/// states solve in milliseconds rather than the minutes a guaranteed-optimal search can take.
#[test]
#[ignore = "discovery benchmark; run explicitly in release with --ignored --nocapture"]
fn bench_hybrid_deep() {
    let t0 = Instant::now();
    let pdbs = build_or_load_pdbs();
    let solver = Solver::new(pdbs);
    println!("PDB load/build + Solver::new took {:?}", t0.elapsed());
    if let Ok(b) = std::env::var("CUBR_KORF_BUDGET_MS") {
        println!("CUBR_KORF_BUDGET_MS = {b}");
    }

    let mut seed = 0x41B0_0001u32;

    for trial in 0..10 {
        // Deep scrambles (length 24..=30) to land on hard, often distance 17-20 states.
        let len = 24 + (lcg(&mut seed) as usize % 7); // 24..=30
        let mut core = CubeCore::solved();
        for _ in 0..len {
            core.apply(Move::ALL[(lcg(&mut seed) as usize) % 18]);
        }
        let state = core.to_state();

        let cancel = AtomicBool::new(false);
        let t = Instant::now();
        let res = solver.solve(&state, &cancel);
        let dt = t.elapsed();

        match res {
            Ok(sol) => println!(
                "trial {trial:2}: scramble_len={len} -> solved in {} moves, {:?}",
                sol.len(),
                dt
            ),
            Err(SolveError::Cancelled) => {
                println!("trial {trial:2}: scramble_len={len} -> CANCELLED (unexpected)")
            }
            Err(SolveError::Unsolvable) => {
                println!("trial {trial:2}: scramble_len={len} -> UNSOLVABLE (unexpected)")
            }
        }
    }
}
