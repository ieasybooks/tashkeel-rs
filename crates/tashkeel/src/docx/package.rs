//! ZIP 9 is deliberately the only ZIP reader used on untrusted input. Do not
//! replace this with docxml's unbounded OPC reader (which depends on ZIP 7).

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use docxml::xml::XmlTree;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::xml::{ParsedXml, is_element};
use super::{Limits, document};
use crate::Result;

const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const REL_BASES: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/",
];
const MAIN_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";

pub(super) struct StoryPart {
    // Resolved package references use ASCII-canonical names throughout.
    pub(super) name: String,
    pub(super) root: &'static str,
}

pub(super) struct Package<'a> {
    archive: ZipArchive<Cursor<&'a [u8]>>,
    // ASCII-canonical keys are only for lookup; ZIP metadata keeps the spelling.
    entries: BTreeMap<String, usize>,
    limits: Limits,
    inflated_size: usize,
}

struct Relationship {
    kind: String,
    target: String,
    external: bool,
}

struct ContentTypes {
    overrides: BTreeMap<String, String>,
    defaults: BTreeMap<String, String>,
}

impl ContentTypes {
    fn get(&self, canonical_name: &str) -> Option<&str> {
        // Defaults stay shared, even when a large value covers many tiny parts.
        self.overrides
            .get(canonical_name)
            .or_else(|| {
                let (_, extension) = canonical_name.rsplit_once('.')?;
                self.defaults.get(extension)
            })
            .map(String::as_str)
    }
}

impl<'a> Package<'a> {
    pub(super) fn read(input: &'a [u8], limits: Limits) -> Result<Self> {
        preflight_zip(input, limits)?;
        let mut archive = ZipArchive::new(Cursor::new(input)).map_err(zip_error)?;
        if archive.len() > limits.max_entries || archive.has_duplicate_names() {
            return Err(document(
                "ZIP has duplicate parts or exceeds the entry count limit",
            ));
        }
        archive.validate_header_consistency().map_err(zip_error)?;
        let mut entries = BTreeMap::new();
        let mut canonical_names = HashSet::new();
        let mut inflated_size = 0usize;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(zip_error)?;
            let mut name = entry.name().map_err(zip_error)?.into_owned();
            validate_name(&name, entry.is_dir())?;
            name.make_ascii_lowercase();
            if !canonical_names.insert(name.trim_end_matches('/').to_owned()) {
                return Err(document("ZIP contains equivalent or duplicate part names"));
            }
            if entry.is_symlink() || entry.encrypted() {
                return Err(document(
                    "encrypted or symbolic-link ZIP entries are unsupported",
                ));
            }
            if !matches!(
                entry.compression(),
                CompressionMethod::Stored | CompressionMethod::Deflated
            ) {
                return Err(document(
                    "DOCX ZIP entries must use Stored or Deflate compression",
                ));
            }
            let declared =
                usize::try_from(entry.size()).map_err(|_| document("ZIP part size overflow"))?;
            if declared > limits.max_part_bytes {
                return Err(document("ZIP part exceeds the inflated part size limit"));
            }
            inflated_size = inflated_size
                .checked_add(declared)
                .filter(|&size| size <= limits.max_total_uncompressed_bytes)
                .ok_or_else(|| document("ZIP exceeds the total inflated size limit"))?;
            // Verify every payload, including opaque images, against actual reads
            // and CRC. Neither an advertised size nor compression ratio is trusted.
            let bound = declared.saturating_add(1) as u64;
            let actual = io::copy(&mut (&mut entry).take(bound), &mut io::sink())
                .map_err(|err| document(format!("invalid ZIP payload: {err}")))?;
            if actual != declared as u64 {
                return Err(document("ZIP payload size differs from its declared size"));
            }
            if entry.is_dir() {
                if declared != 0 {
                    return Err(document("ZIP directory entries must be empty"));
                }
            } else {
                entries.insert(name, index);
            }
        }
        if entries
            .keys()
            .any(|name| name.starts_with("_xmlsignatures/"))
        {
            return Err(document(
                "digitally signed DOCX packages cannot be edited without invalidating signatures",
            ));
        }
        Ok(Self {
            archive,
            entries,
            limits,
            inflated_size,
        })
    }

    pub(super) fn inflated_size(&self) -> usize {
        self.inflated_size
    }

    pub(super) fn read_part(&mut self, name: &str) -> Result<Vec<u8>> {
        let index = *self
            .entries
            .get(name)
            .ok_or_else(|| document(format!("missing DOCX part: {name}")))?;
        let mut file = self.archive.by_index(index).map_err(zip_error)?;
        if file.size() > self.limits.max_xml_bytes as u64 {
            return Err(document(format!(
                "XML part exceeds the XML size limit: {name}"
            )));
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(self.limits.max_xml_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|err| document(format!("invalid ZIP payload: {err}")))?;
        if bytes.len() > self.limits.max_xml_bytes {
            return Err(document("XML part exceeds the XML size limit"));
        }
        Ok(bytes)
    }

    fn relationships(&mut self, source: &str) -> Result<Vec<Relationship>> {
        let part = if source.is_empty() {
            "_rels/.rels".to_owned()
        } else if let Some((directory, file)) = source.rsplit_once('/') {
            format!("{directory}/_rels/{file}.rels")
        } else {
            format!("_rels/{source}.rels")
        };
        if !source.is_empty() && !self.entries.contains_key(&part) {
            return Ok(Vec::new());
        }
        let bytes = self.read_part(&part)?;
        let xml = ParsedXml::parse(&bytes, self.limits)?;
        let tree = &xml.tree;
        if !is_element(&xml, tree.root(), RELS, "Relationships") {
            return Err(document("invalid package relationships root"));
        }
        let mut ids = HashSet::new();
        let mut relationships = Vec::new();
        for &node in tree.children(tree.root()) {
            if tree.name(node).is_none() {
                continue;
            }
            if !is_element(&xml, node, RELS, "Relationship") {
                return Err(document("unexpected element in package relationships"));
            }
            let id = required(tree, node, "Id")?;
            if !ids.insert(id) {
                return Err(document("duplicate relationship identifier"));
            }
            let kind = required(tree, node, "Type")?.to_owned();
            if kind.contains("/digital-signature/") {
                return Err(document("digitally signed DOCX packages are unsupported"));
            }
            let external = match tree.attr(node, "TargetMode") {
                None | Some("Internal") => false,
                Some("External") => true,
                Some(_) => return Err(document("unknown relationship target mode")),
            };
            let target = required(tree, node, "Target")?;
            let target = if external {
                target.to_owned()
            } else {
                let target = resolve_target(source, target)?;
                if !self.entries.contains_key(&target) {
                    return Err(document(format!(
                        "missing internal relationship target: {target}"
                    )));
                }
                target
            };
            relationships.push(Relationship {
                kind,
                target,
                external,
            });
        }
        Ok(relationships)
    }

    pub(super) fn stories(&mut self) -> Result<Vec<StoryPart>> {
        let types = self.content_types()?;
        let root_rels = self.relationships("")?;
        let mut main = root_rels
            .iter()
            .filter(|rel| relationship_kind(&rel.kind) == Some("officeDocument"));
        let main = main
            .next()
            .filter(|rel| !rel.external)
            .filter(|_| main.next().is_none())
            .ok_or_else(|| {
                document("DOCX must have exactly one internal officeDocument relationship")
            })?;
        if types.get(&main.target) != Some(MAIN_TYPE) {
            return Err(document(
                "only DOCX documents are supported; templates and macro-enabled documents are unsupported",
            ));
        }
        let mut pending = VecDeque::from([StoryPart {
            name: main.target.clone(),
            root: "document",
        }]);
        let mut seen = BTreeMap::new();
        let mut stories = Vec::new();
        while let Some(part) = pending.pop_front() {
            if let Some(previous) = seen.insert(part.name.clone(), part.root) {
                if previous != part.root {
                    return Err(document(
                        "a DOCX story part has conflicting relationship types",
                    ));
                }
                continue;
            }
            for rel in self.relationships(&part.name)? {
                let Some(kind) = relationship_kind(&rel.kind) else {
                    continue;
                };
                let root = match kind {
                    "header" => "hdr",
                    "footer" => "ftr",
                    "footnotes" => "footnotes",
                    "endnotes" => "endnotes",
                    _ => continue,
                };
                if rel.external {
                    return Err(document("external DOCX story parts are unsupported"));
                }
                let expected = format!(
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml"
                );
                if types.get(&rel.target) != Some(expected.as_str()) {
                    return Err(document(format!("invalid content type for {}", rel.target)));
                }
                pending.push_back(StoryPart {
                    name: rel.target,
                    root,
                });
            }
            stories.push(part);
        }
        Ok(stories)
    }

    fn content_types(&mut self) -> Result<ContentTypes> {
        let bytes = self.read_part("[content_types].xml")?;
        let xml = ParsedXml::parse(&bytes, self.limits)?;
        let tree = &xml.tree;
        if !is_element(&xml, tree.root(), TYPES, "Types") {
            return Err(document("invalid package content-types root"));
        }
        let mut overrides = BTreeMap::new();
        let mut defaults = BTreeMap::new();
        for &node in tree.children(tree.root()) {
            if tree.name(node).is_none() {
                continue;
            }
            let content_type = required(tree, node, "ContentType")?.to_owned();
            if content_type.contains("digital-signature") {
                return Err(document("digitally signed DOCX packages are unsupported"));
            }
            if is_element(&xml, node, TYPES, "Override") {
                let name = required(tree, node, "PartName")?;
                if !name.starts_with('/') {
                    return Err(document("content-type part names must be absolute"));
                }
                let name = resolve_target("", name)?;
                if overrides.insert(name, content_type).is_some() {
                    return Err(document("duplicate content-type override"));
                }
            } else if is_element(&xml, node, TYPES, "Default") {
                let extension = required(tree, node, "Extension")?.to_ascii_lowercase();
                if defaults.insert(extension, content_type).is_some() {
                    return Err(document("duplicate content-type extension"));
                }
            } else {
                return Err(document("unexpected element in package content types"));
            }
        }
        Ok(ContentTypes {
            overrides,
            defaults,
        })
    }

    pub(super) fn write(
        mut self,
        original: &[u8],
        changed: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Vec<u8>> {
        if changed.is_empty() {
            if original.len() > self.limits.max_output_bytes {
                return Err(document("DOCX exceeds the output size limit"));
            }
            return Ok(original.to_vec());
        }
        let output = BoundedWriter {
            cursor: Cursor::new(Vec::new()),
            limit: self.limits.max_output_bytes,
            failed: false,
        };
        let mut writer = ZipWriter::new(output);
        writer
            .set_raw_comment(self.archive.comment().into())
            .map_err(zip_error)?;
        for index in 0..self.archive.len() {
            let entry = self.archive.by_index(index).map_err(zip_error)?;
            let name = entry.name().map_err(zip_error)?.into_owned();
            if let Some(bytes) = changed.get(&name.to_ascii_lowercase()) {
                let options = entry
                    .options()
                    .into_full_options()
                    .with_file_comment(entry.comment());
                writer.start_file(&name, options).map_err(zip_error)?;
                writer
                    .write_all(bytes)
                    .map_err(|err| document(format!("cannot write DOCX: {err}")))?;
            } else {
                // Raw copying preserves original compressed payloads as well as
                // all uncompressed bytes of parts this transformer never edits.
                writer.raw_copy_file(entry).map_err(zip_error)?;
            }
        }
        let output = writer.finish().map_err(zip_error)?;
        if output.failed {
            return Err(document("DOCX exceeds the output size limit"));
        }
        Ok(output.cursor.into_inner())
    }
}

fn required<'a>(tree: &'a XmlTree, node: docxml::xml::NodeId, name: &str) -> Result<&'a str> {
    tree.attr(node, name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| document(format!("missing package attribute: {name}")))
}

fn relationship_kind(kind: &str) -> Option<&str> {
    REL_BASES.iter().find_map(|base| kind.strip_prefix(base))
}

fn validate_name(name: &str, directory: bool) -> Result<()> {
    let trimmed = if directory {
        name.strip_suffix('/').unwrap_or(name)
    } else {
        name
    };
    if trimmed.is_empty()
        || name.len() > 1024
        || trimmed.starts_with('/')
        || trimmed
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '\\' | ':' | '%' | '?' | '#'))
        || trimmed
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.ends_with('.'))
    {
        return Err(document(format!(
            "unsupported or unsafe ZIP part name: {name:?}"
        )));
    }
    Ok(())
}

fn resolve_target(source: &str, target: &str) -> Result<String> {
    if target.contains(['\\', ':', '%', '?', '#']) || target.starts_with("//") {
        return Err(document("unsupported internal package URI"));
    }
    let directory = source
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);
    let mut parts: Vec<&str> = if target.starts_with('/') || directory.is_empty() {
        Vec::new()
    } else {
        directory.split('/').collect()
    };
    for segment in target.trim_start_matches('/').split('/') {
        match segment {
            "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(document("package URI escapes the package root"));
                }
            }
            "" => return Err(document("empty package URI segment")),
            _ => parts.push(segment),
        }
    }
    let mut result = parts.join("/");
    validate_name(&result, false)?;
    result.make_ascii_lowercase();
    Ok(result)
}

/// Bound the central-directory count before ZIP allocates metadata. ZIP64 is
/// intentionally refused: these in-memory limits do not need multi-GiB files.
fn preflight_zip(input: &[u8], limits: Limits) -> Result<()> {
    if input.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
        return Err(document(
            "legacy DOC and encrypted Office files are unsupported; use an unencrypted DOCX",
        ));
    }
    if input.len() > limits.max_archive_bytes {
        return Err(document("ZIP exceeds the input size limit"));
    }
    if !input.starts_with(b"PK\x03\x04") || input.len() < 22 {
        return Err(document("input is not a DOCX ZIP package"));
    }
    let end = (input.len().saturating_sub(65_557)..=input.len() - 22)
        .rev()
        .find(|&offset| {
            input[offset..].starts_with(b"PK\x05\x06")
                && offset + 22 + usize::from(le16(input, offset + 20)) == input.len()
        })
        .ok_or_else(|| document("missing or invalid ZIP end record"))?;
    let count = usize::from(le16(input, end + 10));
    let size = le32(input, end + 12) as usize;
    let start = le32(input, end + 16) as usize;
    if le16(input, end + 4) != 0
        || le16(input, end + 6) != 0
        || usize::from(le16(input, end + 8)) != count
        || count == usize::from(u16::MAX)
        || size == u32::MAX as usize
        || start == u32::MAX as usize
    {
        return Err(document(
            "ZIP64 and multi-volume ZIP packages are unsupported",
        ));
    }
    if count > limits.max_entries {
        return Err(document("ZIP exceeds the entry count limit"));
    }
    if start.checked_add(size) != Some(end) {
        return Err(document("invalid ZIP central directory bounds"));
    }
    let mut position = start;
    for _ in 0..count {
        if position.checked_add(46).is_none_or(|next| next > end)
            || !input[position..].starts_with(b"PK\x01\x02")
        {
            return Err(document("invalid ZIP central directory entry"));
        }
        let length = 46
            + usize::from(le16(input, position + 28))
            + usize::from(le16(input, position + 30))
            + usize::from(le16(input, position + 32));
        position = position
            .checked_add(length)
            .filter(|&next| next <= end)
            .ok_or_else(|| document("invalid ZIP central directory length"))?;
    }
    if position != end {
        return Err(document(
            "ZIP entry count does not match the central directory",
        ));
    }
    Ok(())
}

fn le16(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([input[offset], input[offset + 1]])
}

fn le32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
    ])
}

fn zip_error(err: impl std::fmt::Display) -> crate::Error {
    document(format!("invalid or unsupported ZIP package: {err}"))
}

struct BoundedWriter {
    cursor: Cursor<Vec<u8>>,
    limit: usize,
    failed: bool,
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // ZipWriter finalizes again in Drop after an error. Discard those writes
        // without allocating or printing a second failure to the caller's stderr.
        if self.failed {
            self.cursor
                .set_position(self.cursor.position().saturating_add(bytes.len() as u64));
            return Ok(bytes.len());
        }
        if self
            .cursor
            .position()
            .checked_add(bytes.len() as u64)
            .is_none_or(|end| end > self.limit as u64)
        {
            self.failed = true;
            return Err(io::Error::other("DOCX exceeds the output size limit"));
        }
        self.cursor.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for BoundedWriter {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.cursor.seek(position)
    }
}

#[cfg(test)]
mod tests {
    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn large_content_type_default_is_shared_across_many_parts() {
        const TINY_PARTS: usize = 192;
        let default_type = format!("application/{}", "x".repeat(1024 * 1024 - 12));
        let types_xml = format!(
            "<Types xmlns='{TYPES}'><Default Extension='BIN' ContentType='{default_type}'/><Override PartName='/word/document.xml' ContentType='{MAIN_TYPE}'/></Types>"
        );
        let root_rels = format!(
            "<Relationships xmlns='{RELS}'><Relationship Id='main' Type='{}officeDocument' Target='word/document.xml'/></Relationships>",
            REL_BASES[0]
        );
        let document = b"<w:document xmlns:w='http://schemas.openxmlformats.org/wordprocessingml/2006/main'><w:body/></w:document>";
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, bytes) in [
            ("[Content_Types].xml", types_xml.as_bytes()),
            ("_rels/.rels", root_rels.as_bytes()),
            ("word/document.xml", document.as_slice()),
        ] {
            writer.start_file(name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        for index in 0..TINY_PARTS {
            writer
                .start_file(format!("tiny/part-{index}.BIN"), options)
                .unwrap();
            writer.write_all(b"x").unwrap();
        }
        let input = writer.finish().unwrap().into_inner();
        let mut package = Package::read(&input, Limits::default()).unwrap();
        let types = package.content_types().unwrap();

        // Prove bounded storage and shared borrows without measuring the process
        // allocator or reproducing the former 192 MiB of expanded defaults.
        assert_eq!(types.defaults.len(), 1);
        assert_eq!(types.overrides.len(), 1);
        let default = types.defaults["bin"].as_str();
        assert_eq!(default, default_type);
        let mut matched = 0;
        for name in package.entries.keys().filter(|name| {
            name.rsplit_once('.')
                .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("bin"))
        }) {
            assert!(std::ptr::eq(types.get(name).unwrap(), default));
            matched += 1;
        }
        assert_eq!(matched, TINY_PARTS);
        assert_eq!(types.get("word/document.xml"), Some(MAIN_TYPE));
        assert_eq!(types.get("unknown.part"), None);
        assert_eq!(
            super::super::transform(&input, |text| Ok(text.to_owned())).unwrap(),
            input
        );
    }
}
