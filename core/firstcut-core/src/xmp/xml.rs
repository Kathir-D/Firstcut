//! A byte-offset scanner for the XML that XMP sidecars actually contain.
//!
//! Why not a parser: writing a sidecar means changing `xmp:Rating` and `xmp:Label` and leaving
//! **every other byte alone** (todo.md §11: "preserve any existing unknown XMP content (merge,
//! don't clobber)"). Round-tripping a document through a DOM re-serialises it: attribute order,
//! self-closing style, entity spellings and whitespace all move. So this module produces spans —
//! offsets into the original text — and [`super::document`] splices new text into them.
//!
//! It handles what sidecars contain and skips the rest safely: start tags with attributes, end
//! tags, self-closing tags, comments, processing instructions, the XML declaration, the DOCTYPE and
//! CDATA. It is not a validating parser and is not meant to be one: an XMP file is read as data
//! and rewritten surgically, and anything it cannot make sense of is left untouched.

/// A half-open byte range of the source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }

    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start..self.end]
    }

    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// One attribute of a start tag.
#[derive(Clone, Debug)]
pub struct Attr<'a> {
    /// Qualified name as written, e.g. `xmp:Rating` or `xmlns:xmp`.
    pub name: &'a str,
    pub name_span: Span,
    /// `None` for a valueless attribute such as `xmp:ReadOnly`.
    pub value: Option<&'a str>,
    /// The value without its quotes, for attributes that have one.
    pub value_span: Option<Span>,
}

impl<'a> Attr<'a> {
    /// The attribute with the given qualified name, ignoring case: XML names are
    /// case-sensitive, but the tools that write XMP are not consistent, and a `xmp:rating` from
    /// another program should still be found rather than duplicated.
    pub fn find<'b>(attrs: &'b [Attr<'a>], name: &str) -> Option<&'b Attr<'a>> {
        attrs
            .iter()
            .find(|attr| attr.name.eq_ignore_ascii_case(name))
    }

    pub fn has_namespace(attrs: &[Attr<'_>], prefix: &str) -> bool {
        let declaration = format!("xmlns:{prefix}");
        attrs
            .iter()
            .any(|attr| attr.name.eq_ignore_ascii_case(&declaration))
    }
}

#[derive(Clone, Debug)]
pub struct StartTag<'a> {
    pub name: &'a str,
    /// The whole tag, from `<` to `>` inclusive.
    pub span: Span,
    /// Offset of the `>`.
    pub end: usize,
    pub self_closing: bool,
    pub attrs: Vec<Attr<'a>>,
}

impl<'a> StartTag<'a> {
    /// Where a new attribute can be inserted: just before `/>` or `>`.
    pub fn attribute_insert_point(&self) -> usize {
        if self.self_closing {
            self.end - 1
        } else {
            self.end
        }
    }

    pub fn attr(&self, name: &str) -> Option<&Attr<'a>> {
        Attr::find(&self.attrs, name)
    }
}

#[derive(Clone, Debug)]
pub enum Tag<'a> {
    Start(StartTag<'a>),
    End {
        name: &'a str,
        span: Span,
    },
    /// Text, comments, processing instructions, the DOCTYPE, CDATA.
    Other {
        span: Span,
    },
}

impl<'a> Tag<'a> {
    pub fn span(&self) -> Span {
        match self {
            Tag::Start(tag) => tag.span,
            Tag::End { span, .. } | Tag::Other { span } => *span,
        }
    }

    pub fn as_start(&self) -> Option<&StartTag<'a>> {
        match self {
            Tag::Start(tag) => Some(tag),
            _ => None,
        }
    }

    pub fn is_start(&self, name: &str) -> bool {
        matches!(self, Tag::Start(tag) if tag.name.eq_ignore_ascii_case(name))
    }
}

/// Scans `source` into tags, in order. Offsets are byte offsets into `source`.
pub fn scan(source: &str) -> Vec<Tag<'_>> {
    let bytes = source.as_bytes();
    let mut tags = Vec::new();
    let mut pos = 0;

    while pos < bytes.len() {
        let Some(lt) = find(bytes, b'<', pos) else {
            break;
        };
        if lt > pos {
            // Text between tags. Whitespace-only runs are not interesting, but non-whitespace text
            // is recorded so nothing is silently ignored.
            let text = &source[pos..lt];
            if !text.trim().is_empty() {
                tags.push(Tag::Other {
                    span: Span::new(pos, lt),
                });
            }
        }

        if starts_with(bytes, lt, b"<!--") {
            let end = find_from(bytes, b"-->", lt + 4).map_or(bytes.len(), |i| i + 3);
            tags.push(Tag::Other {
                span: Span::new(lt, end),
            });
            pos = end;
        } else if starts_with(bytes, lt, b"<![CDATA[") {
            let end = find_from(bytes, b"]]>", lt + 9).map_or(bytes.len(), |i| i + 3);
            tags.push(Tag::Other {
                span: Span::new(lt, end),
            });
            pos = end;
        } else if starts_with(bytes, lt, b"<?") {
            let end = find_from(bytes, b"?>", lt + 2).map_or(bytes.len(), |i| i + 2);
            tags.push(Tag::Other {
                span: Span::new(lt, end),
            });
            pos = end;
        } else if starts_with(bytes, lt, b"<!") {
            // DOCTYPE and friends. An internal subset ends with `]>`, not `>`.
            let end =
                if find_from(bytes, b"[", lt).is_some_and(|i| i < find_or_end(bytes, b">", lt)) {
                    find_from(bytes, b"]>", lt).map_or(bytes.len(), |i| i + 2)
                } else {
                    find_from(bytes, b">", lt).map_or(bytes.len(), |i| i + 1)
                };
            tags.push(Tag::Other {
                span: Span::new(lt, end),
            });
            pos = end;
        } else if starts_with(bytes, lt, b"</") {
            let name_start = lt + 2;
            let Some(name_end) =
                find_any(bytes, name_start, |b| b.is_ascii_whitespace() || *b == b'>')
            else {
                break;
            };
            let name = &source[name_start..name_end];
            let end = find_from(bytes, b">", name_end).map_or(bytes.len(), |i| i + 1);
            tags.push(Tag::End {
                name,
                span: Span::new(lt, end),
            });
            pos = end;
        } else {
            match parse_start_tag(source, bytes, lt) {
                Some((tag, end)) => {
                    let self_closing = tag.self_closing;
                    tags.push(Tag::Start(tag));
                    pos = end;
                    if self_closing {
                        continue;
                    }
                }
                None => {
                    // A `<` that starts nothing we understand: treat it as text and move on, so one
                    // odd byte cannot swallow the rest of the file.
                    pos = lt + 1;
                }
            }
        }
    }

    tags
}

fn parse_start_tag<'a>(source: &'a str, bytes: &[u8], lt: usize) -> Option<(StartTag<'a>, usize)> {
    let name_start = lt + 1;
    if name_start >= bytes.len() || !is_name_start(bytes[name_start]) {
        return None;
    }
    let name_end = find_any(bytes, name_start, |b| {
        b.is_ascii_whitespace() || *b == b'/' || *b == b'>'
    })?;
    let name = &source[name_start..name_end];

    let mut attrs = Vec::new();
    let mut pos = name_end;
    let mut self_closing = false;

    loop {
        pos = skip_whitespace(bytes, pos);
        if pos >= bytes.len() {
            return None;
        }
        match bytes[pos] {
            b'>' => {
                return Some((
                    StartTag {
                        name,
                        span: Span::new(lt, pos + 1),
                        end: pos,
                        self_closing,
                        attrs,
                    },
                    pos + 1,
                ));
            }
            b'/' if bytes.get(pos + 1) == Some(&b'>') => {
                self_closing = true;
                return Some((
                    StartTag {
                        name,
                        span: Span::new(lt, pos + 2),
                        end: pos + 1,
                        self_closing,
                        attrs,
                    },
                    pos + 2,
                ));
            }
            _ => {}
        }

        if !is_name_start(bytes[pos]) {
            // Unexpected byte inside a tag: skip it rather than give up on the whole document.
            pos += 1;
            continue;
        }
        let attr_name_end = find_any(bytes, pos, |b| {
            b.is_ascii_whitespace() || *b == b'=' || *b == b'>' || *b == b'/'
        })
        .unwrap_or(bytes.len());
        let attr_name = &source[pos..attr_name_end];
        let name_span = Span::new(pos, attr_name_end);
        pos = skip_whitespace(bytes, attr_name_end);

        let (value, value_span) = if bytes.get(pos) == Some(&b'=') {
            pos = skip_whitespace(bytes, pos + 1);
            match bytes.get(pos) {
                Some(quote @ (b'"' | b'\'')) => {
                    let value_start = pos + 1;
                    let value_end = find_from(bytes, &[*quote], value_start)
                        .unwrap_or(bytes.len())
                        .min(bytes.len());
                    (
                        Some(&source[value_start..value_end]),
                        Some(Span::new(value_start, value_end)),
                    )
                }
                // Unquoted value: read up to whitespace or the end of the tag.
                _ => {
                    let value_end = find_any(bytes, pos, |b| {
                        b.is_ascii_whitespace() || *b == b'>' || *b == b'/'
                    })
                    .unwrap_or(bytes.len());
                    (
                        Some(&source[pos..value_end]),
                        Some(Span::new(pos, value_end)),
                    )
                }
            }
        } else {
            (None, None)
        };

        attrs.push(Attr {
            name: attr_name,
            name_span,
            value,
            value_span,
        });
        pos = match value_span {
            Some(span) => span.end,
            None => pos,
        };
    }
}

/// The text between `<name>` and its `</name>`, when the element has no child elements.
///
/// `index` must point at a start tag. Returns `None` for a self-closing tag, an unclosed tag, or
/// an element with element children. The caller slices the span out of its own copy of the source.
pub fn element_content(tags: &[Tag<'_>], index: usize) -> Option<Span> {
    let start = tags.get(index)?.as_start()?;
    if start.self_closing {
        return None;
    }
    let mut depth = 0usize;
    for tag in &tags[index + 1..] {
        match tag {
            Tag::Start(inner) if inner.name.eq_ignore_ascii_case(start.name) => depth += 1,
            Tag::End { name, span } if name.eq_ignore_ascii_case(start.name) => {
                if depth == 0 {
                    return Some(Span::new(start.end + 1, span.start));
                }
                depth -= 1;
            }
            Tag::Start(_) => return None,
            _ => {}
        }
    }
    None
}

fn find(bytes: &[u8], needle: u8, from: usize) -> Option<usize> {
    find_from(bytes, &[needle], from)
}

fn find_from(bytes: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= bytes.len() {
        return None;
    }
    (from..=bytes.len().saturating_sub(needle.len()))
        .find(|&i| &bytes[i..i + needle.len()] == needle)
}

fn find_or_end(bytes: &[u8], needle: &[u8], from: usize) -> usize {
    find_from(bytes, needle, from).unwrap_or(bytes.len())
}

fn find_any(bytes: &[u8], from: usize, predicate: impl Fn(&u8) -> bool) -> Option<usize> {
    (from..bytes.len()).find(|&i| predicate(&bytes[i]))
}

fn starts_with(bytes: &[u8], at: usize, prefix: &[u8]) -> bool {
    bytes.len() >= at + prefix.len() && &bytes[at..at + prefix.len()] == prefix
}

fn skip_whitespace(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b':'
}

/// Undoes the five XML entities plus numeric character references. Anything unrecognised is left
/// as it is, so a value we do not understand is never silently changed.
pub fn unescape(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        let tail = &rest[index..];
        let Some(semi) = tail.find(';').filter(|semi| *semi <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..semi];
        let replacement = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix('#')
                .and_then(|number| match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse().ok(),
                })
                .and_then(char::from_u32),
        };
        match replacement {
            Some(character) => {
                out.push(character);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The inverse of [`unescape`], for the values Firstcut writes.
pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(character),
        }
    }
    out
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
    fn finds_the_description_and_its_attributes() {
        let tags = scan(LIGHTROOM);
        let description = tags
            .iter()
            .find(|tag| tag.is_start("rdf:Description"))
            .expect("no rdf:Description");
        let start = description.as_start().unwrap();

        assert!(start.self_closing);
        assert_eq!(start.attr("rdf:about").unwrap().value, Some(""));
        assert_eq!(start.attr("xmp:Rating").unwrap().value, Some("4"));
        assert_eq!(start.attr("photoshop:Urgency").unwrap().value, Some("1"));
        assert!(Attr::has_namespace(&start.attrs, "xmp"));
        assert!(!Attr::has_namespace(&start.attrs, "dc"));
        assert!(start.attr("dc:creator").is_none());
    }

    #[test]
    fn attribute_lookups_ignore_case() {
        let tags = scan(LIGHTROOM);
        let start = tags
            .iter()
            .find(|tag| tag.is_start("rdf:description"))
            .unwrap()
            .as_start()
            .unwrap();
        assert_eq!(start.attr("XMP:RATING").unwrap().value, Some("4"));
    }

    #[test]
    fn offsets_point_at_the_original_bytes() {
        let tags = scan(LIGHTROOM);
        for tag in &tags {
            let span = tag.span();
            // Every span is a valid, in-bounds slice: no panic, no mis-parse.
            let _ = span.text(LIGHTROOM);
            assert!(span.end <= LIGHTROOM.len());
            assert!(span.start <= span.end);
        }
        let start = tags
            .iter()
            .find(|tag| tag.is_start("rdf:Description"))
            .unwrap()
            .as_start()
            .unwrap();
        assert!(start.span.text(LIGHTROOM).starts_with("<rdf:Description"));
        assert!(start.span.text(LIGHTROOM).ends_with("/>"));
        let rating = start.attr("xmp:Rating").unwrap();
        assert_eq!(rating.value_span.unwrap().text(LIGHTROOM), "4");
    }

    #[test]
    fn comments_cdata_pis_and_doctype_are_skipped() {
        let source = r#"<?xml version="1.0"?>
<!-- a comment with <tags> inside -->
<!DOCTYPE xmpmeta [ <!ENTITY x "y"> ]>
<?xpacket end="w"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/">
   <xmp:Rating><![CDATA[3]]></xmp:Rating>
   <xmp:Label>Red</xmp:Label>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        let tags = scan(source);
        let starts: Vec<&str> = tags
            .iter()
            .filter_map(|tag| tag.as_start().map(|start| start.name))
            .collect();
        assert_eq!(
            starts,
            vec![
                "x:xmpmeta",
                "rdf:RDF",
                "rdf:Description",
                "xmp:Rating",
                "xmp:Label"
            ]
        );

        let rating_index = tags
            .iter()
            .position(|tag| tag.is_start("xmp:Rating"))
            .unwrap();
        let content = element_content(&tags, rating_index).unwrap();
        assert_eq!(content.text(source), "<![CDATA[3]]>");
        assert_eq!(unescape(strip_cdata(content.text(source))), "3");

        let label_index = tags
            .iter()
            .position(|tag| tag.is_start("xmp:Label"))
            .unwrap();
        let content = element_content(&tags, label_index).unwrap();
        assert_eq!(content.text(source), "Red");
    }

    #[test]
    fn nested_elements_have_no_text_content() {
        let tags = scan("<a><b><c>x</c></b></a>");
        assert_eq!(element_content(&tags, 0), None);
    }

    #[test]
    fn valueless_and_unquoted_attributes() {
        let tags = scan("<a flag b='two' c=three>");
        let start = tags[0].as_start().unwrap();
        assert_eq!(start.attr("flag").unwrap().value, None);
        assert_eq!(start.attr("b").unwrap().value, Some("two"));
        assert_eq!(start.attr("c").unwrap().value, Some("three"));
    }

    #[test]
    fn a_stray_angle_bracket_does_not_eat_the_document() {
        let source = "<rdf:RDF>a < b <rdf:Description rdf:about=\"\"/></rdf:RDF>";
        let tags = scan(source);
        assert!(tags.iter().any(|tag| tag.is_start("rdf:Description")));
    }

    #[test]
    fn an_unterminated_tag_is_dropped() {
        let tags = scan("<a b=\"1\"");
        assert!(tags.iter().all(|tag| tag.as_start().is_none()));
    }

    #[test]
    fn entities_round_trip() {
        assert_eq!(unescape("a &amp; b"), "a & b");
        assert_eq!(unescape("&lt;tag&gt;"), "<tag>");
        assert_eq!(unescape("&#65;&#x42;"), "AB");
        assert_eq!(unescape("100% &unknown; kept"), "100% &unknown; kept");
        assert_eq!(unescape("bare & ampersand"), "bare & ampersand");
        assert_eq!(
            escape("a & b <c> \"d\" 'e'"),
            "a &amp; b &lt;c&gt; &quot;d&quot; &apos;e&apos;"
        );
        assert_eq!(unescape(&escape("Red & Green")), "Red & Green");
    }

    fn strip_cdata(text: &str) -> &str {
        text.trim()
            .strip_prefix("<![CDATA[")
            .and_then(|rest| rest.strip_suffix("]]>"))
            .unwrap_or(text)
    }
}
