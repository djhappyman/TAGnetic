//! TagRex core.
//!
//! Module boundaries follow `docs/architecture.md` in the repository root.
//! The one invariant everything else follows from: **no module writes tags or
//! renames files directly** — every operation is compiled into a
//! [`plan::ChangePlan`], previewed, applied through [`plan::Executor`], and
//! journaled for rollback. Newly encoded audio can be tagged in private,
//! identity-bound staging before any output is published.

pub mod cover;
pub mod export;
pub mod journal;
pub mod mask;
pub mod matching;
pub mod media_mutation;
pub mod model;
pub mod plan;
pub mod provider;
pub mod report_template;
pub mod resources;
pub mod scanner;
pub mod text_codec;
pub mod transform;
pub mod values;
