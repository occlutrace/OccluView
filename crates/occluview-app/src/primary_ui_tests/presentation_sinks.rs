//! Catalog-key coupling: every id the UI resolves through the catalogue must
//! exist in the embedded English catalogue, so a rename cannot reach the
//! operator as a `⟦key⟧` marker.
//!
//! The former "no inline English prose at a sink" scanner was a source-text
//! test over the crate's own `.rs` and was removed with the rest of that
//! class. The remaining check is cross-artifact: code call sites against the
//! shipped `en` catalogue.
//!
//! Test code never scans as production: every `#[cfg(test)]`-gated item is cut
//! by [`strip_test_regions`], which covers the non-standard module names a
//! plain `mod tests` search would miss.

// Filesystem discovery and fixture paths cannot fail in a checkout;
// `expect` marks those invariants (test-only convention, as elsewhere).
#![allow(clippy::expect_used)]

use super::collect_rust_source_files;

/// Test-only sources: the scanner itself, `*_tests.rs` fixtures,
/// `tests.rs` helpers (e.g. `bridge_split/tests.rs`) and anything under
/// a `tests/` directory.
fn is_scanner_or_fixture(relative: &str) -> bool {
    relative.starts_with("primary_ui_tests")
        || relative.ends_with("_tests.rs")
        || relative.ends_with("tests.rs")
        || relative.contains("/tests/")
        || relative.contains("hostile_tests")
}

/// Every runtime source under `src/`, test regions stripped. Sorted for
/// deterministic failure output.
fn discover_sources() -> Vec<(String, String)> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = Vec::new();
    collect_rust_source_files(&root, &mut paths).expect("walk src");
    let mut out = Vec::new();
    for path in paths {
        let relative = path
            .strip_prefix(&root)
            .expect("src prefix")
            .to_string_lossy()
            .into_owned();
        if is_scanner_or_fixture(&relative) {
            continue;
        }
        let content = std::fs::read_to_string(&path).expect("read src");
        out.push((relative, strip_test_regions(&content)));
    }
    out.sort();
    assert!(
        out.len() >= 130,
        "source discovery found only {} files; the walk is broken (losing src/app/ alone drops ~40)",
        out.len()
    );
    out
}

/// Cut every `#[cfg(test)]`-gated item: `mod name { ... }`, `mod name;`
/// (including `#[path]` declarations), functions, uses. Occurrences
/// inside `//` comments or `"..."` literals on the same line are not
/// attributes and are left alone.
fn strip_test_regions(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0_usize;
    while i < bytes.len() {
        let Some((hit, attr_end)) = find_cfg_test(source, i) else {
            out.push_str(&source[i..]);
            break;
        };
        out.push_str(&source[i..hit]);
        i = skip_test_item(source, attr_end);
    }
    out
}

/// Locate the next `#[cfg(...)]` attribute whose condition mentions the
/// standalone `test` configuration (`test`, `all(test, ...)`,
/// `any(test, ...)`). The marker must open the line (nothing but
/// whitespace before it): trailing `// cfg(test)` comments, block
/// comments and `"..."` literals can never match, so commentary about
/// the attribute cannot eat production code.
fn find_cfg_test(source: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        let offset = source[i..].find("#[cfg(")?;
        let hit = i + offset;
        let line_start = source[..hit].rfind('\n').map_or(0, |pos| pos + 1);
        let mut j = hit + "#[cfg(".len();
        let mut depth = 1_usize;
        while j < bytes.len() && depth > 0 {
            match bytes[j] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            j += 1;
        }
        let condition = &source[hit + "#[cfg(".len()..j.saturating_sub(1)];
        let mentions_test = condition
            .split(|cell: char| !(cell.is_ascii_alphanumeric() || cell == '_' || cell == '-'))
            .any(|token| token == "test");
        if mentions_test && source[line_start..hit].trim().is_empty() {
            return Some((hit, j));
        }
        i = j;
    }
    None
}

/// Index just past the gated item starting at `i` (after the marker
/// and any sibling attributes or comments).
fn skip_test_item(source: &str, mut i: usize) -> usize {
    let bytes = source.as_bytes();
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if source[i..].starts_with("#[") {
            i = source[i..].find(']').map_or(bytes.len(), |end| i + end + 1);
        } else if source[i..].starts_with("///")
            || source[i..].starts_with("//!")
            || source[i..].starts_with("//")
            || source[i..].starts_with("/*")
        {
            i = skip_comment(source, i);
        } else {
            break;
        }
    }
    // Skip visibility/qualifier prefixes so `pub(crate) mod` and
    // `pub(crate) use` dispatch like their bare forms.
    loop {
        let rest = &source[i..];
        if let Some(after_pub) = rest.strip_prefix("pub") {
            let trimmed = after_pub.trim_start_matches([' ', '\t', '\n', '\r']);
            let mut skip = rest.len() - trimmed.len();
            if let Some(parens) = trimmed.strip_prefix('(') {
                let mut depth = 1_usize;
                let mut end = parens.len();
                for (n, cell) in parens.bytes().enumerate() {
                    match cell {
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                end = n + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                skip += 1 + end;
            }
            i += skip;
        } else if ["unsafe ", "async ", "extern "]
            .iter()
            .any(|keyword| rest.starts_with(keyword))
        {
            i += rest.find(' ').map_or(rest.len(), |space| space + 1);
        } else {
            break;
        }
    }
    if source[i..].starts_with("mod ") {
        match first_semi_or_brace(source, i) {
            // `mod name;` (e.g. `#[path]` declarations): cut to `;`.
            (Some(semi), brace) if brace.is_none_or(|open| semi < open) => i + semi + 1,
            _ => skip_balanced(source, i),
        }
    } else if source[i..].starts_with("use ") {
        source[i..]
            .find(';')
            .map_or(bytes.len(), |semi| i + semi + 1)
    } else {
        match first_semi_or_brace(source, i) {
            (Some(semi), brace) if brace.is_none_or(|open| semi < open) => i + semi + 1,
            _ => skip_balanced(source, i),
        }
    }
}

/// First top-level `;` and `{` at or after `i`, skipping strings,
/// chars and comments. A `;` inside an array length can only mis-cut
/// test code, which fails loudly instead of hiding production.
fn first_semi_or_brace(source: &str, i: usize) -> (Option<usize>, Option<usize>) {
    let bytes = source.as_bytes();
    let mut semi = None;
    let mut brace = None;
    let mut j = i;
    while j < bytes.len() && (semi.is_none() || brace.is_none()) {
        match bytes[j] {
            b';' if semi.is_none() => semi = Some(j - i),
            b'{' if brace.is_none() => brace = Some(j - i),
            b'/' if bytes.get(j + 1) == Some(&b'/') => {
                j = skip_comment(source, j);
                continue;
            }
            b'/' if bytes.get(j + 1) == Some(&b'*') => {
                j = skip_comment(source, j);
                continue;
            }
            b'"' => j = skip_string(source, j),
            b'\'' => j = skip_char_or_lifetime(source, j),
            b'r' if matches!(bytes.get(j + 1), Some(b'"' | b'#')) => {
                j = skip_raw_string(source, j);
            }
            _ => {}
        }
        j += 1;
    }
    (semi, brace)
}

/// Skip a `//...` or `/*...*/` (nesting) comment at `i`; returns the
/// index past it.
fn skip_comment(source: &str, i: usize) -> usize {
    let bytes = source.as_bytes();
    if bytes.get(i + 1) == Some(&b'/') {
        return source[i..]
            .find('\n')
            .map_or(bytes.len(), |end| i + end + 1);
    }
    let mut depth = 1_usize;
    let mut j = i + 2;
    while j < bytes.len() && depth > 0 {
        match bytes[j] {
            b'/' if bytes.get(j + 1) == Some(&b'*') => {
                depth += 1;
                j += 1;
            }
            b'*' if bytes.get(j + 1) == Some(&b'/') => {
                depth -= 1;
                j += 1;
            }
            _ => {}
        }
        j += 1;
    }
    j
}

/// Skip a `"..."` literal at `i` (which points at the opening quote).
fn skip_string(source: &str, i: usize) -> usize {
    let bytes = source.as_bytes();
    let mut j = i + 1;
    while j < bytes.len() && bytes[j] != b'"' {
        if bytes[j] == b'\\' {
            j += 1;
        }
        j += 1;
    }
    j
}

/// Skip a `r#"..."#` literal at `i` (which points at the `r`).
fn skip_raw_string(source: &str, i: usize) -> usize {
    let bytes = source.as_bytes();
    let mut j = i + 1;
    let mut hashes = 0_usize;
    while bytes.get(j) == Some(&b'#') {
        hashes += 1;
        j += 1;
    }
    if bytes.get(j) != Some(&b'"') {
        return i;
    }
    j += 1;
    loop {
        if bytes.get(j) == Some(&b'"') && (0..hashes).all(|n| bytes.get(j + 1 + n) == Some(&b'#')) {
            return j + 1 + hashes;
        }
        if j >= bytes.len() {
            return bytes.len();
        }
        j += 1;
    }
}

/// Skip a `'x'` literal at `i`, or just the quote of a `'lifetime`.
fn skip_char_or_lifetime(source: &str, i: usize) -> usize {
    let bytes = source.as_bytes();
    let lifetime = matches!(bytes.get(i + 1), Some(cell) if cell.is_ascii_alphabetic() || *cell == b'_')
        && !matches!(bytes.get(i + 1), Some(b'\\'))
        && !matches!(bytes.get(i + 2), Some(b'\''));
    if lifetime {
        return i + 1;
    }
    let mut j = i + 1;
    while j < bytes.len() && bytes[j] != b'\'' && bytes[j] != b'\n' {
        if bytes[j] == b'\\' {
            j += 1;
        }
        j += 1;
    }
    j
}

/// Index just past the `{...}` block starting at or after `i`,
/// honouring nesting, comments, and all string forms.
fn skip_balanced(source: &str, i: usize) -> usize {
    let bytes = source.as_bytes();
    let mut j = i;
    while j < bytes.len() && bytes[j] != b'{' {
        j += 1;
    }
    let mut depth = 0_usize;
    while j < bytes.len() {
        match bytes[j] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            b'/' if bytes.get(j + 1) == Some(&b'/') || bytes.get(j + 1) == Some(&b'*') => {
                j = skip_comment(source, j);
                continue;
            }
            b'"' => j = skip_string(source, j),
            b'\'' => j = skip_char_or_lifetime(source, j),
            b'r' if matches!(bytes.get(j + 1), Some(b'"' | b'#')) => {
                j = skip_raw_string(source, j);
            }
            _ => {}
        }
        j += 1;
    }
    bytes.len()
}

/// Read the string literal starting at `bytes[i]` (which points at the
/// opening `"`). Returns the literal body and the index past the closing quote.
///
/// Backslash escapes are walked, not scanned: the old loop stopped at the first
/// `"` it saw, so a future `ui.label("a \"quoted\" word")` truncated the literal
/// and everything after the escape became invisible to BOTH catalog scanners —
/// a sink that silently stops being checked.
fn read_literal(bytes: &[u8], mut i: usize) -> (String, usize) {
    debug_assert_eq!(bytes[i], b'"');
    i += 1;
    let start = i;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => break,
            _ => i += 1,
        }
    }
    (
        String::from_utf8_lossy(&bytes[start..i]).into_owned(),
        i + 1,
    )
}

/// Find `needle` in `haystack` from `from`, skipping `//` line comments.
fn find_code(haystack: &str, needle: &str, mut from: usize) -> Option<usize> {
    while let Some(hit) = haystack[from..].find(needle) {
        let index = from + hit;
        let line_start = haystack[..index].rfind('\n').map_or(0, |pos| pos + 1);
        if haystack[line_start..index].trim_start().starts_with("//") {
            from = index + needle.len();
            continue;
        }
        return Some(index);
    }
    None
}

/// Every id the UI resolves must exist in `en`: a missing one renders
/// the ⟦id⟧ marker instead of failing loudly, so this test fails first.
/// (Catalog↔catalog drift fails the build via `build.rs`; this pins the
/// code→catalog direction. Help row/section keys are part of this same scan.)
#[test]
fn code_resolved_ids_exist_in_english() {
    let keys = crate::i18n::catalog::embedded_en_keys();
    assert!(
        keys.contains(crate::i18n::NATIVE_TITLE_KEY),
        "window title key missing from en"
    );
    let mut failures = Vec::new();
    for (name, production) in &discover_sources() {
        for opener in [
            ".tr(\"",
            ".text(\"",
            ".tr_with(\"",
            ".tr_plural(\"",
            "status_key: \"",
        ] {
            let mut from = 0_usize;
            while let Some(hit) = find_code(production, opener, from) {
                let bytes = production.as_bytes();
                let i = hit + opener.len() - 1;
                debug_assert_eq!(bytes[i], b'"');
                let (id, _) = read_literal(bytes, i);
                if !keys.contains(&id) {
                    failures.push(format!("{name}: unknown key {id:?}"));
                }
                from = hit + opener.len();
            }
        }
        // Multi-line call shape: `.tr(\n"key"`.
        for opener in [".tr(", ".text(", ".tr_with(", ".tr_plural("] {
            let mut from = 0_usize;
            while let Some(hit) = find_code(production, opener, from) {
                let bytes = production.as_bytes();
                let mut i = hit + opener.len();
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if bytes.get(i) == Some(&b'"') {
                    let (id, _) = read_literal(bytes, i);
                    if !keys.contains(&id) {
                        failures.push(format!("{name}: unknown key {id:?}"));
                    }
                }
                from = hit + opener.len();
            }
        }
    }
    assert!(
        failures.is_empty(),
        "code references keys missing from en:\n{}",
        failures.join("\n")
    );
}
