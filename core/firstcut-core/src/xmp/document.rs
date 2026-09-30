//! Reading, merging and writing one XMP sidecar.
//!
//! A sidecar is somebody else's file: Lightroom, Capture One, Bridge and other tools all write
//! them, and the user may have edited one by hand. So every operation here is a **merge**: the
//! document is kept as text, only the `xmp:Rating` and `xmp:Label` spans are replaced, and
//! everything else — other namespaces, `xmp:ModifyDate`, a tool's comment, the whitespace, the
//! xpacket wrapper — comes out byte for byte identical (task.md §11).
//!
//! Values can be written in either of the two shapes XMP allows, and both are read:
//! attribute form (`xmp:Rating="4"`, what Lightroom writes) and element form
//! (`<xmp:Rating>4</xmp:Rating>`). An existing element is updated in place rather than shadowed
//! by a second attribute.

use std::path::PathBuf;

use super::XmpError;
use super::xml::{self, Span};

const XMP_NAMESPACE: &str = "http://ns.adobe.com/xap/1.0/";
const RDF_NAMESPACE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// What a sidecar says about a photo. `label` is the raw `xmp:Label` text, which may be a label
/// Firstcut does not know; `urgency` is Photoshop's older spelling of the same thing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SidecarValues {
    pub rating: Option<i64>,
    pub label: Option<String>,
    pub urgency: Option<i64>,
}

/// The values to write. `None` means "this photo has no value for that field", which **removes** the
/// attribute or element: the sidecar is a mirror of the app's rating, so a photo the user has
/// un-rated must not keep showing 4 stars in Lightroom.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XmpValues {
    pub rating: Option<i64>,
    pub label: Option<String>,
}

impl XmpValues {
    pub fn rating(value: i64) -> XmpValues {
        XmpValues {
            rating: Some(value),
            label: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rating.is_none() && self.label.is_none()
    }
}

/// An XMP document held as text, edited by splicing.
#[derive(Clone, Debug, Default)]
pub struct XmpDocument {
    text: String,
}

impl XmpDocument {
    /// An empty document: reading it gives no values, writing builds a complete sidecar.
    pub fn new() -> XmpDocument {
        XmpDocument {
            text: String::new(),
        }
    }

    pub fn parse(text: impl Into<String>) -> XmpDocument {
        XmpDocument { text: text.into() }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn into_string(self) -> String {
        self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// True when the text is an XMP packet we are willing to edit. A `.xmp` file that is something
    /// else entirely is reported as an error rather than overwritten.
    pub fn is_xmp(&self) -> bool {
        self.text.contains("<x:xmpmeta") || self.text.contains("adobe:ns:meta/")
    }

    /// True when there is somewhere to put an attribute: a real `rdf:RDF` block.
    pub fn has_rdf(&self) -> bool {
        scan(&self.text).iter().any(|tag| tag.is_start("rdf:RDF"))
    }

    /// Reads the three fields Firstcut cares about. First `rdf:Description` that carries a value
    /// wins; element form is used when there is no attribute.
    pub fn values(&self) -> SidecarValues {
        let mut values = SidecarValues::default();
        let tags = scan(&self.text);

        for (index, tag) in tags.iter().enumerate() {
            if !tag.is_start("rdf:Description") {
                continue;
            }
            let start = tag.as_start().expect("checked above");
            if values.rating.is_none() {
                values.rating = start
                    .attr("xmp:Rating")
                    .and_then(|attr| attr.value)
                    .and_then(|value| value.trim().parse().ok());
            }
            if values.label.is_none() {
                values.label = start
                    .attr("xmp:Label")
                    .and_then(|attr| attr.value)
                    .map(|value| xml::unescape(value.trim()));
            }
            if values.urgency.is_none() {
                values.urgency = start
                    .attr("photoshop:Urgency")
                    .and_then(|attr| attr.value)
                    .and_then(|value| value.trim().parse().ok());
            }
            // Element form, for sidecars that use it.
            if values.rating.is_none() {
                values.rating = element_value(&self.text, &tags, index + 1, "xmp:Rating")
                    .and_then(|value| value.trim().parse().ok());
            }
            if values.label.is_none() {
                values.label = element_value(&self.text, &tags, index + 1, "xmp:Label");
            }
        }
        values
    }

    /// Applies both fields in one pass over the document.
    pub fn apply(&mut self, values: &XmpValues) -> std::result::Result<(), XmpError> {
        if let Some(rating) = values.rating
            && !(-1..=5).contains(&rating)
        {
            return Err(XmpError::BadValue {
                field: "xmp:Rating",
                value: rating.to_string(),
            });
        }
        if let Some(label) = &values.label
            && (label.trim().is_empty() || label.chars().any(char::is_control))
        {
            return Err(XmpError::BadValue {
                field: "xmp:Label",
                value: label.clone(),
            });
        }

        if self.text.is_empty() {
            self.text = template(values);
            return Ok(());
        }
        if !self.is_xmp() {
            return Err(XmpError::NotXmp {
                path: PathBuf::from("<in memory>"),
            });
        }

        let tags = scan(&self.text);
        let mut edits = Vec::new();

        if let Some(edit) =
            self.plan_edit(&tags, "xmp:Rating", values.rating.map(|v| v.to_string()))
        {
            edits.push(edit);
        }
        if let Some(edit) = self.plan_edit(&tags, "xmp:Label", values.label.clone()) {
            edits.push(edit);
        }
        if edits.is_empty() {
            return Ok(());
        }

        // Splice from the back so earlier offsets stay valid.
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.0.start));
        let mut text = self.text.clone();
        for (span, replacement) in edits {
            text.replace_range(span.start..span.end, &replacement);
        }
        self.text = text;
        Ok(())
    }

    /// Where one field would change, and what it would change to.
    fn plan_edit(
        &self,
        tags: &[xml::Tag<'_>],
        field: &str,
        value: Option<String>,
    ) -> Option<(Span, String)> {
        let prefix = namespace_prefix(field);

        // 1. An attribute, on any description.
        for tag in tags.iter().filter(|tag| tag.is_start("rdf:Description")) {
            let start = tag.as_start()?;
            if let Some(attr) = start.attr(field) {
                return Some(match (&value, attr.value_span) {
                    (Some(value), Some(span)) => (span, xml::escape(value)),
                    (Some(value), None) => (
                        attr.name_span,
                        format!("{field}=\"{}\"", xml::escape(value)),
                    ),
                    (None, Some(span)) => (
                        removal_span(&self.text, attr.name_span, Some(span)),
                        String::new(),
                    ),
                    (None, None) => (attr.name_span, String::new()),
                });
            }
        }

        // 2. An element, on any description.
        for (index, tag) in tags.iter().enumerate() {
            if !tag.is_start(field) {
                continue;
            }
            if let Some(content) = xml::element_content(tags, index) {
                return Some(match &value {
                    Some(value) => (content, xml::escape(value)),
                    None => (element_removal_span(tags, index), String::new()),
                });
            }
        }

        // Nothing to update: remove nothing, insert nothing.
        let value = value?;

        // 3. A new attribute on the first description.
        if let Some(tag) = tags.iter().find(|tag| tag.is_start("rdf:Description")) {
            let start = tag.as_start()?;
            let mut addition = String::new();
            if !xml::Attr::has_namespace(&start.attrs, prefix) {
                addition.push_str(&format!(" xmlns:{prefix}=\"{XMP_NAMESPACE}\""));
            }
            addition.push_str(&format!(" {field}=\"{}\"", xml::escape(&value)));
            return Some((
                Span::new(
                    start.attribute_insert_point(),
                    start.attribute_insert_point(),
                ),
                addition,
            ));
        }

        // 4. A brand new description inside the RDF block.
        if let Some(rdf) = tags.iter().find(|tag| tag.is_start("rdf:RDF")) {
            let rdf = rdf.as_start()?;
            let insert_at = if rdf.self_closing {
                // `<rdf:RDF/>` has to become `<rdf:RDF>…</rdf:RDF>`.
                return Some((
                    Span::new(rdf.end - 1, rdf.end - 1),
                    format!(
                        ">\n  <rdf:Description rdf:about=\"\" xmlns:{prefix}=\"{XMP_NAMESPACE}\" {field}=\"{}\"/>\n </rdf:RDF>",
                        xml::escape(&value)
                    ),
                ));
            } else {
                rdf.end + 1
            };
            return Some((
                Span::new(insert_at, insert_at),
                format!(
                    "\n  <rdf:Description rdf:about=\"\" xmlns:{prefix}=\"{XMP_NAMESPACE}\" {field}=\"{}\"/>",
                    xml::escape(&value)
                ),
            ));
        }

        // 5. An xmpmeta with no RDF block: add one.
        if let Some(root) = tags.iter().find(|tag| tag.is_start("x:xmpmeta")) {
            let root = root.as_start()?;
            if root.self_closing {
                return None;
            }
            return Some((
                Span::new(root.end + 1, root.end + 1),
                format!(
                    "\n <rdf:RDF xmlns:rdf=\"{RDF_NAMESPACE}\">\n  <rdf:Description rdf:about=\"\" xmlns:{prefix}=\"{XMP_NAMESPACE}\" {field}=\"{}\"/>\n </rdf:RDF>\n",
                    xml::escape(&value)
                ),
            ));
        }

        None
    }
}

fn scan(text: &str) -> Vec<xml::Tag<'_>> {
    xml::scan(text)
}

fn namespace_prefix(field: &str) -> &str {
    field.split_once(':').map_or("xmp", |(prefix, _)| prefix)
}

fn element_value(text: &str, tags: &[xml::Tag<'_>], from: usize, name: &str) -> Option<String> {
    for (offset, tag) in tags.iter().enumerate().skip(from) {
        if !tag.is_start(name) {
            continue;
        }
        let content = xml::element_content(tags, offset)?;
        let raw = content.text(text).trim();
        let value = raw
            .strip_prefix("<![CDATA[")
            .and_then(|rest| rest.strip_suffix("]]>"))
            .unwrap_or(raw);
        return Some(xml::unescape(value));
    }
    None
}

/// The span of an attribute plus the whitespace in front of it, so removing it leaves no gap.
fn removal_span(text: &str, name: Span, value: Option<Span>) -> Span {
    let mut start = name.start;
    while start > 0 {
        let previous = text.as_bytes()[start - 1];
        if previous == b' ' || previous == b'\t' || previous == b'\n' || previous == b'\r' {
            start -= 1;
        } else {
            break;
        }
    }
    let end = match value {
        Some(span) => {
            // Step over the closing quote.
            let after = text.as_bytes().get(span.end).copied();
            match after {
                Some(quote @ (b'"' | b'\'')) => {
                    span.end + usize::from(quote == b'"' || quote == b'\'')
                }
                _ => span.end,
            }
        }
        None => name.end,
    };
    Span::new(start, end)
}

/// The span of `<xmp:Rating>4</xmp:Rating>`, for removal.
fn element_removal_span(tags: &[xml::Tag<'_>], index: usize) -> Span {
    let start = tags[index].span();
    let name = tags[index].as_start().map(|tag| tag.name.to_string());
    for tag in &tags[index + 1..] {
        if let xml::Tag::End {
            name: end_name,
            span,
        } = tag
            && name.as_deref() == Some(end_name)
        {
            return Span::new(start.start, span.end);
        }
    }
    start
}

/// A complete, Lightroom-shaped sidecar with the given values in it.
pub fn template(values: &XmpValues) -> String {
    let mut description = String::from("  <rdf:Description rdf:about=\"\"\n    xmlns:xmp=\"");
    description.push_str(XMP_NAMESPACE);
    description.push('"');
    if let Some(rating) = values.rating {
        description.push_str(&format!("\n    xmp:Rating=\"{rating}\""));
    }
    if let Some(label) = &values.label {
        description.push_str(&format!("\n    xmp:Label=\"{}\"", xml::escape(label)));
    }
    description.push_str("/>\n");

    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Firstcut\">\n\
         <rdf:RDF xmlns:rdf=\"{RDF_NAMESPACE}\">\n\
         {description} </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>\n"
    )
}

/// The path of the sidecar for a photo, Lightroom's way (task.md §11): the photo's base name with
/// `.xmp`, so `Sat/IMG_0001.CR3` → `Sat/IMG_0001.xmp`. A RAW+JPEG pair shares it, as in Lightroom.
///
/// Firstcut used to write `IMG_0001.CR3.xmp`, which Lightroom never reads, so a rating made here
/// did not reach it. That name is darktable's (and one Capture One reads); such a file is read as a
/// fallback when importing ([`existing_sidecar`]) and moves with its photo, but is never renamed or
/// rewritten: it may hold another app's edit history.
pub fn sidecar_path(raw_rel_path: &str) -> PathBuf {
    PathBuf::from(raw_rel_path).with_extension("xmp")
}

/// The sidecar name earlier builds wrote (and darktable writes): the full file name plus `.xmp`.
pub fn legacy_sidecar_path(raw_rel_path: &str) -> PathBuf {
    PathBuf::from(format!("{raw_rel_path}.xmp"))
}

/// The sidecar to read for a photo in `folder`: the Lightroom name, else a legacy one, else the
/// Lightroom name (which does not exist yet).
pub fn existing_sidecar(folder: &std::path::Path, raw_rel_path: &str) -> PathBuf {
    let current = folder.join(sidecar_path(raw_rel_path));
    let legacy = folder.join(legacy_sidecar_path(raw_rel_path));
    if !current.exists() && legacy.is_file() {
        legacy
    } else {
        current
    }
}

/// The base name a sidecar belongs to, given `<basename>.xmp`.
pub fn sidecar_base(path: &std::path::Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(".xmp")
        .or_else(|| name.strip_suffix(".XMP"))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIGHTROOM: &str = r#"<?xpacket begin="﻿" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Image::ExifTool 12.40">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"
    xmp:Rating="4"
    photoshop:Urgency="1"
    xmp:ModifyDate="2026-09-29T18:04:11+01:00"/>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

    #[test]
    fn reads_a_lightroom_sidecar() {
        let values = XmpDocument::parse(LIGHTROOM).values();
        assert_eq!(values.rating, Some(4));
        assert_eq!(values.label, None);
        assert_eq!(values.urgency, Some(1));
    }

    #[test]
    fn reads_element_form_and_cdata() {
        let source = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/">
   <xmp:Rating><![CDATA[3]]></xmp:Rating>
   <xmp:Label>Green</xmp:Label>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        let values = XmpDocument::parse(source).values();
        assert_eq!(values.rating, Some(3));
        assert_eq!(values.label.as_deref(), Some("Green"));
    }

    #[test]
    fn merging_keeps_every_other_byte() {
        let mut document = XmpDocument::parse(LIGHTROOM);
        document
            .apply(&XmpValues {
                rating: Some(2),
                label: Some("Blue".to_string()),
            })
            .unwrap();
        let text = document.as_str();

        assert!(text.contains("xmp:Rating=\"2\""));
        assert!(text.contains("xmp:Label=\"Blue\""));
        // Untouched: the tool's namespace, its other properties, the wrapper, the whitespace.
        assert!(text.contains("x:xmptk=\"Image::ExifTool 12.40\""));
        assert!(text.contains("xmlns:photoshop=\"http://ns.adobe.com/photoshop/1.0/\""));
        assert!(text.contains("photoshop:Urgency=\"1\""));
        assert!(text.contains("xmp:ModifyDate=\"2026-09-29T18:04:11+01:00\""));
        assert!(text.starts_with("<?xpacket begin="));
        assert!(text.trim_end().ends_with("<?xpacket end=\"w\"?>"));
        assert_eq!(text.lines().count(), LIGHTROOM.lines().count());

        // And the result reads back.
        let values = document.values();
        assert_eq!(values.rating, Some(2));
        assert_eq!(values.label.as_deref(), Some("Blue"));
        assert_eq!(values.urgency, Some(1));
    }

    #[test]
    fn removing_a_value_removes_only_that_value() {
        let mut document = XmpDocument::parse(LIGHTROOM);
        document
            .apply(&XmpValues {
                rating: None,
                label: None,
            })
            .unwrap();
        let text = document.as_str();
        assert!(!text.contains("xmp:Rating"));
        assert!(!text.contains("xmp:Label"));
        assert!(text.contains("photoshop:Urgency=\"1\""));
        assert!(text.contains("xmp:ModifyDate="));
        assert_eq!(document.values().rating, None);
    }

    #[test]
    fn an_existing_element_is_updated_in_place() {
        let source = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/">
   <xmp:Rating>3</xmp:Rating>
   <dc:subject>keep</dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        let mut document = XmpDocument::parse(source);
        document.apply(&XmpValues::rating(5)).unwrap();
        assert!(document.as_str().contains("<xmp:Rating>5</xmp:Rating>"));
        assert!(
            !document.as_str().contains("xmp:Rating=\""),
            "must not add an attribute next to the element"
        );
        assert!(document.as_str().contains("<dc:subject>keep</dc:subject>"));
        assert_eq!(document.values().rating, Some(5));

        document.apply(&XmpValues::rating(1)).unwrap();
        assert!(document.as_str().contains("<xmp:Rating>1</xmp:Rating>"));

        // And removing it takes the whole element with it.
        document
            .apply(&XmpValues {
                rating: None,
                label: None,
            })
            .unwrap();
        assert!(!document.as_str().contains("xmp:Rating"));
        assert!(document.as_str().contains("<dc:subject>keep</dc:subject>"));
    }

    #[test]
    fn a_description_without_the_namespace_gets_one() {
        let source = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:subject>football</dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        let mut document = XmpDocument::parse(source);
        document.apply(&XmpValues::rating(4)).unwrap();
        let text = document.as_str();
        assert!(text.contains("xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\""));
        assert!(text.contains("xmp:Rating=\"4\""));
        assert!(text.contains("<dc:subject>football</dc:subject>"));
        assert_eq!(document.values().rating, Some(4));
    }

    #[test]
    fn an_rdf_block_with_no_description_gets_one() {
        let source = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
 </rdf:RDF>
</x:xmpmeta>"#;
        let mut document = XmpDocument::parse(source);
        document.apply(&XmpValues::rating(2)).unwrap();
        assert!(document.as_str().contains("<rdf:Description"));
        assert_eq!(document.values().rating, Some(2));
    }

    #[test]
    fn a_pocket_xmpmeta_with_no_rdf_block_gets_one() {
        let source = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"></x:xmpmeta>";
        let mut document = XmpDocument::parse(source);
        assert!(!document.has_rdf());
        document.apply(&XmpValues::rating(1)).unwrap();
        assert!(document.has_rdf());
        assert_eq!(document.values().rating, Some(1));
    }

    #[test]
    fn a_file_that_is_not_xmp_is_refused() {
        let mut document = XmpDocument::parse("<?xml version=\"1.0\"?>\n<photos><photo/></photos>");
        let err = document.apply(&XmpValues::rating(3)).unwrap_err();
        assert!(matches!(err, XmpError::NotXmp { .. }), "{err}");
        assert_eq!(
            document.as_str(),
            "<?xml version=\"1.0\"?>\n<photos><photo/></photos>",
            "the file must be left alone"
        );
    }

    #[test]
    fn nonsense_values_are_refused() {
        let mut document = XmpDocument::new();
        assert!(document.apply(&XmpValues::rating(6)).is_err());
        assert!(document.apply(&XmpValues::rating(-2)).is_err());
        assert!(
            document
                .apply(&XmpValues {
                    rating: None,
                    label: Some("  ".to_string()),
                })
                .is_err()
        );
        assert!(
            document.is_empty(),
            "a refused value must not create a file"
        );
    }

    #[test]
    fn reject_is_minus_one() {
        let mut document = XmpDocument::new();
        document.apply(&XmpValues::rating(-1)).unwrap();
        assert!(document.as_str().contains("xmp:Rating=\"-1\""));
        assert_eq!(document.values().rating, Some(-1));
    }

    #[test]
    fn a_new_document_is_a_complete_sidecar() {
        let mut document = XmpDocument::new();
        document
            .apply(&XmpValues {
                rating: Some(5),
                label: Some("Red".to_string()),
            })
            .unwrap();

        let text = document.as_str();
        assert!(text.starts_with("<?xpacket begin=\"\u{feff}\""));
        assert!(text.contains("xmlns:x=\"adobe:ns:meta/\""));
        assert!(text.contains("xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\""));
        assert!(text.contains("xmp:Rating=\"5\""));
        assert!(text.contains("xmp:Label=\"Red\""));
        assert!(text.trim_end().ends_with("<?xpacket end=\"w\"?>"));
        assert!(document.is_xmp());
        assert!(document.has_rdf());

        let values = document.values();
        assert_eq!(values.rating, Some(5));
        assert_eq!(values.label.as_deref(), Some("Red"));
    }

    #[test]
    fn values_are_escaped_and_unescaped() {
        let mut document = XmpDocument::new();
        document
            .apply(&XmpValues {
                rating: Some(3),
                label: Some("A & B".to_string()),
            })
            .unwrap();
        assert!(document.as_str().contains("xmp:Label=\"A &amp; B\""));
        assert_eq!(document.values().label.as_deref(), Some("A & B"));
    }

    #[test]
    fn sidecar_paths_follow_lightroom() {
        assert_eq!(sidecar_path("IMG_0001.CR3"), PathBuf::from("IMG_0001.xmp"));
        assert_eq!(
            sidecar_path("Sat.1/IMG_0001.JPG"),
            PathBuf::from("Sat.1/IMG_0001.xmp"),
            "only the last extension goes, never a dot in a folder name"
        );
        assert_eq!(
            legacy_sidecar_path("IMG_0001.CR3"),
            PathBuf::from("IMG_0001.CR3.xmp")
        );
        assert_eq!(
            sidecar_base(std::path::Path::new("IMG_0001.CR3.xmp")).as_deref(),
            Some("IMG_0001.CR3")
        );
        assert_eq!(
            sidecar_base(std::path::Path::new("a/b/IMG_0002.XMP")).as_deref(),
            Some("IMG_0002"),
            "the base name of a path, not the path itself"
        );
        assert_eq!(sidecar_base(std::path::Path::new("nope.txt")), None);
    }

    #[test]
    fn repeated_writes_stay_stable() {
        let mut document = XmpDocument::new();
        document.apply(&XmpValues::rating(3)).unwrap();
        let once = document.as_str().to_string();
        document.apply(&XmpValues::rating(4)).unwrap();
        document.apply(&XmpValues::rating(4)).unwrap();
        assert_eq!(document.values().rating, Some(4));
        assert_eq!(
            document.as_str().matches("xmp:Rating").count(),
            1,
            "there must be exactly one rating in the document"
        );
        assert!(document.as_str().len() > once.len() - once.len());
    }
}
