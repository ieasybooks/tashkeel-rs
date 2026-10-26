//! CPU Arabic diacritization with preservation of source text and DOCX formatting.

pub mod docx;
mod error;
mod inference;
pub mod models;
pub mod text;

pub use error::{Error, Result};
pub use inference::{Diacritizer, InferenceOptions, Model, Progress};
