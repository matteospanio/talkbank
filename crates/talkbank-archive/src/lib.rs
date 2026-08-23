//! TalkBank archive client: catalogue, metadata, access, downloads.
//!
//! No GTK in here: everything is testable without a user interface.

pub mod api;
pub mod batch;
pub mod cache;
pub mod catalog;
pub mod download;
pub mod index;

pub use api::{ApiError, Client, Downloadable, LoginOutcome, Table};
pub use catalog::{Archive, Folder, Media};
