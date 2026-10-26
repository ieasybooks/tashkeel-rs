# DOCX preservation

`tashkeel::docx::transform` processes a DOCX in memory and returns a complete new package. It gives the callback paragraph text across run and hyperlink boundaries, then assigns the returned Arabic marks to the text nodes containing their original base characters. A word split between bold and italic runs keeps those runs and their formatting. An existing mark in the next run is removed from that run and its replacement follows the base letter in the preceding run.

```rust
pub fn transform(
    input: &[u8],
    transform: impl FnMut(&str) -> tashkeel::Result<String>,
) -> tashkeel::Result<Vec<u8>>;

pub fn transform_with_limits(
    input: &[u8],
    limits: tashkeel::docx::Limits,
    transform: impl FnMut(&str) -> tashkeel::Result<String>,
) -> tashkeel::Result<Vec<u8>>;
```

The callback may replace only U+064B through U+0652: fathatan, dammatan, kasratan, fatha, damma, kasra, shadda, and sukun. Every other Unicode scalar must remain in its original order, including punctuation, whitespace, tatweel, hamza, and other Arabic combining marks. There is no Unicode normalization. Changing or removing any base character returns `Error::Document`. Added marks require a preceding real text character; structural tabs, breaks, and object placeholders cannot own marks. Paragraphs without text do not invoke the callback. A paragraph returned unchanged retains its original mark locations too.

Callback errors propagate unchanged, including `Error::Cancelled`. Callers can check cancellation inside the callback before processing each paragraph. ZIP validation and XML parsing do not invoke the callback and do not provide an asynchronous interruption hook. The function returns no partial output on an error; callers remain responsible for writing the returned bytes atomically.

## What is retained

The implementation uses **docxml 0.3.1**, licensed **MIT OR Apache-2.0**, specifically its public `docxml::xml::XmlTree` retained tree. The version and licenses were verified against the published crate and crates.io index. The typed `Document`, paragraph/run setters, OPC package reader, and package writer are not used. ZIP input and output use the workspace's ZIP 9 dependency; uploaded bytes never pass through docxml's ZIP 7 dependency. `quick-xml` 0.41 validates bounded XML and records original text locations before docxml parses the tree.

Only selected `w:t` text payloads change. Original XML bytes outside those payloads remain identical, including namespace declarations, attribute order and quotes, comments, processing instructions, unknown elements, formatting, relationships, bookmarks, hyperlinks, table and section structure, tabs, breaks, and drawing markup. Even inside an edited text payload, each non-mark character keeps its original lexical spelling, including numeric entities and literal CR/LF. CDATA delimiters and interleaved comments are retained. If removing marks would create a literal `]]>` within ordinary text or a CDATA payload, the transform returns `Error::Document` instead of returning malformed XML, changing base-character spelling, or splitting CDATA. Existing escaped spelling such as `]]&gt;` remains supported. The implementation never replaces a paragraph or run with a flattened reconstruction.

Every package entry remains present and in its original archive order. Unchanged parts are copied with their original compressed payloads: fonts, styles, numbering, images, relationship files, settings, custom XML, comments, and other opaque parts are not regenerated. The archive comment is retained. ZIP headers and compression of changed parts may differ; a package with no text changes is returned byte for byte, including its original ZIP representation.

## Text traversal

Both Transitional and Strict WordprocessingML namespace URIs are recognized. Prefixes need not be `w`, and a default WordprocessingML namespace is supported. Main, header, footer, footnote, and endnote parts are located through OPC relationships and content types, including a main part outside `word/document.xml`. Package part references, relationship-part discovery, and content-type override names compare ASCII letters without case distinctions; original ZIP entry spelling and untouched bytes are retained on output. Case-equivalent ZIP entries and duplicate content-type overrides are refused. Tables and nested tables are visited in document order. Word text boxes containing `w:txbxContent` have their own paragraph contexts.

The callback receives `\t` for `w:tab` and `w:ptab`, `\n` for `w:br` and `w:cr`, U+2011 for `w:noBreakHyphen`, and U+00AD for `w:softHyphen`. Opaque drawings, embedded objects, symbols, note references, and equations appear as U+FFFC in surrounding paragraph context. These structural characters must survive unchanged and never become new `w:t` text. Text-box paragraphs are processed independently of the surrounding object placeholder.

The current revision view includes inserted and moved-to text and excludes `w:del`, `w:moveFrom`, deleted text, deleted table rows, and deleted cells. Historical property snapshots remain untouched. Complex field instructions, including text before a field separator, are excluded; displayed field results are processed. Nested fields and fields spanning paragraphs are supported. Simple-field instruction attributes are unchanged. Field results can later be regenerated by Word; tashkeel does not update field instructions or calculate fields.

For `mc:AlternateContent`, only the first supported `Choice`, or otherwise its `Fallback`, is processed. Supported `Requires` namespaces are the two WordprocessingML namespaces, the two WordprocessingDrawing namespaces, Microsoft Word 2010 WordprocessingShape and WordprocessingGroup, and VML. Inactive alternatives remain byte-identical and are not added to paragraph context. A document consumer choosing a different alternative may display that unchanged alternative. A structure with no supported choice and no fallback is refused. This is a declared subset of markup compatibility, not an implementation of every Word extension or rendering rule.

## Explicit boundaries

These inputs or structures return `Error::Document` rather than silently replacing or dropping content:

- Legacy binary `.doc`, encrypted Office files, macro-enabled documents, templates, and digitally signed DOCX packages.
- `w:altChunk` imports, ruby annotations, and data-bound, date, dropdown, or combo-box content controls. Their displayed text has another representation that this transform cannot safely synchronize. Plain rich-text and plain-text content controls are traversed.
- Tracked deletion or move-from of a paragraph mark, which would merge paragraph context; accept or reject that revision before transforming.
- Unbalanced complex fields, ambiguous or malformed alternate content, Word text outside a paragraph, nested paragraphs outside a text box, and element children inside `w:t`.
- Non-UTF-8 XML, XML versions other than 1.0, DTDs, unknown entities, forbidden XML characters, missing namespace prefixes, malformed XML, missing required package parts, conflicting story relationship types, or missing internal targets in traversed relationship files.
- A replacement that would create the forbidden literal `]]>` in a text payload after removing marks; use escaped ordinary text when that base sequence must remain representable without changing the original XML structure.
- ZIP64, multi-volume ZIP, prepended self-extracting data, trailing data outside the ZIP comment, encrypted or symbolic-link entries, and compression methods other than Stored and Deflate.
- Duplicate or ASCII-case-equivalent part names; absolute ZIP entry names; path traversal; backslashes, control characters, URI schemes, query or fragment delimiters, percent-encoded internal names, empty path segments, and names over 1,024 bytes. Internal relationships may use normalized relative `.` and `..` segments only if they remain inside the package.

Unknown markup and non-WordprocessingML text are preserved. Comments, document properties, chart and SmartArt labels, equation text, image pixels, embedded files, external resources, and unreachable story parts are not diacritized. No external resources are fetched and no ZIP files are extracted to disk. This is a structural paragraph transform, not a layout engine: hidden text inherited through styles, pagination, field recalculation, and application-specific extension semantics are not evaluated. Ordinary `w:t` text in retained runs is processed even if a style hides it.

## Resource limits

Each namespace declaration value is limited to 1,024 bytes in its original XML spelling, including entity references. This fixed preflight bound prevents a long inherited URI from multiplying expanded-name hashing work across descendant attributes.

Content-type defaults and overrides are stored separately. Lookup borrows the matching override or extension default, so a large default value is stored once instead of copied for every matching ZIP part. Additional content-type storage scales with the declarations in the bounded content-types XML, without multiplying default-value size by the package entry count.

`Limits::default()` allows a 128 MiB input ZIP, 4,096 entries including directories, 32 MiB per inflated part, 256 MiB total inflated bytes, 8 MiB per parsed or edited XML part, XML depth 128, 200,000 XML events per parsed part, 128 attributes per XML element including namespace declarations, 1 MiB per input or output paragraph, and a 256 MiB returned ZIP. Each field can be configured through `transform_with_limits`; values must be positive, and both XML depth and `max_xml_attributes_per_element` cannot exceed 256.

ZIP central-directory bounds and entry count are checked before constructing the ZIP reader. Header consistency and overlapping entries are checked, followed by bounded actual reads and CRC verification of every payload, including untouched parts. Declared sizes alone are not trusted. XML size, event count, depth, attribute count, encoding, and DTD checks precede tree construction. Attribute counting scans linearly and stops at the first excess attribute, before docxml's parsing and duplicate-attribute checks. Namespace declarations are indexed per element; namespace validation, element names, field attributes, and `AlternateContent` requirements use those indexes with a depth-bounded ancestor walk, without rescanning attribute lists for each prefix. Edited XML, total inflated output, callback output, and actual ZIP writes have separate bounds. The callback must bound its own allocations and execution time; the library can check a returned string only after the callback returns it.

## Verification

`cargo test -p tashkeel --test docx --locked` runs model-free synthetic package tests. The fixtures cover styled Arabic and mixed-language text, a word split across runs, all eight replacement marks, exact package part sets and untouched bytes, tables, headers, footers, notes, images, hyperlinks, tabs and breaks, entities and CDATA, unsafe `]]>` creation, Strict and Transitional namespaces, renamed/default and scoped field-attribute prefixes, relationship-based part discovery, case-equivalent references and relationship filenames, original ZIP spelling after edits, tracked changes, fields, text boxes, alternate content, preservation violations, cancellation, malformed inputs, duplicate paths and overrides, internal URI restrictions, and configurable limits. Attribute-limit tests include a 30,000-attribute document and boundary checks on both start and empty elements. `cargo test -p tashkeel --lib docx --locked` also checks a 1 MiB content-type default shared by 192 tiny parts: table sizes and borrowed-value identity prove there is no per-part value copy, and a no-op transform retains the exact archive bytes. These tests do not claim visual compatibility with every Word or LibreOffice release.
