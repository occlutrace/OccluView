use crate::error::HpsError;
use std::collections::BTreeMap;

#[derive(Copy, Clone, Debug)]
pub(super) struct XmlElement<'a> {
    pub(super) open_tag: &'a str,
    pub(super) body: &'a str,
}

pub(super) fn text_from_bytes(bytes: &[u8]) -> Result<&str, HpsError> {
    std::str::from_utf8(bytes).map_err(|_| HpsError::UnsupportedEncoding {
        reason: "raw HPS XML must be valid UTF-8".to_string(),
    })
}

pub(super) fn looks_like_hps_xml(text: &str) -> bool {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rest = text.trim_start();
    if rest.starts_with("<?xml") {
        let Some(end_decl) = rest.find("?>") else {
            return false;
        };
        rest = rest[end_decl + 2..].trim_start();
    }
    if rest.starts_with("<HPS") {
        return true;
    }
    // Slice on a character boundary, not on byte 512. `read_package` runs this
    // over every valid-UTF-8 entry of a package, so a sidecar carrying a
    // patient name in Cyrillic or CJK routinely puts a multi-byte character
    // across that byte — and slicing a `str` there is a panic, which under the
    // shipped `panic = "abort"` profile takes the whole viewer down on open.
    let cap = text.len().min(512);
    let end = (0..=cap)
        .rev()
        .find(|&index| text.is_char_boundary(index))
        .unwrap_or(0);
    let scan = &text[..end];
    scan.contains("<HPS") && scan.contains("<Schema>")
}

pub(super) fn find_element<'a>(xml: &'a str, name: &str) -> Result<XmlElement<'a>, HpsError> {
    find_element_from(xml, name, 0)
}

pub(super) fn find_optional_element<'a>(xml: &'a str, name: &str) -> Option<XmlElement<'a>> {
    find_element(xml, name).ok()
}

pub(super) fn find_elements<'a>(xml: &'a str, name: &str) -> Vec<XmlElement<'a>> {
    let mut elements = Vec::new();
    let mut from = 0;
    while let Ok(element) = find_element_from(xml, name, from) {
        let next = element.open_tag.as_ptr() as usize - xml.as_ptr() as usize
            + element.open_tag.len()
            + element.body.len()
            + name.len()
            + 3;
        elements.push(element);
        if next <= from || next >= xml.len() {
            break;
        }
        from = next;
    }
    elements
}

fn find_element_from<'a>(
    xml: &'a str,
    name: &str,
    from: usize,
) -> Result<XmlElement<'a>, HpsError> {
    let needle = format!("<{name}");
    let mut open = from;
    loop {
        let Some(relative_open) = xml[open..].find(&needle) else {
            return Err(super::malformed(format!("XML missing <{name}> element")));
        };
        open += relative_open;
        let boundary = open + needle.len();
        if xml
            .as_bytes()
            .get(boundary)
            .is_some_and(|&c| is_tag_name_boundary(c))
        {
            break;
        }
        open += 1;
    }

    let Some(relative_open_end) = xml[open..].find('>') else {
        return Err(super::malformed(format!(
            "XML truncated in <{name}> element"
        )));
    };
    let open_end = open + relative_open_end;
    let close_tag = format!("</{name}>");
    let body_start = open_end + 1;
    let Some(relative_close) = xml[body_start..].find(&close_tag) else {
        return Err(super::malformed(format!("XML missing </{name}> element")));
    };
    let close = body_start + relative_close;

    Ok(XmlElement {
        open_tag: &xml[open..=open_end],
        body: &xml[body_start..close],
    })
}

pub(super) fn attr_value<'a>(open_tag: &'a str, attr: &str) -> Result<Option<&'a str>, HpsError> {
    let needle = format!("{attr}=");
    let mut search_from = 0;
    while let Some(relative_pos) = open_tag[search_from..].find(&needle) {
        let pos = search_from + relative_pos;
        if pos == 0 || is_attr_name_boundary(open_tag.as_bytes()[pos - 1]) {
            let mut value_start = pos + needle.len();
            let quote = open_tag.as_bytes().get(value_start).copied();
            if !matches!(quote, Some(b'"' | b'\'')) {
                return Err(super::malformed(format!("XML malformed attribute {attr}")));
            }
            value_start += 1;
            let quote = quote.unwrap_or_default() as char;
            let Some(relative_end) = open_tag[value_start..].find(quote) else {
                return Err(super::malformed(format!(
                    "XML unterminated attribute {attr}"
                )));
            };
            return Ok(Some(&open_tag[value_start..value_start + relative_end]));
        }
        search_from = pos + needle.len();
    }
    Ok(None)
}

pub(super) fn required_attr<'a>(open_tag: &'a str, attr: &str) -> Result<&'a str, HpsError> {
    attr_value(open_tag, attr)?
        .ok_or_else(|| super::malformed(format!("XML missing attribute {attr}")))
}

pub(super) fn parse_usize_attr(value: &str, attr: &str) -> Result<usize, HpsError> {
    value
        .parse::<usize>()
        .map_err(|_| super::malformed(format!("XML invalid integer attribute {attr}")))
}

pub(super) fn optional_usize_attr(open_tag: &str, attr: &str) -> Result<Option<usize>, HpsError> {
    attr_value(open_tag, attr)?
        .map(|value| parse_usize_attr(value, attr))
        .transpose()
}

pub(super) fn parse_u32_attr(value: &str, attr: &str) -> Result<u32, HpsError> {
    value
        .parse::<u32>()
        .map_err(|_| super::malformed(format!("XML invalid uint32 attribute {attr}")))
}

pub(super) fn parse_color_attr(value: &str) -> Result<[u8; 4], HpsError> {
    let parsed = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16)
    } else {
        value.parse::<u32>()
    }
    .map_err(|_| super::malformed("XML invalid color attribute"))?;

    Ok([
        ((parsed >> 16) & 0xff) as u8,
        ((parsed >> 8) & 0xff) as u8,
        (parsed & 0xff) as u8,
        255,
    ])
}

pub(super) fn parse_properties(xml: &str) -> Result<BTreeMap<String, String>, HpsError> {
    let Some(properties) = find_optional_element(xml, "Properties") else {
        return Ok(BTreeMap::new());
    };

    let mut out = BTreeMap::new();
    let mut pos = 0;
    while let Some(relative_open) = properties.body[pos..].find("<Property") {
        let open = pos + relative_open;
        let Some(relative_end) = properties.body[open..].find('>') else {
            break;
        };
        let open_end = open + relative_end;
        let tag = &properties.body[open..=open_end];
        if let Some(name) = attr_value(tag, "name")? {
            let value = attr_value(tag, "value")?.unwrap_or_default();
            out.insert(name.to_string(), value.to_string());
        }
        pos = open_end + 1;
    }
    Ok(out)
}

fn is_tag_name_boundary(ch: u8) -> bool {
    ch == b'>' || ch == b'/' || ch.is_ascii_whitespace()
}

fn is_attr_name_boundary(ch: u8) -> bool {
    ch == b'<' || ch == b'/' || ch.is_ascii_whitespace()
}

#[cfg(test)]
mod tests {
    use super::looks_like_hps_xml;

    #[test]
    fn a_non_ascii_document_longer_than_the_scan_window_is_rejected_not_a_panic() {
        // Pad with two-byte characters so byte 512 lands inside one. A raw
        // `str` slice panics here, and under `panic = "abort"` that ends the
        // process while opening a file.
        // One ASCII byte then two-byte characters: every boundary after it is
        // odd, so byte 512 falls inside a character.
        let padded = format!("a{}", "й".repeat(400));
        assert!(padded.len() > 512, "fixture must exceed the scan window");
        assert!(!padded.is_char_boundary(512), "byte 512 must split a char");

        assert!(!looks_like_hps_xml(&padded));
    }

    #[test]
    fn a_marker_inside_the_scan_window_still_matches_after_multi_byte_padding() {
        let mut text = "й".repeat(20);
        text.push_str("<HPS><Schema>");
        assert!(looks_like_hps_xml(&text));
    }

    #[test]
    fn a_marker_past_the_scan_window_is_not_matched() {
        let mut text = "a".repeat(600);
        text.push_str("<HPS><Schema>");
        assert!(!looks_like_hps_xml(&text));
    }
}
