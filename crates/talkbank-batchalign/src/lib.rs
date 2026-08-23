//! Client for the local Batchalign3 server.
//!
//! Batchalign3 is TalkBank's ASR and morphosyntax pipeline. We need it for a
//! specific reason: the classic MOR grammars now cover only English, French,
//! Spanish and Chinese, while Batchalign covers roughly 26 languages through
//! Stanza/UD. Without it our "the %mor tier is missing" warning would have no
//! answer to offer for the rest.
//!
//! We talk to its **local HTTP control plane**, not to its crates: that is the
//! contract their own desktop app uses, it is described by a versioned
//! `openapi.json`, and it does not drag their ML build into ours.

pub mod client;
pub mod server;
pub mod types;

pub use client::{Client, Error};
pub use server::{Availability, Server};
pub use types::{Command, FailureCategory, JobInfo, Status, Submission};
