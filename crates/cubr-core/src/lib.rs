//! `cubr-core` — the pure, Bevy-free heart of `cubr`.
//!
//! This crate is the "frozen engine": an integer-math 3×3×3 cube model and a
//! guaranteed-optimal Korf IDA\* solver, with no rendering dependency. The Bevy
//! application (`cubr`) depends on it and mirrors its state into ECS entities.
//!
//! - [`core`] — the pure [`core::CubeCore`]: the single source of truth.
//! - [`model`] — [`model::StickerColor`], [`model::Face`], [`model::Move`],
//!   [`model::CubeState`] (the serde JSON shape, README cube-state contract).
//! - [`solver`] — the guaranteed-optimal solver: [`solver::build_or_load_pdbs`]
//!   / [`solver::solve`].

pub mod core;
pub mod model;
pub mod solver;
