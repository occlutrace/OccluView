//! Regression inventory: user-visible presentation sinks must route
//! through the catalogs, never inline English.
//!
//! Targeted source-contract scanner, not an AST and not a repo-wide
//! English regex: exact sink shapes over discovered sources. New
//! modules are picked up automatically — a localized presentation
//! surface cannot silently escape the inventory (the discovery count
//! assertion fails loudly if the walk itself breaks).
//!
//! Test code never scans as production: every `#[cfg(test)]`-gated
//! item (modules in both declaration forms, functions, uses) is cut by
//! [`strip_test_regions`], which covers the non-standard module names
//! that [`production_source`] misses.
//!
//! Deliberate non-sinks, documented so the next reader does not
//! "fix" them:
//! - Help section/row literals render via `key`; the English stays as
//!   source-of-truth pinned by `english_catalog_matches_source_wording`.
//! - `Window::new` stable IDs under `title_bar(false)` never paint.
//! - Shortcut/gesture tokens without spaces (`Ctrl+O`, `LMB`), product
//!   names, file formats and units are invariant vocabulary, not prose.
//!   (Limitation: a spaced invariant that ever appears inline, like a
//!   version string, would trip `is_prose` — none does today; all live
//!   in catalogs or fixtures.)
//! - Support surfaces stay locale-neutral English by design and are
//!   pinned by comments, not keys: `AppErrorDialog.details` (see
//!   `state.rs`), `repair_report::copy_details`, update failure messages
//!   and upstream release notes (`update_notice.rs`), redaction tokens
//!   (`app_loading.rs`), ASCII default filename stems
//!   (`app_mesh_export.rs`). Titles/summaries around them are cataloged.
//! - Paint-level text (`painter.text`, chips, custom buttons) and
//!   `RichText::new(variable)` carry already-localized or raw-payload
//!   values the scanner cannot judge statically; they were covered by
//!   the manual audit and stay out of the rules on purpose.

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

/// Single-word UI prose that must never appear as a whole literal at a
/// presentation sink (multi-word prose is caught by the space rule).
const BARE_PROSE: &[&str] = &[
    "Recent",
    "Close",
    "Open",
    "Save",
    "Cancel",
    "Delete",
    "Remove",
    "Clear",
    "Apply",
    "Retry",
    "Undo",
    "Redo",
    "Back",
    "Next",
    "Done",
    "Loading",
    "Empty",
    "Hidden",
    "Visible",
    "Selected",
    "Failed",
    "Missing",
    "Restored",
    "Translucent",
];

/// True when `text` reads as prose: ASCII letters around a space, or a
/// bare single-word UI term. Tokens without spaces (shortcuts, product
/// names, formats, units, IDs) are invariant, not prose.
fn is_prose(literal: &str) -> bool {
    let has_spaced_letters = literal
        .split(' ')
        .any(|word| word.bytes().filter(u8::is_ascii_alphabetic).count() >= 2)
        && literal.contains(' ');
    has_spaced_letters || BARE_PROSE.contains(&literal)
}

/// Read the string literal starting at `bytes[i]` (which points at the
/// opening `"`). Returns the literal body and the index past the
/// closing quote. No escape sequences occur in the scanned sinks.
fn read_literal(bytes: &[u8], mut i: usize) -> (String, usize) {
    debug_assert_eq!(bytes[i], b'"');
    i += 1;
    let start = i;
    while i < bytes.len() && bytes[i] != b'"' {
        i += 1;
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

/// Strip `{...}` placeholders; what remains of a pure-data format is
/// punctuation and numbers, never prose words.
fn deplaceholdered(format: &str) -> String {
    let mut out = String::new();
    let mut depth = 0_usize;
    for cell in format.chars() {
        match cell {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(cell),
            _ => {}
        }
    }
    out
}

fn snippet(source: &str, index: usize) -> String {
    let start = source[..index].rfind('\n').map_or(0, |pos| pos + 1);
    let end = source[index..]
        .find('\n')
        .map_or(source.len(), |pos| index + pos);
    source[start..end].trim().to_owned()
}

#[test]
fn presentation_sinks_route_through_catalogs() {
    let mut failures = Vec::new();
    for (name, production) in &discover_sources() {
        // 1-2. Status sinks: direct literals and prose-bearing format!.
        for sink in ["status = Some(", "status_message = Some("] {
            let mut from = 0_usize;
            while let Some(hit) = find_code(production, sink, from) {
                let mut i = hit + sink.len();
                let bytes = production.as_bytes();
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if bytes.get(i) == Some(&b'"') {
                    let (literal, _) = read_literal(bytes, i);
                    if is_prose(&literal) {
                        failures.push(format!(
                            "{name}: literal status: {}",
                            snippet(production, hit)
                        ));
                    }
                } else if production[i..].starts_with("format!(") {
                    let mut j = i + "format!(".len();
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if bytes.get(j) == Some(&b'"') {
                        let (format, _) = read_literal(bytes, j);
                        if is_prose(&deplaceholdered(&format)) {
                            failures.push(format!(
                                "{name}: prose format! status: {}",
                                snippet(production, hit)
                            ));
                        }
                    }
                }
                from = hit + sink.len();
            }
        }
        // 3. Localizer-wrapper reasons must arrive pre-localized.
        for sink in ["forget_align_fit(\"", "invalidate_deviation_map(\""] {
            let mut from = 0_usize;
            while let Some(hit) = find_code(production, sink, from) {
                failures.push(format!("{name}: raw reason: {}", snippet(production, hit)));
                from = hit + sink.len();
            }
        }
        // 4-5. Widget constructors with prose literals.
        for sink in ["Button::new(\"", "button(\""] {
            let mut from = 0_usize;
            while let Some(hit) = find_code(production, sink, from) {
                let (literal, _) = read_literal(production.as_bytes(), hit + sink.len() - 1);
                if !literal.is_empty() && is_prose(&literal) {
                    failures.push(format!(
                        "{name}: prose button: {}",
                        snippet(production, hit)
                    ));
                }
                from = hit + sink.len();
            }
        }
        // 6. Labels and headings with prose literals.
        for sink in ["ui.label(\"", "ui.heading(\"", "RichText::new(\""] {
            let mut from = 0_usize;
            while let Some(hit) = find_code(production, sink, from) {
                let (literal, _) = read_literal(production.as_bytes(), hit + sink.len() - 1);
                if !literal.is_empty() && is_prose(&literal) {
                    failures.push(format!("{name}: prose label: {}", snippet(production, hit)));
                }
                from = hit + sink.len();
            }
        }
        // 7. Window titles: prose is fine only for stable IDs and
        // unpainted title bars.
        let mut from = 0_usize;
        while let Some(hit) = find_code(production, "Window::new(\"", from) {
            let (literal, _) =
                read_literal(production.as_bytes(), hit + "Window::new(\"".len() - 1);
            let tail = &production[hit..production.len().min(hit + 800)];
            let unpainted = tail.contains("title_bar(false)");
            if is_prose(&literal) && !literal.starts_with("occluview") && !unpainted {
                failures.push(format!(
                    "{name}: prose window title: {}",
                    snippet(production, hit)
                ));
            }
            from = hit + "Window::new(\"".len();
        }
        // 8. Display-name fallbacks: a literal default for an operator-
        // facing name is presentation English (route via a key).
        for sink in ["map_or_else(|| \"", "unwrap_or_else(|| \"", "unwrap_or(\""] {
            let mut from = 0_usize;
            while let Some(hit) = find_code(production, sink, from) {
                let (literal, _) = read_literal(production.as_bytes(), hit + sink.len() - 1);
                if !literal.is_empty() && is_prose(&literal) {
                    failures.push(format!(
                        "{name}: prose fallback name: {}",
                        snippet(production, hit)
                    ));
                }
                from = hit + sink.len();
            }
        }
        // 9. Capital-L `Layer` display fallbacks (`format!("Layer {}")`):
        // the ASCII filename stem helper uses lowercase `layer-`; a
        // capital `Layer` display literal is an unkeyed operator-visible
        // name (route via `layer-unnamed`). Support `details:` blocks
        // stay out by design (documented above).
        let mut from = 0_usize;
        while let Some(hit) = find_code(production, "format!(\"Layer {", from) {
            failures.push(format!(
                "{name}: unkeyed Layer fallback: {}",
                snippet(production, hit)
            ));
            from = hit + "format!(\"Layer {".len();
        }
    }
    assert!(
        failures.is_empty(),
        "presentation English outside the catalogs:\n{}",
        failures.join("\n")
    );
}

/// Every id the UI resolves must exist in `en`: a missing one renders
/// the ⟦id⟧ marker instead of failing loudly, so this test fails first.
/// (Catalog↔catalog drift fails the build via `build.rs`; this pins the
/// code→catalog direction. Help row/section keys are pinned separately
/// by `english_catalog_matches_source_wording`.)
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
