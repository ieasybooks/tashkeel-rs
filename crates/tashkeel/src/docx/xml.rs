//! Bounded XML parsing and byte-preserving text patches.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use docxml::xml::{NodeId, NodeKind, XmlTree};
use quick_xml::Reader;
use quick_xml::events::Event;

use super::{Limits, document, is_mark};
use crate::Result;

pub(super) const WORD: [&str; 2] = [
    "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    "http://purl.oclc.org/ooxml/wordprocessingml/main",
];
pub(super) const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const XML: &str = "http://www.w3.org/XML/1998/namespace";

pub(super) struct ParsedXml {
    pub(super) tree: XmlTree,
    spans: HashMap<NodeId, TextSpan>,
    /// Only namespace declarations are indexed. Lookup never rescans ordinary
    /// attributes and still respects declarations on the nearest ancestor.
    namespaces: HashMap<NodeId, HashMap<String, String>>,
}

struct TextSpan {
    range: Range<usize>,
    cdata: bool,
}

impl ParsedXml {
    pub(super) fn parse(bytes: &[u8], limits: Limits) -> Result<Self> {
        let ranges = preflight(bytes, limits)?;
        let tree = XmlTree::parse(bytes).map_err(|err| document(format!("invalid XML: {err}")))?;
        let namespaces = namespace_index(&tree);
        let mut parsed = Self {
            tree,
            spans: HashMap::new(),
            namespaces,
        };
        let tree = &parsed.tree;
        let mut ranges = ranges.into_iter();
        for node in tree.descendants(tree.root()) {
            if let Some(text) = tree.text(node) {
                validate_characters(text)?;
                let span = ranges
                    .next()
                    .ok_or_else(|| document("XML text layout could not be retained"))?;
                if span.cdata != matches!(tree.kind(node), NodeKind::CData(_)) {
                    return Err(document("XML text layout could not be retained"));
                }
                // The lexical token map and docxml must agree before any edits.
                if decode_span(bytes, &span)? != text {
                    return Err(document("XML text decoding could not be retained"));
                }
                parsed.spans.insert(node, span);
            }
            if tree.name(node).is_some() {
                validate_namespaces(&parsed, node)?;
            }
        }
        if ranges.next().is_some() {
            return Err(document("XML text layout could not be retained"));
        }
        Ok(parsed)
    }

    pub(super) fn namespace_uri(&self, node: NodeId, prefix: Option<&str>) -> Option<&str> {
        let prefix = prefix.unwrap_or("");
        let mut ancestor = Some(node);
        while let Some(node) = ancestor {
            if let Some(uri) = self
                .namespaces
                .get(&node)
                .and_then(|scope| scope.get(prefix))
            {
                return Some(uri);
            }
            ancestor = self.tree.parent(node);
        }
        None
    }

    pub(super) fn patched(
        &self,
        original: &[u8],
        replacements: HashMap<NodeId, String>,
        limit: usize,
    ) -> Result<Option<Vec<u8>>> {
        let mut patches = Vec::new();
        let mut size = original.len();
        for (node, replacement) in replacements {
            if self.tree.text(node) == Some(replacement.as_str()) {
                continue;
            }
            let span = self
                .spans
                .get(&node)
                .ok_or_else(|| document("missing original XML text location"))?;
            let raw = preserve_base_spelling(original, span, &replacement)?;
            size = size
                .checked_sub(span.range.len())
                .and_then(|size| size.checked_add(raw.len()))
                .ok_or_else(|| document("edited XML size overflow"))?;
            patches.push((span.range.clone(), raw));
        }
        if patches.is_empty() {
            return Ok(None);
        }
        if size > limit {
            return Err(document("edited XML exceeds the XML size limit"));
        }
        patches.sort_unstable_by_key(|(range, _)| range.start);
        let mut result = Vec::with_capacity(size);
        let mut position = 0;
        for (range, raw) in patches {
            result.extend_from_slice(&original[position..range.start]);
            result.extend_from_slice(raw.as_bytes());
            position = range.end;
        }
        result.extend_from_slice(&original[position..]);
        Ok(Some(result))
    }
}

fn preflight(bytes: &[u8], limits: Limits) -> Result<Vec<TextSpan>> {
    if bytes.len() > limits.max_xml_bytes {
        return Err(document("XML part exceeds the XML size limit"));
    }
    let utf8 = std::str::from_utf8(bytes)
        .map_err(|_| document("only UTF-8 DOCX XML parts are supported"))?;
    validate_characters(utf8)?;
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().check_comments = true;
    reader.config_mut().check_end_names = true;
    let mut depth = 0usize;
    let mut roots = 0usize;
    let mut events = 0usize;
    let mut declaration = false;
    let mut spans: Vec<TextSpan> = Vec::new();
    let mut pending: Option<Range<usize>> = None;
    loop {
        let start = usize::try_from(reader.buffer_position())
            .map_err(|_| document("XML position overflow"))?;
        let event = reader
            .read_event()
            .map_err(|err| document(format!("invalid XML: {err}")))?;
        let end = usize::try_from(reader.buffer_position())
            .map_err(|_| document("XML position overflow"))?;
        events += 1;
        if events > limits.max_xml_events {
            return Err(document("XML exceeds the event count limit"));
        }
        if !matches!(event, Event::Text(_) | Event::GeneralRef(_))
            && let Some(range) = pending.take()
        {
            spans.push(TextSpan {
                range,
                cdata: false,
            });
        }
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                check_attribute_count(element, limits.max_xml_attributes_per_element)?;
                if depth == 0 {
                    roots += 1;
                    if roots > 1 {
                        return Err(document("XML has multiple root elements"));
                    }
                }
                if depth >= limits.max_xml_depth {
                    return Err(document("XML exceeds the nesting depth limit"));
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| document("XML has an unmatched closing tag"))?;
            }
            Event::Text(_) | Event::GeneralRef(_) => {
                if depth == 0 {
                    if bytes[start..end]
                        .iter()
                        .any(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
                    {
                        return Err(document("XML has text outside its root element"));
                    }
                } else if let Some(range) = &mut pending {
                    range.end = end;
                } else if start != end {
                    pending = Some(start..end);
                }
            }
            Event::CData(_) => {
                if depth == 0 {
                    return Err(document("XML has CDATA outside its root element"));
                }
                spans.push(TextSpan {
                    range: start + 9..end - 3,
                    cdata: true,
                });
            }
            Event::Decl(decl) => {
                if declaration || roots != 0 || (start != 0 && start != 3) {
                    return Err(document("XML declaration is misplaced"));
                }
                declaration = true;
                validate_declaration(&decl)?;
            }
            Event::DocType(_) => {
                return Err(document("DTD declarations are forbidden in DOCX XML"));
            }
            Event::Eof => break,
            Event::Comment(_) | Event::PI(_) => {}
        }
    }
    if depth != 0 || roots != 1 {
        return Err(document("XML has an unclosed or missing root element"));
    }
    Ok(spans)
}

fn check_attribute_count(element: &quick_xml::events::BytesStart<'_>, limit: usize) -> Result<()> {
    // Count with linear scanning and stop at the first excess attribute before
    // docxml builds its tree or performs duplicate-attribute checks.
    for (index, attribute) in element.attributes().with_checks(false).enumerate() {
        if index >= limit {
            return Err(document("XML element exceeds the attribute count limit"));
        }
        attribute.map_err(|err| document(format!("invalid XML attribute: {err}")))?;
    }
    Ok(())
}

fn validate_declaration(decl: &quick_xml::events::BytesDecl<'_>) -> Result<()> {
    if decl
        .version()
        .map_err(|err| document(err.to_string()))?
        .as_ref()
        != b"1.0"
    {
        return Err(document("only XML version 1.0 is supported"));
    }
    if let Some(encoding) = decl.encoding() {
        let encoding = encoding.map_err(|err| document(err.to_string()))?;
        if !encoding.eq_ignore_ascii_case(b"utf-8") {
            return Err(document("only UTF-8 DOCX XML parts are supported"));
        }
    }
    Ok(())
}

fn decode_span(bytes: &[u8], span: &TextSpan) -> Result<String> {
    let raw = std::str::from_utf8(&bytes[span.range.clone()])
        .map_err(|_| document("invalid UTF-8 XML text"))?;
    if span.cdata {
        Ok(raw.to_owned())
    } else {
        quick_xml::escape::unescape(raw)
            .map(std::borrow::Cow::into_owned)
            .map_err(|err| document(format!("invalid XML entity: {err}")))
    }
}

/// Keep each non-mark scalar's original spelling, including entity references
/// and literal CR/LF. Re-escaping whole text nodes would change CR semantics.
fn preserve_base_spelling(bytes: &[u8], span: &TextSpan, text: &str) -> Result<String> {
    let mut raw = std::str::from_utf8(&bytes[span.range.clone()])
        .map_err(|_| document("invalid UTF-8 XML text"))?;
    let mut result = String::new();
    for ch in text.chars() {
        if is_mark(ch) {
            result.push(ch);
            continue;
        }
        loop {
            let (old, spelling) = next_atom(raw, span.cdata)?
                .ok_or_else(|| document("replacement changed XML base characters"))?;
            raw = &raw[spelling.len()..];
            if !is_mark(old) {
                if old != ch {
                    return Err(document("replacement changed XML base characters"));
                }
                result.push_str(spelling);
                break;
            }
        }
    }
    while let Some((ch, spelling)) = next_atom(raw, span.cdata)? {
        if !is_mark(ch) {
            return Err(document("replacement removed XML base characters"));
        }
        raw = &raw[spelling.len()..];
    }
    // Removing a mark between `]]` and `>` can terminate CDATA or create an
    // illegal ordinary text sequence. Refuse rather than change base spelling
    // or split the original CDATA structure to repair the result.
    if result.contains("]]>") {
        return Err(document(
            "diacritic replacement would create the forbidden XML sequence ]]>",
        ));
    }
    Ok(result)
}

fn next_atom(raw: &str, cdata: bool) -> Result<Option<(char, &str)>> {
    let Some(ch) = raw.chars().next() else {
        return Ok(None);
    };
    if ch != '&' || cdata {
        return Ok(Some((ch, &raw[..ch.len_utf8()])));
    }
    let end = raw
        .find(';')
        .ok_or_else(|| document("unterminated XML entity"))?
        + 1;
    let spelling = &raw[..end];
    let decoded = quick_xml::escape::unescape(spelling)
        .map_err(|err| document(format!("invalid XML entity: {err}")))?;
    let mut chars = decoded.chars();
    let ch = chars.next().ok_or_else(|| document("empty XML entity"))?;
    if chars.next().is_some() {
        return Err(document("unsupported XML entity"));
    }
    Ok(Some((ch, spelling)))
}

fn validate_characters(text: &str) -> Result<()> {
    if text.chars().any(|ch| {
        !matches!(ch, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
    }) {
        return Err(document("XML contains a forbidden character"));
    }
    Ok(())
}

fn namespace_index(tree: &XmlTree) -> HashMap<NodeId, HashMap<String, String>> {
    let mut namespaces = HashMap::new();
    for node in tree.descendants(tree.root()) {
        let declarations: HashMap<_, _> = tree
            .attrs(node)
            .iter()
            .filter_map(|(name, uri)| {
                let prefix = if name == "xmlns" {
                    ""
                } else {
                    name.strip_prefix("xmlns:")?
                };
                Some((prefix.to_owned(), uri.clone()))
            })
            .collect();
        if !declarations.is_empty() {
            namespaces.insert(node, declarations);
        }
    }
    namespaces
}

fn validate_namespaces(xml: &ParsedXml, node: NodeId) -> Result<()> {
    let tree = &xml.tree;
    let Some(name) = tree.name(node) else {
        return Ok(());
    };
    resolve_name(xml, node, name, true)?;
    let mut names = HashSet::new();
    for (name, value) in tree.attrs(node) {
        validate_characters(value)?;
        if name == "xmlns" || name.starts_with("xmlns:") {
            continue;
        }
        let expanded = resolve_name(xml, node, name, false)?;
        if !names.insert(expanded) {
            return Err(document("XML has duplicate expanded attribute names"));
        }
    }
    Ok(())
}

fn resolve_name<'a>(
    xml: &'a ParsedXml,
    node: NodeId,
    name: &'a str,
    default: bool,
) -> Result<(&'a str, &'a str)> {
    let (prefix, local) = split_name(name);
    if local.is_empty() || local.contains(':') || prefix == Some("") {
        return Err(document("invalid XML qualified name"));
    }
    let namespace = match prefix {
        Some("xml") => XML,
        Some(prefix) => xml
            .namespace_uri(node, Some(prefix))
            .filter(|uri| !uri.is_empty())
            .ok_or_else(|| document(format!("undeclared XML namespace prefix: {prefix}")))?,
        None if default => xml.namespace_uri(node, None).unwrap_or(""),
        None => "",
    };
    Ok((namespace, local))
}

fn split_name(name: &str) -> (Option<&str>, &str) {
    name.split_once(':')
        .map_or((None, name), |(prefix, local)| (Some(prefix), local))
}

pub(super) fn expanded_name(xml: &ParsedXml, node: NodeId) -> Option<(&str, &str)> {
    let name = xml.tree.name(node)?;
    resolve_name(xml, node, name, true).ok()
}

pub(super) fn is_element(xml: &ParsedXml, node: NodeId, namespace: &str, local: &str) -> bool {
    expanded_name(xml, node) == Some((namespace, local))
}

pub(super) fn is_word(xml: &ParsedXml, node: NodeId, local: &str) -> bool {
    expanded_name(xml, node)
        .is_some_and(|(namespace, name)| WORD.contains(&namespace) && name == local)
}

pub(super) fn word_attr<'a>(xml: &'a ParsedXml, node: NodeId, local: &str) -> Option<&'a str> {
    xml.tree.attrs(node).iter().find_map(|(name, value)| {
        if split_name(name).1 != local {
            return None;
        }
        resolve_name(xml, node, name, false)
            .ok()
            .filter(|(namespace, name)| WORD.contains(namespace) && *name == local)
            .map(|_| value.as_str())
    })
}
