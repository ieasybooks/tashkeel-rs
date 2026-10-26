//! Traverse the current revision view without flattening the retained XML tree.

use std::collections::HashMap;

use docxml::xml::{NodeId, NodeKind, XmlTree};

use super::xml::{self, MC, ParsedXml, WORD, expanded_name, is_element, is_word, word_attr};
use super::{Limits, document, is_mark};
use crate::Result;

struct Segment {
    node: Option<NodeId>,
    text: String,
}

#[derive(Default)]
struct Paragraph {
    segments: Vec<Segment>,
    bytes: usize,
}

struct Walker<'a, F> {
    xml: &'a ParsedXml,
    tree: &'a XmlTree,
    limits: Limits,
    callback: &'a mut F,
    paragraph: Option<Paragraph>,
    /// True for the displayed result, false for the field instruction phase.
    fields: Vec<bool>,
    replacements: HashMap<NodeId, String>,
}

pub(super) fn transform(
    xml: &ParsedXml,
    original: &[u8],
    limits: Limits,
    callback: &mut impl FnMut(&str) -> Result<String>,
) -> Result<Option<Vec<u8>>> {
    let mut walker = Walker {
        xml,
        tree: &xml.tree,
        limits,
        callback,
        paragraph: None,
        fields: Vec::new(),
        replacements: HashMap::new(),
    };
    walker.walk(xml.tree.root())?;
    walker.finish_fields()?;
    xml.patched(original, walker.replacements, limits.max_xml_bytes)
}

impl<F: FnMut(&str) -> Result<String>> Walker<'_, F> {
    fn walk(&mut self, node: NodeId) -> Result<()> {
        let Some((namespace, local)) = expanded_name(self.xml, node) else {
            return Ok(());
        };
        if namespace == MC && local == "AlternateContent" {
            let selected = self.alternate(node)?;
            return self.children(selected);
        }
        if !WORD.contains(&namespace) {
            if matches!(
                namespace,
                "http://schemas.openxmlformats.org/officeDocument/2006/math"
                    | "http://purl.oclc.org/ooxml/officeDocument/math"
            ) && matches!(local, "oMath" | "oMathPara")
            {
                return self.segment(None, "\u{fffc}".into());
            }
            return self.children(node);
        }
        match local {
            // Historical revisions and property snapshots are not current text.
            "del" | "moveFrom" | "delText" | "delInstrText" | "instrText" => Ok(()),
            name if name.ends_with("Pr") || name.ends_with("PrChange") => Ok(()),
            "tr" if self.deleted_property(node, "trPr", "del") => Ok(()),
            "tc" if self.deleted_property(node, "tcPr", "cellDel") => Ok(()),
            "p" => self.paragraph(node),
            "t" => self.text(node),
            "fldChar" => self.field(node),
            "tab" | "ptab" => self.segment(None, "\t".into()),
            "br" | "cr" => self.segment(None, "\n".into()),
            "noBreakHyphen" => self.segment(None, "\u{2011}".into()),
            "softHyphen" => self.segment(None, "\u{00ad}".into()),
            "sym" | "footnoteReference" | "endnoteReference" | "footnoteRef" | "endnoteRef" => {
                self.segment(None, "\u{fffc}".into())
            }
            "drawing" | "pict" | "object" => {
                self.segment(None, "\u{fffc}".into())?;
                self.text_boxes(node)
            }
            "txbxContent" | "footnote" | "endnote" => self.separate_story(node),
            "altChunk" => Err(document(
                "altChunk imported content must be converted to ordinary Word paragraphs before diacritization",
            )),
            "ruby" => Err(document(
                "ruby annotations are unsupported; their base and annotation text cannot share paragraph context",
            )),
            "sdt" => {
                if self.tree.children(node).iter().any(|&child| {
                    is_word(self.xml, child, "sdtPr")
                        && self.tree.descendants(child).any(|property| {
                            is_word(self.xml, property, "dataBinding")
                                || is_word(self.xml, property, "dropDownList")
                                || is_word(self.xml, property, "comboBox")
                                || is_word(self.xml, property, "date")
                        })
                }) {
                    return Err(document(
                        "data-bound, date, and list content controls are unsupported; displayed text may be regenerated",
                    ));
                }
                self.children(node)
            }
            _ => self.children(node),
        }
    }

    fn children(&mut self, node: NodeId) -> Result<()> {
        for &child in self.tree.children(node) {
            self.walk(child)?;
        }
        Ok(())
    }

    fn paragraph(&mut self, node: NodeId) -> Result<()> {
        // Accepting a deleted paragraph mark would merge paragraphs. Refuse
        // instead of silently giving the callback a different current view.
        if self.tree.children(node).iter().any(|&property| {
            is_word(self.xml, property, "pPr")
                && self.tree.children(property).iter().any(|&run| {
                    is_word(self.xml, run, "rPr")
                        && self.tree.children(run).iter().any(|&revision| {
                            is_word(self.xml, revision, "del")
                                || is_word(self.xml, revision, "moveFrom")
                        })
                })
        }) {
            return Err(document(
                "tracked deletion of a paragraph mark is unsupported; accept or reject that revision first",
            ));
        }
        if self.paragraph.is_some() {
            return Err(document(
                "nested paragraphs outside a text box are unsupported",
            ));
        }
        self.paragraph = Some(Paragraph::default());
        self.children(node)?;
        let paragraph = self
            .paragraph
            .take()
            .ok_or_else(|| document("missing paragraph context"))?;
        self.replace(paragraph)
    }

    fn text(&mut self, node: NodeId) -> Result<()> {
        if self.fields.contains(&false) {
            return Ok(());
        }
        if self.paragraph.is_none() {
            return Err(document("Word text outside a paragraph is unsupported"));
        }
        for &child in self.tree.children(node) {
            match self.tree.kind(child) {
                NodeKind::Text(text) | NodeKind::CData(text) => {
                    self.segment(Some(child), text.clone())?;
                }
                NodeKind::Comment(_) | NodeKind::Pi(_) => {}
                NodeKind::Element(_) => {
                    return Err(document("nested elements inside Word text are unsupported"));
                }
            }
        }
        Ok(())
    }

    fn segment(&mut self, node: Option<NodeId>, text: String) -> Result<()> {
        if self.fields.contains(&false) {
            return Ok(());
        }
        if let Some(paragraph) = &mut self.paragraph {
            paragraph.bytes = paragraph
                .bytes
                .checked_add(text.len())
                .filter(|&bytes| bytes <= self.limits.max_paragraph_bytes)
                .ok_or_else(|| document("paragraph exceeds the text size limit"))?;
            paragraph.segments.push(Segment { node, text });
        }
        Ok(())
    }

    fn field(&mut self, node: NodeId) -> Result<()> {
        match word_attr(self.xml, node, "fldCharType") {
            Some("begin") => {
                if self.fields.len() >= self.limits.max_xml_depth {
                    return Err(document("complex fields exceed the nesting limit"));
                }
                self.fields.push(false);
            }
            Some("separate") => {
                let phase = self
                    .fields
                    .last_mut()
                    .filter(|phase| !**phase)
                    .ok_or_else(|| document("unmatched or repeated complex field separator"))?;
                *phase = true;
            }
            Some("end") => {
                self.fields
                    .pop()
                    .ok_or_else(|| document("unmatched complex field end"))?;
            }
            _ => return Err(document("missing or unknown complex field type")),
        }
        Ok(())
    }

    fn finish_fields(&self) -> Result<()> {
        if self.fields.is_empty() {
            Ok(())
        } else {
            Err(document("unclosed complex field"))
        }
    }

    fn separate_story(&mut self, node: NodeId) -> Result<()> {
        let paragraph = self.paragraph.take();
        let fields = std::mem::take(&mut self.fields);
        self.children(node)?;
        self.finish_fields()?;
        self.fields = fields;
        self.paragraph = paragraph;
        Ok(())
    }

    fn text_boxes(&mut self, node: NodeId) -> Result<()> {
        for &child in self.tree.children(node) {
            if is_word(self.xml, child, "txbxContent") {
                self.separate_story(child)?;
            } else if is_element(self.xml, child, MC, "AlternateContent") {
                let selected = self.alternate(child)?;
                self.text_boxes(selected)?;
            } else {
                self.text_boxes(child)?;
            }
        }
        Ok(())
    }

    fn deleted_property(&self, node: NodeId, property: &str, revision: &str) -> bool {
        self.tree.children(node).iter().any(|&child| {
            is_word(self.xml, child, property)
                && self
                    .tree
                    .children(child)
                    .iter()
                    .any(|&item| is_word(self.xml, item, revision))
        })
    }

    fn alternate(&self, node: NodeId) -> Result<NodeId> {
        let mut selected = None;
        let mut fallback = None;
        for &child in self.tree.children(node) {
            if is_element(self.xml, child, MC, "Choice") {
                if fallback.is_some() {
                    return Err(document("AlternateContent Choice follows its Fallback"));
                }
                let requires = self
                    .tree
                    .attr(child, "Requires")
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| document("AlternateContent Choice lacks Requires"))?;
                let mut understood = true;
                for prefix in requires.split_whitespace() {
                    let uri = self.xml.namespace_uri(child, Some(prefix)).ok_or_else(|| {
                        document("AlternateContent Requires uses an undeclared namespace")
                    })?;
                    understood &= supported_choice(uri);
                }
                if selected.is_none() && understood {
                    selected = Some(child);
                }
            } else if is_element(self.xml, child, MC, "Fallback") {
                if fallback.replace(child).is_some() {
                    return Err(document("multiple AlternateContent fallbacks"));
                }
            } else if self.tree.name(child).is_some() {
                return Err(document("unexpected AlternateContent child"));
            }
        }
        selected
            .or(fallback)
            .ok_or_else(|| document("AlternateContent has no supported Choice or Fallback"))
    }

    fn replace(&mut self, paragraph: Paragraph) -> Result<()> {
        if !paragraph
            .segments
            .iter()
            .any(|segment| segment.node.is_some() && !segment.text.is_empty())
        {
            return Ok(());
        }
        let original: String = paragraph
            .segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect();
        let transformed = (self.callback)(&original)?;
        if transformed.len() > self.limits.max_paragraph_bytes {
            return Err(document(
                "transformed paragraph exceeds the text size limit",
            ));
        }
        if transformed == original {
            return Ok(());
        }
        let mut bases = paragraph
            .segments
            .iter()
            .enumerate()
            .flat_map(|(index, segment)| {
                segment
                    .text
                    .chars()
                    .filter(|&ch| !is_mark(ch))
                    .map(move |ch| (index, ch))
            });
        let mut outputs = vec![String::new(); paragraph.segments.len()];
        let mut owner: Option<usize> = None;
        for ch in transformed.chars() {
            if is_mark(ch) {
                let index = owner.ok_or_else(|| {
                    document("transformed text has a mark without a preceding base character")
                })?;
                if paragraph.segments[index].node.is_none() {
                    return Err(document(
                        "transformed text attaches a mark to a structural character",
                    ));
                }
                outputs[index].push(ch);
            } else {
                let (index, base) = bases.next().filter(|(_, base)| *base == ch).ok_or_else(
                    || {
                        document(
                            "transformation changed base characters, punctuation, or whitespace",
                        )
                    },
                )?;
                outputs[index].push(base);
                owner = Some(index);
            }
        }
        if bases.next().is_some() {
            return Err(document(
                "transformation removed base characters, punctuation, or whitespace",
            ));
        }
        for (segment, output) in paragraph.segments.into_iter().zip(outputs) {
            if let Some(node) = segment.node
                && segment.text != output
            {
                self.replacements.insert(node, output);
            }
        }
        Ok(())
    }
}

/// We can traverse Word text boxes in these drawing containers. Other
/// extensions use their fallback; inactive alternatives are never transformed.
fn supported_choice(namespace: &str) -> bool {
    xml::WORD.contains(&namespace)
        || matches!(
            namespace,
            "http://schemas.microsoft.com/office/word/2010/wordprocessingShape"
                | "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"
                | "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
                | "http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing"
                | "urn:schemas-microsoft-com:vml"
        )
}
