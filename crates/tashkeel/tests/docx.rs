//! Observable DOCX preservation, current-view, and untrusted-input tests.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Cursor, Read, Write};

use tashkeel::docx::{Limits, transform, transform_with_limits};
use tashkeel::{Error, Result};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const WORD: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const STRICT: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
const PACKAGE_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const MAIN_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const DOCUMENT: &str = include_str!("fixtures/docx/document.xml");

fn package_xml(body: &str) -> String {
    format!("<w:document xmlns:w=\"{WORD}\"><w:body>{body}</w:body></w:document>")
}

fn parts(document: &str) -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        ("[Content_Types].xml".into(), format!("<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"{MAIN_TYPE}\"/></Types>").into_bytes()),
        ("_rels/.rels".into(), format!("<Relationships xmlns=\"{PACKAGE_REL}\"><Relationship Id=\"main\" Type=\"{REL}officeDocument\" Target=\"word/document.xml\"/></Relationships>").into_bytes()),
        ("word/document.xml".into(), document.as_bytes().to_vec()),
    ])
}

fn pack(parts: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.set_comment("preserve this archive comment").unwrap();
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in parts {
        writer.start_file(name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn unpack(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut result = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let name = entry.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        result.insert(name, bytes);
    }
    result
}

// Keep the real callback signature so the helper can be passed directly.
#[allow(clippy::unnecessary_wraps)]
fn add_fatha(text: &str) -> Result<String> {
    Ok(text
        .chars()
        .filter(|ch| !matches!(ch, '\u{064b}'..='\u{0652}'))
        .flat_map(|ch| {
            let mark = matches!(ch, '\u{0621}'..='\u{063a}' | '\u{0641}'..='\u{064a}')
                && !matches!(ch, 'ا' | 'و' | 'ي');
            std::iter::once(ch).chain(mark.then_some('\u{064e}'))
        })
        .collect())
}

fn rich_parts() -> BTreeMap<String, Vec<u8>> {
    let mut result = parts(DOCUMENT);
    let mut rels = format!("<Relationships xmlns=\"{PACKAGE_REL}\">");
    let mut types = String::from_utf8(result["[Content_Types].xml"].clone()).unwrap();
    for (name, kind, xml) in [
        (
            "header.xml",
            "header",
            include_str!("fixtures/docx/header.xml"),
        ),
        (
            "footer.xml",
            "footer",
            include_str!("fixtures/docx/footer.xml"),
        ),
        (
            "footnotes.xml",
            "footnotes",
            include_str!("fixtures/docx/footnotes.xml"),
        ),
        (
            "endnotes.xml",
            "endnotes",
            include_str!("fixtures/docx/endnotes.xml"),
        ),
    ] {
        result.insert(format!("word/{name}"), xml.as_bytes().to_vec());
        write!(
            rels,
            "<Relationship Id=\"{kind}\" Type=\"{REL}{kind}\" Target=\"{name}\"/>"
        )
        .unwrap();
        types = types.replace("</Types>", &format!("<Override PartName=\"/word/{name}\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml\"/></Types>"));
    }
    write!(rels, "<Relationship Id=\"link\" Type=\"{REL}hyperlink\" Target=\"https://example.com/العربية\" TargetMode=\"External\"/><Relationship Id=\"image\" Type=\"{REL}image\" Target=\"media/pixel.png\"/></Relationships>").unwrap();
    result.insert("[Content_Types].xml".into(), types.into_bytes());
    result.insert("word/_rels/document.xml.rels".into(), rels.into_bytes());
    result.insert(
        "word/styles.xml".into(),
        include_bytes!("fixtures/docx/styles.xml").to_vec(),
    );
    // A complete 1x1 PNG. Its bytes must not be decoded or regenerated.
    result.insert(
        "word/media/pixel.png".into(),
        vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31,
            0, 3, 3, 2, 0, 239, 162, 167, 91, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ],
    );
    result.insert(
        "customXml/item1.xml".into(),
        b"<?xml version='1.0'?><opaque>  preserve &amp; unknown  </opaque>".to_vec(),
    );
    result.insert(
        "word/comments.xml".into(),
        "<comments>تعليق</comments>".as_bytes().to_vec(),
    );
    result
}

#[test]
fn preserves_styled_package_and_only_changes_selected_text_payloads() {
    let original = rich_parts();
    let input = pack(&original);
    let mut contexts = Vec::new();
    let output = transform(&input, |text| {
        contexts.push(text.to_owned());
        add_fatha(text)
    })
    .unwrap();
    let result = unpack(&output);
    assert_eq!(
        result.keys().collect::<Vec<_>>(),
        original.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        contexts,
        [
            "كُتب عربي & English 42!\tباب\nبيت\u{fffc}",
            "جدول",
            "قبل جديد نقل",
            "نتيجة",
            "رأس",
            "ذيل",
            "\u{fffc}حاشية",
            "\u{fffc}نهاية",
        ]
    );
    let mut expected_document = DOCUMENT.to_owned();
    // The mark at the start of the second run belongs to the first run's ك.
    expected_document = expected_document
        .replace("<w:t>ك</w:t>", "<w:t>كَ</w:t>")
        .replace("<w:t>ُتب</w:t>", "<w:t>تَبَ</w:t>");
    for text in [
        " عربي &amp; English 42!",
        "باب",
        "بيت",
        "جدول",
        "قبل ",
        "جديد ",
        "نقل",
        "نتيجة",
    ] {
        expected_document = expected_document.replace(
            &format!(">{text}</w:t>"),
            &format!(">{}</w:t>", add_fatha(text).unwrap()),
        );
    }
    assert_eq!(result["word/document.xml"], expected_document.as_bytes());
    for (name, bytes) in &original {
        match name.as_str() {
            "word/document.xml" => {}
            "word/header.xml" | "word/footer.xml" | "word/footnotes.xml" | "word/endnotes.xml" => {
                let source = std::str::from_utf8(bytes).unwrap();
                let word = match name.as_str() {
                    "word/header.xml" => "رأس",
                    "word/footer.xml" => "ذيل",
                    "word/footnotes.xml" => "حاشية",
                    _ => "نهاية",
                };
                assert_eq!(
                    result[name],
                    source.replace(word, &add_fatha(word).unwrap()).as_bytes()
                );
            }
            _ => assert_eq!(&result[name], bytes, "untouched part {name}"),
        }
    }
    let archive = ZipArchive::new(Cursor::new(output)).unwrap();
    assert_eq!(archive.comment(), b"preserve this archive comment");
}

#[test]
fn noop_is_an_exact_archive_roundtrip() {
    let original = pack(&rich_parts());
    assert_eq!(
        transform(&original, |text| Ok(text.into())).unwrap(),
        original
    );
}

#[test]
fn combines_word_runs_and_replaces_all_eight_marks_at_the_base_run() {
    let xml = package_xml(
        "<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>ك</w:t></w:r><w:r><w:t>ًٌٍَُِّْ</w:t><w:t>تب</w:t></w:r></w:p>",
    );
    let input = pack(&parts(&xml));
    let output = transform(&input, |text| {
        assert_eq!(text, "كًٌٍَُِّْتب");
        // Intentionally non-NFC shadda/fatha order must survive unchanged.
        Ok("كَت\u{651}\u{64e}بَ".into())
    })
    .unwrap();
    let expected = xml
        .replace("<w:t>ك</w:t>", "<w:t>كَ</w:t>")
        .replace("<w:t>ًٌٍَُِّْ</w:t>", "<w:t></w:t>")
        .replace("<w:t>تب</w:t>", "<w:t>ت\u{651}\u{64e}بَ</w:t>");
    assert_eq!(unpack(&output)["word/document.xml"], expected.as_bytes());
}

#[test]
fn preserves_entities_cdata_comments_and_line_endings_inside_text() {
    let xml = package_xml(
        "<w:p><w:r><w:t xml:space='preserve'>&#x643;<!--keep--><![CDATA[تب]]><?keep data?> &amp; &lt; &#13;\r\nA</w:t></w:r></w:p>",
    );
    let input = pack(&parts(&xml));
    let output = transform(&input, add_fatha).unwrap();
    assert_eq!(
        unpack(&output)["word/document.xml"],
        xml.replace("&#x643;", "&#x643;َ")
            .replace("<![CDATA[تب]]>", "<![CDATA[تَبَ]]>")
            .as_bytes()
    );
}

#[test]
fn rejects_mark_removal_that_would_terminate_cdata() {
    let xml = package_xml("<w:p><w:r><w:t><![CDATA[ب]]َ>tail]]></w:t></w:r></w:p>");
    let input = pack(&parts(&xml));
    assert_eq!(transform(&input, |text| Ok(text.into())).unwrap(), input);
    let result = transform(&input, |text| {
        assert_eq!(text, "ب]]َ>tail");
        Ok("بِ]]>tail".into())
    });
    assert!(matches!(result, Err(Error::Document(message)) if message.contains("]]>")));
}

#[test]
fn rejects_mark_removal_that_would_create_a_forbidden_text_sequence() {
    let xml = package_xml("<w:p><w:r><w:t>ب]]َ>tail</w:t></w:r></w:p>");
    let input = pack(&parts(&xml));
    assert_eq!(transform(&input, |text| Ok(text.into())).unwrap(), input);
    let result = transform(&input, |text| {
        assert_eq!(text, "ب]]َ>tail");
        Ok("بِ]]>tail".into())
    });
    assert!(matches!(result, Err(Error::Document(message)) if message.contains("]]>")));

    // The same semantic characters are safe when the original spelling already
    // escapes the closing angle bracket; keep that spelling rather than refuse.
    let escaped = xml.replace("]]َ>", "]]َ&gt;");
    let output = transform(&pack(&parts(&escaped)), |_| Ok("بِ]]>tail".into())).unwrap();
    assert_eq!(
        unpack(&output)["word/document.xml"],
        escaped.replace("ب]]َ&gt;", "بِ]]&gt;").as_bytes()
    );
}

#[test]
fn refuses_excessive_attributes_before_transforming_text() {
    let mut xml = format!("<w:document xmlns:w='{WORD}'");
    for index in 0..30_000 {
        write!(xml, " x:a{index}='x'").unwrap();
    }
    xml.push_str(
        " xmlns:x='urn:x'><w:body><w:p><w:r><w:t>كتب</w:t></w:r></w:p></w:body></w:document>",
    );
    let mut called = false;
    let result = transform(&pack(&parts(&xml)), |text| {
        called = true;
        Ok(text.into())
    });
    assert!(
        matches!(result, Err(Error::Document(message)) if message.contains("attribute count limit"))
    );
    assert!(!called);
}

#[test]
fn attribute_limit_counts_namespaces_on_start_and_empty_elements() {
    let limits = Limits {
        max_xml_attributes_per_element: 4,
        ..Limits::default()
    };
    for body in [
        "<w:p><w:r ATTRS><w:t>كتب</w:t></w:r></w:p>",
        "<w:p><w:r><w:tab ATTRS/><w:t>كتب</w:t></w:r></w:p>",
    ] {
        let attributes = "x:a='x' x:b='x' x:c='x' xmlns:x='urn:x'";
        let xml = package_xml(&body.replace("ATTRS", attributes));
        let input = pack(&parts(&xml));
        let output = transform_with_limits(&input, limits, add_fatha).unwrap();
        assert_eq!(
            unpack(&output)["word/document.xml"],
            xml.replace("كتب", "كَتَبَ").as_bytes()
        );

        let extra = package_xml(&body.replace("ATTRS", &format!("x:d='x' {attributes}")));
        let input = pack(&parts(&extra));
        let result = transform_with_limits(&input, limits, add_fatha);
        assert!(
            matches!(result, Err(Error::Document(message)) if message.contains("attribute count limit"))
        );
        let raised = Limits {
            max_xml_attributes_per_element: 5,
            ..limits
        };
        assert!(transform_with_limits(&input, raised, add_fatha).is_ok());
    }
    for max_xml_attributes_per_element in [0, 257] {
        let limits = Limits {
            max_xml_attributes_per_element,
            ..limits
        };
        let result = transform_with_limits(&pack(&parts(&package_xml(""))), limits, add_fatha);
        assert!(matches!(result, Err(Error::InvalidInput(_))));
    }
}

#[test]
fn indexed_namespaces_respect_scoped_field_attribute_prefixes() {
    let xml = package_xml(&format!(
        "<w:p xmlns:f='{WORD}' xmlns:other='urn:other'><w:r><f:fldChar other:fldCharType='unrelated' f:fldCharType='begin'/><w:instrText>حقل</w:instrText><w:fldChar w:fldCharType='separate'/><w:t>كتب</w:t><w:fldChar w:fldCharType='end'/></w:r><f:r xmlns:f='urn:other'><f:t>كتب</f:t></f:r></w:p>"
    ));
    let output = transform(&pack(&parts(&xml)), |text| {
        assert_eq!(text, "كتب");
        add_fatha(text)
    })
    .unwrap();
    assert_eq!(
        unpack(&output)["word/document.xml"],
        xml.replacen("كتب", "كَتَبَ", 1).as_bytes()
    );
}

#[test]
fn strict_namespaces_arbitrary_prefixes_and_default_namespaces_are_supported() {
    for xml in [
        format!(
            "<z:document xmlns:z='{STRICT}'><z:body><z:p><z:r><z:t>كتب</z:t></z:r></z:p></z:body></z:document>"
        ),
        format!("<document xmlns='{STRICT}'><body><p><r><t>كتب</t></r></p></body></document>"),
        format!(
            "<w:document xmlns:w='{WORD}'><w:body><p xmlns='{WORD}'><r><t>كتب</t></r></p><foreign xmlns:w='urn:other'><w:t>كتب</w:t></foreign></w:body></w:document>"
        ),
    ] {
        let mut original = parts(&xml);
        let rels = String::from_utf8(original["_rels/.rels"].clone())
            .unwrap()
            .replace(
                REL,
                "http://purl.oclc.org/ooxml/officeDocument/relationships/",
            );
        original.insert("_rels/.rels".into(), rels.into_bytes());
        let output = transform(&pack(&original), add_fatha).unwrap();
        assert_eq!(
            unpack(&output)["word/document.xml"],
            xml.replacen("كتب", "كَتَبَ", 1).as_bytes()
        );
    }
}

#[test]
fn follows_relationships_instead_of_assuming_word_document_location() {
    let xml = package_xml("<w:p><w:r><w:t>كتب</w:t></w:r></w:p>");
    let mut original = parts(&xml);
    let document = original.remove("word/document.xml").unwrap();
    original.insert("documents/main.xml".into(), document);
    for name in ["[Content_Types].xml", "_rels/.rels"] {
        let updated = String::from_utf8(original[name].clone())
            .unwrap()
            .replace("word/document.xml", "documents/main.xml");
        original.insert(name.into(), updated.into_bytes());
    }
    let output = transform(&pack(&original), add_fatha).unwrap();
    assert_eq!(
        unpack(&output)["documents/main.xml"],
        xml.replace("كتب", "كَتَبَ").as_bytes()
    );
}

#[test]
fn alternate_content_is_visited_once_and_inactive_branches_remain_exact() {
    let xml = package_xml(
        "<w:p xmlns:mc='http://schemas.openxmlformats.org/markup-compatibility/2006' xmlns:wps='http://schemas.microsoft.com/office/word/2010/wordprocessingShape' xmlns:u='urn:unknown'><mc:AlternateContent><mc:Choice Requires='wps'><w:r><w:t>كتب</w:t></w:r></mc:Choice><mc:Fallback><w:r><w:t>بديل</w:t></w:r></mc:Fallback></mc:AlternateContent><mc:AlternateContent><mc:Choice Requires='u'><w:r><w:t>قديم</w:t></w:r></mc:Choice><mc:Fallback><w:r><w:t>باب</w:t></w:r></mc:Fallback></mc:AlternateContent></w:p>",
    );
    let output = transform(&pack(&parts(&xml)), |text| {
        assert_eq!(text, "كتبباب");
        add_fatha(text)
    })
    .unwrap();
    assert_eq!(
        unpack(&output)["word/document.xml"],
        xml.replace("كتب", "كَتَبَ").replace("باب", "بَابَ").as_bytes()
    );
}

#[test]
fn textbox_text_is_an_independent_story_and_deleted_rows_are_unchanged() {
    let xml = package_xml(
        "<w:p xmlns:v='urn:schemas-microsoft-com:vml'><w:r><w:t>قبل</w:t><w:pict><v:shape><v:textbox><w:txbxContent><w:p><w:r><w:t>صندوق</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict><w:t>بعد</w:t></w:r></w:p><w:tbl><w:tr><w:trPr><w:del/></w:trPr><w:tc><w:p><w:r><w:t>محذوف</w:t></w:r></w:p></w:tc></w:tr></w:tbl>",
    );
    let mut calls = Vec::new();
    let output = transform(&pack(&parts(&xml)), |text| {
        calls.push(text.to_owned());
        add_fatha(text)
    })
    .unwrap();
    assert_eq!(calls, ["صندوق", "قبل\u{fffc}بعد"]);
    let expected = xml
        .replace("قبل", "قَبَلَ")
        .replace("بعد", "بَعَدَ")
        .replace("صندوق", "صَنَدَوقَ");
    assert_eq!(unpack(&output)["word/document.xml"], expected.as_bytes());
}

#[test]
fn nested_fields_and_fields_spanning_paragraphs_use_the_current_result_only() {
    let xml = package_xml(
        "<w:p><w:r><w:fldChar w:fldCharType='begin'/><w:instrText>حقل</w:instrText><w:t>خفي</w:t></w:r></w:p><w:p><w:r><w:fldChar w:fldCharType='separate'/><w:t>قبل</w:t><w:fldChar w:fldCharType='begin'/><w:instrText>حقل</w:instrText><w:fldChar w:fldCharType='separate'/><w:t>نتيجة</w:t><w:fldChar w:fldCharType='end'/><w:t>بعد</w:t><w:fldChar w:fldCharType='end'/></w:r></w:p>",
    );
    let mut calls = Vec::new();
    let output = transform(&pack(&parts(&xml)), |text| {
        calls.push(text.to_owned());
        add_fatha(text)
    })
    .unwrap();
    assert_eq!(calls, ["قبلنتيجةبعد"]);
    let expected = xml
        .replace("قبل", "قَبَلَ")
        .replace("نتيجة", "نَتَيجَةَ")
        .replace("بعد", "بَعَدَ");
    assert_eq!(unpack(&output)["word/document.xml"], expected.as_bytes());
}

#[test]
fn rejects_non_mark_mutations_and_new_marks_without_real_text_anchors() {
    let xml = package_xml("<w:p><w:r><w:t>أب C!\u{0653}</w:t><w:tab/></w:r></w:p>");
    let input = pack(&parts(&xml));
    for replacement in [
        "اب C!\u{0653}\t",
        "أبC!\u{0653}\t",
        "أب C?\u{0653}\t",
        "أب C!\t",
        "أب C!\u{0653}\tَ",
        "َأب C!\u{0653}\t",
    ] {
        assert!(
            matches!(
                transform(&input, |_| Ok(replacement.into())),
                Err(Error::Document(_))
            ),
            "{replacement:?}"
        );
    }
}

#[test]
fn callback_errors_and_cancellation_propagate_without_partial_output() {
    let xml =
        package_xml("<w:p><w:r><w:t>كتب</w:t></w:r></w:p><w:p><w:r><w:t>باب</w:t></w:r></w:p>");
    let input = pack(&parts(&xml));
    let mut count = 0;
    let result = transform(&input, |text| {
        count += 1;
        if count == 2 {
            Err(Error::Cancelled)
        } else {
            add_fatha(text)
        }
    });
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(count, 2);
    assert!(
        matches!(transform(&input, |_| Err(Error::Inference("fixture failure".into()))), Err(Error::Inference(message)) if message == "fixture failure")
    );
}

#[test]
fn refuses_structures_whose_current_text_cannot_be_preserved() {
    for body in [
        "<w:altChunk/>",
        "<w:p><w:r><w:ruby><w:rubyBase><w:r><w:t>كتب</w:t></w:r></w:rubyBase></w:ruby></w:r></w:p>",
        "<w:sdt><w:sdtPr><w:dataBinding/></w:sdtPr><w:sdtContent><w:p><w:r><w:t>كتب</w:t></w:r></w:p></w:sdtContent></w:sdt>",
        "<w:p><w:pPr><w:rPr><w:del/></w:rPr></w:pPr><w:r><w:t>كتب</w:t></w:r></w:p>",
        "<w:p><w:r><w:fldChar w:fldCharType='end'/></w:r></w:p>",
        "<w:p><w:r><w:fldChar w:fldCharType='begin'/></w:r></w:p>",
        "<w:p xmlns:mc='http://schemas.openxmlformats.org/markup-compatibility/2006' xmlns:u='urn:unknown'><mc:AlternateContent><mc:Choice Requires='u'><w:r><w:t>كتب</w:t></w:r></mc:Choice></mc:AlternateContent></w:p>",
    ] {
        assert!(
            matches!(
                transform(&pack(&parts(&package_xml(body))), add_fatha),
                Err(Error::Document(_))
            ),
            "{body}"
        );
    }
}

#[test]
fn rejects_malformed_xml_dtd_entities_and_unsupported_encodings() {
    for xml in [
        format!("<w:document xmlns:w='{WORD}'><w:body>"),
        format!(
            "<!DOCTYPE w:document [<!ENTITY e 'كتب'>]><w:document xmlns:w='{WORD}'><w:body/></w:document>"
        ),
        package_xml("<w:p><w:r><w:t>&missing;</w:t></w:r></w:p>"),
        package_xml("<w:p><w:r><w:t>&#0;</w:t></w:r></w:p>"),
        package_xml("<w:p><w:r><w:t x:a='1'>كتب</w:t></w:r></w:p>"),
        format!("<?xml version='1.0' encoding='UTF-16'?>{}", package_xml("")),
        format!("{}{}", package_xml(""), package_xml("")),
    ] {
        assert!(
            matches!(
                transform(&pack(&parts(&xml)), add_fatha),
                Err(Error::Document(_))
            ),
            "{xml}"
        );
    }
}

#[test]
fn enforces_configurable_input_inflation_xml_and_output_limits() {
    let xml = package_xml("<w:p><w:r><w:t>كتب</w:t></w:r></w:p>");
    let input = pack(&parts(&xml));
    for limits in [
        Limits {
            max_archive_bytes: input.len() - 1,
            ..Limits::default()
        },
        Limits {
            max_entries: 2,
            ..Limits::default()
        },
        Limits {
            max_part_bytes: 8,
            ..Limits::default()
        },
        Limits {
            max_total_uncompressed_bytes: 8,
            ..Limits::default()
        },
        Limits {
            max_xml_bytes: 8,
            ..Limits::default()
        },
        Limits {
            max_xml_depth: 3,
            ..Limits::default()
        },
        Limits {
            max_xml_events: 3,
            ..Limits::default()
        },
        Limits {
            max_paragraph_bytes: 5,
            ..Limits::default()
        },
        Limits {
            max_paragraph_bytes: 8,
            ..Limits::default()
        },
        Limits {
            max_output_bytes: 32,
            ..Limits::default()
        },
    ] {
        assert!(
            matches!(
                transform_with_limits(&input, limits, add_fatha),
                Err(Error::Document(_))
            ),
            "{limits:?}"
        );
    }
    assert!(matches!(
        transform_with_limits(
            &input,
            Limits {
                max_xml_depth: 257,
                ..Limits::default()
            },
            add_fatha
        ),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn refuses_zip_path_aliases_signatures_legacy_doc_and_malformed_sizes() {
    let xml = package_xml("<w:p><w:r><w:t>كتب</w:t></w:r></w:p>");
    for name in [
        "../outside",
        "/absolute",
        "word/../escape",
        "word\\document.xml",
        "WORD/document.xml",
        "word/%64ocument.xml",
        "_xmlsignatures/sig1.xml",
    ] {
        let mut original = parts(&xml);
        original.insert(name.into(), b"opaque".to_vec());
        assert!(
            matches!(
                transform(&pack(&original), add_fatha),
                Err(Error::Document(_))
            ),
            "{name}"
        );
    }
    let ole = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
    assert!(
        matches!(transform(&ole, add_fatha), Err(Error::Document(message)) if message.contains("legacy DOC"))
    );
    let mut input = pack(&parts(&xml));
    let central = input
        .windows(4)
        .position(|window| window == b"PK\x01\x02")
        .unwrap();
    input[central + 24..central + 28].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        transform(&input, add_fatha),
        Err(Error::Document(_))
    ));
}

#[test]
fn rejects_duplicate_zip_names_even_when_the_zip_reader_deduplicates_them() {
    let mut original = parts(&package_xml(""));
    original.insert("aaaa".into(), b"one".to_vec());
    original.insert("bbbb".into(), b"two".to_vec());
    let mut input = pack(&original);
    // Rewrite equal-length local and central names, preserving a valid ZIP
    // layout. Public ZipWriter correctly refuses creating duplicates directly.
    for index in 0..input.len().saturating_sub(3) {
        if &input[index..index + 4] == b"bbbb" {
            input[index..index + 4].copy_from_slice(b"aaaa");
        }
    }
    assert!(matches!(
        transform(&input, add_fatha),
        Err(Error::Document(_))
    ));
}

#[test]
fn bounds_inherited_namespace_values_before_expanded_name_hashing() {
    let namespace_document = |length: usize, body: &str| {
        let uri = format!("urn:{}", "x".repeat(length - 4));
        package_xml(body).replacen(
            "<w:document ",
            &format!("<w:document xmlns:x=\"{uri}\" "),
            1,
        )
    };
    let allowed = pack(&parts(&namespace_document(1024, "<u x:a='v'/>")));
    assert_eq!(
        transform(&allowed, |text| Ok(text.to_owned())).unwrap(),
        allowed
    );
    for (length, count) in [(1025, 1), (1_000_000, 50_000)] {
        // The large case is below XML event/byte limits but would repeatedly
        // hash the inherited megabyte URI without the declaration-size bound.
        let input = pack(&parts(&namespace_document(
            length,
            &"<u x:a='v'/>".repeat(count),
        )));
        assert!(matches!(
            transform(&input, |_| panic!("oversized namespace reached transformation")),
            Err(Error::Document(message)) if message.contains("namespace declaration")
        ));
    }
}
