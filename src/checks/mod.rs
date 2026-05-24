// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Individual checks. Each module implements [`Check`] over its own
//! struct, keeping the runner agnostic of the cargo invocation details.
//!
//! Cross-cutting machinery lives in:
//! - [`runner`]: spawning strategy ([`Runner`], [`CargoCli`]).
//! - [`plan`]: [`Check`] trait, [`Plan`] and the serial [`chain`].
//! - [`util`]: shared helpers ([`cargo_outcome`], [`fmt_cargo_cmd`],
//!   [`combine_streams`], [`COMMON_ARGS`]).

pub mod audit;
pub mod clippy;
pub mod compile;
pub mod coverage;
pub mod doc;
pub mod doctest;
pub mod fmt;
pub mod license_header;
pub mod machete;
pub mod test;

pub mod plan;
pub mod runner;
pub mod util;

pub use plan::{Check, Plan, build_plan, chain};
pub use runner::{CargoCli, Runner};
pub use util::{
    COMMON_ARGS, cargo_outcome, cargo_outcome_with_env, combine_streams, fmt_cargo_cmd,
};
