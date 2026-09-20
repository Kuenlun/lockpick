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

pub(crate) mod audit;
pub(crate) mod clippy;
pub(crate) mod compile;
pub(crate) mod coverage;
pub(crate) mod doc;
pub(crate) mod doctest;
pub(crate) mod fmt;
pub(crate) mod license_header;
pub(crate) mod machete;
pub(crate) mod targets;
pub(crate) mod test;

pub(crate) mod plan;
pub(crate) mod runner;
pub(crate) mod util;

pub(crate) use plan::{Check, Plan, build_plan, chain};
pub(crate) use runner::{CargoCli, Runner};
pub(crate) use util::{COMMON_ARGS, cargo_outcome, combine_streams, fmt_cargo_cmd};
