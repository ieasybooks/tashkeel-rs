//! Diacritize DOCX text while retaining the original package and XML markup.
//!
//! Paragraphs are read with `docxml`'s retained XML tree. Only the eight common
//! Arabic marks (U+064B–U+0652) may change. Marks are assigned to the original
//! base character's text node, even when a word spans differently styled runs.
//! See `docs/docx.md` for current-view traversal and unsupported structures.

mod package;
mod story;
mod xml;

use std::collections::BTreeMap;

use crate::{Error, Result};

/// Resource bounds for a DOCX transformation.
///
/// Bounds apply before parsing or allocation where possible and during actual
/// decompression and output writes. A caller-provided closure is responsible
/// for its own execution time and allocations; its returned text is checked.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum input ZIP size in bytes (default: 128 MiB).
    pub max_archive_bytes: usize,
    /// Maximum number of ZIP entries, including directories (default: 4,096).
    pub max_entries: usize,
    /// Maximum inflated size of one ZIP entry (default: 32 MiB).
    pub max_part_bytes: usize,
    /// Maximum total inflated ZIP size (default: 256 MiB).
    pub max_total_uncompressed_bytes: usize,
    /// Maximum size of each parsed or edited XML part (default: 8 MiB).
    pub max_xml_bytes: usize,
    /// Maximum XML nesting depth (default: 128; at most 256).
    pub max_xml_depth: usize,
    /// Maximum XML events per parsed part (default: 200,000).
    pub max_xml_events: usize,
    /// Maximum attributes per element, including namespace declarations
    /// (default: 128; at most 256).
    pub max_xml_attributes_per_element: usize,
    /// Maximum paragraph input and closure output in UTF-8 bytes (default: 1 MiB).
    pub max_paragraph_bytes: usize,
    /// Maximum returned ZIP size (default: 256 MiB).
    pub max_output_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_archive_bytes: 128 * 1024 * 1024,
            max_entries: 4096,
            max_part_bytes: 32 * 1024 * 1024,
            max_total_uncompressed_bytes: 256 * 1024 * 1024,
            max_xml_bytes: 8 * 1024 * 1024,
            max_xml_depth: 128,
            max_xml_events: 200_000,
            max_xml_attributes_per_element: 128,
            max_paragraph_bytes: 1024 * 1024,
            max_output_bytes: 256 * 1024 * 1024,
        }
    }
}

impl Limits {
    fn validate(self) -> Result<()> {
        if [
            self.max_archive_bytes,
            self.max_entries,
            self.max_part_bytes,
            self.max_total_uncompressed_bytes,
            self.max_xml_bytes,
            self.max_xml_depth,
            self.max_xml_events,
            self.max_xml_attributes_per_element,
            self.max_paragraph_bytes,
            self.max_output_bytes,
        ]
        .contains(&0)
            || self.max_xml_depth > 256
            || self.max_xml_attributes_per_element > 256
        {
            return Err(Error::InvalidInput(
                "DOCX limits must be positive; XML depth and attributes per element must not exceed 256".into(),
            ));
        }
        Ok(())
    }
}

/// Transform contextual paragraph text in a DOCX package using default limits.
///
/// The callback receives text across runs and hyperlinks, including existing
/// marks. Tabs and breaks appear as `\t` and `\n`; opaque inline objects appear
/// as U+FFFC. All characters except U+064B–U+0652 must remain exactly unchanged.
/// Body, table, header, footer, footnote, endnote, and Word text-box paragraphs
/// are processed. Deleted revisions and field instructions are left untouched.
///
/// Callback errors, including [`Error::Cancelled`], propagate unchanged. No
/// partial document is returned. An unchanged document is returned byte for
/// byte. Legacy binary `.doc` files are explicitly unsupported.
pub fn transform(input: &[u8], transform: impl FnMut(&str) -> Result<String>) -> Result<Vec<u8>> {
    transform_with_limits(input, Limits::default(), transform)
}

/// Transform a DOCX package with explicit resource limits.
///
/// See [`transform`] for the callback's preservation contract. Invalid limits
/// produce [`Error::InvalidInput`]; malformed or unsupported documents and
/// preservation violations produce [`Error::Document`].
pub fn transform_with_limits(
    input: &[u8],
    limits: Limits,
    mut transform: impl FnMut(&str) -> Result<String>,
) -> Result<Vec<u8>> {
    limits.validate()?;
    let mut package = package::Package::read(input, limits)?;
    let stories = package.stories()?;
    let mut changed = BTreeMap::new();
    let mut inflated_size = package.inflated_size();
    for part in stories {
        let original = package.read_part(&part.name)?;
        let xml = xml::ParsedXml::parse(&original, limits)?;
        if !xml::is_word(&xml, xml.tree.root(), part.root) {
            return Err(document(format!("invalid root element in {}", part.name)));
        }
        if let Some(updated) = story::transform(&xml, &original, limits, &mut transform)? {
            if updated.len() > limits.max_part_bytes {
                return Err(document("edited part exceeds the part size limit"));
            }
            inflated_size = inflated_size
                .checked_sub(original.len())
                .and_then(|size| size.checked_add(updated.len()))
                .filter(|&size| size <= limits.max_total_uncompressed_bytes)
                .ok_or_else(|| document("edited package exceeds the inflated size limit"))?;
            changed.insert(part.name, updated);
        }
    }
    package.write(input, &changed)
}

fn document(message: impl Into<String>) -> Error {
    Error::Document(message.into())
}

fn is_mark(ch: char) -> bool {
    matches!(ch, '\u{064b}'..='\u{0652}')
}
