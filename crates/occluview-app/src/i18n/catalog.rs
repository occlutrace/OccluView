//! Embedded Fluent catalogs, per-message English fallback, contract
//! validation, and pseudo-localization.
//!
//! Catalogs live in `crates/occluview-app/i18n/*.ftl`, are compiled into the
//! binary via `include_str!`, and are validated against `en` at test time:
//! parse errors, junk, duplicate keys, key/attribute/variable drift, and
//! unused keys all fail. At runtime a missing message, attribute, variable,
//! or whole catalog degrades atomically to English for that message —
//! never panic, never blank. A frame may mix languages on a partial
//! catalog; the fallback is per message, and the impossible case (the
//! key missing even in `en`) renders the diagnostic ⟦id⟧ marker.

#[cfg(test)]
use std::collections::{BTreeMap, BTreeSet, HashMap};

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
#[cfg(test)]
use fluent_syntax::ast;
use unic_langid::LanguageIdentifier;

use super::tags::FALLBACK_TAG;

/// Catalogs compiled into this binary (pilot + wave 1 Latin scripts;
/// CJK stays out until the font spike lands).
pub(crate) const EMBEDDED_TAGS: &[&str] = &["en", "ru", "de", "es", "fr", "it", "pt-BR"];
/// Pseudo-locale tag for layout testing. Never user-selectable.
#[cfg(test)]
pub(crate) const PSEUDO_TAG: &str = "qps-ploc";

const SOURCES: &[(&str, &str)] = &[
    ("en", include_str!("../../i18n/en.ftl")),
    ("ru", include_str!("../../i18n/ru.ftl")),
    ("de", include_str!("../../i18n/de.ftl")),
    ("es", include_str!("../../i18n/es.ftl")),
    ("fr", include_str!("../../i18n/fr.ftl")),
    ("it", include_str!("../../i18n/it.ftl")),
    ("pt-BR", include_str!("../../i18n/pt-BR.ftl")),
];

/// One compiled locale bundle.
pub(crate) struct Catalog {
    /// Diagnostic identity (also asserted by the en-wording lock tests).
    #[allow(dead_code)]
    tag: &'static str,
    bundle: FluentBundle<FluentResource>,
}

impl Catalog {
    /// The embedded tag this catalog was built for.
    #[cfg(test)]
    pub(crate) fn tag(&self) -> &'static str {
        self.tag
    }

    /// Build a catalog for an embedded tag. `Err` carries human-readable
    /// reasons (parse errors, bad language identifier, resource errors).
    pub(crate) fn build(tag: &'static str) -> Result<Self, String> {
        let (_, source) = SOURCES
            .iter()
            .find(|(known, _)| *known == tag)
            .ok_or_else(|| format!("no embedded catalog for '{tag}'"))?;
        Self::build_from_source(tag, source)
    }

    fn build_from_source(tag: &'static str, source: &str) -> Result<Self, String> {
        let langid: LanguageIdentifier = tag.parse().map_err(|_| format!("bad tag '{tag}'"))?;
        let resource = FluentResource::try_new(source.to_owned())
            .map_err(|(_, errors)| join_errors(&errors))?;
        let mut bundle = FluentBundle::new(vec![langid]);
        bundle
            .add_resource(resource)
            .map_err(|errors| join_errors(&errors))?;
        Ok(Self { tag, bundle })
    }

    /// Last-resort English catalog that renders markers for everything.
    /// Only used when the embedded English source itself is broken — a
    /// build/CI failure, never a shipped state.
    pub(crate) fn empty_fallback() -> Self {
        let langid: LanguageIdentifier = FALLBACK_TAG.parse().unwrap_or_default();
        Self {
            tag: FALLBACK_TAG,
            bundle: FluentBundle::new(vec![langid]),
        }
    }

    /// Pseudo-locale catalog generated from the English source: expanded,
    /// accented, bracketed. Used by layout tests to prove surfaces adapt.
    #[cfg(test)]
    pub(crate) fn pseudo() -> Result<Self, String> {
        let (_, source) = SOURCES
            .iter()
            .find(|(known, _)| *known == FALLBACK_TAG)
            .ok_or("no embedded English source")?;
        let expanded = pseudo_source(source);
        let langid: LanguageIdentifier = FALLBACK_TAG
            .parse()
            .map_err(|_| "bad fallback tag".to_owned())?;
        let resource =
            FluentResource::try_new(expanded).map_err(|(_, errors)| join_errors(&errors))?;
        let mut bundle = FluentBundle::new(vec![langid]);
        bundle
            .add_resource(resource)
            .map_err(|errors| join_errors(&errors))?;
        Ok(Self {
            tag: PSEUDO_TAG,
            bundle,
        })
    }

    /// Format a message id (`key` or `key.attribute`). `None` on any
    /// problem: missing message/attribute, missing variable, or formatter
    /// errors. The caller falls back to English for that message.
    pub(crate) fn format(&self, id: &str, args: Option<&FluentArgs<'_>>) -> Option<String> {
        let (head, attribute) = match id.split_once('.') {
            Some((head, attribute)) => (head, Some(attribute)),
            None => (id, None),
        };
        let message = self.bundle.get_message(head)?;
        let pattern = match attribute {
            Some(name) => message.get_attribute(name)?.value(),
            None => message.value()?,
        };
        let mut errors = Vec::new();
        let value = self.bundle.format_pattern(pattern, args, &mut errors);
        if errors.is_empty() {
            Some(value.into_owned())
        } else {
            None
        }
    }

    /// Plain-text lookup without arguments.
    #[cfg(test)]
    pub(crate) fn text(&self, id: &str) -> Option<String> {
        self.format(id, None)
    }
}

fn join_errors<T: std::fmt::Display>(errors: &[T]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Pseudo-localize an FTL source: expand plain-text runs, keep syntax
/// (message keys, variant keys, `{...}`, comments) byte-identical.
#[cfg(test)]
fn pseudo_source(source: &str) -> String {
    let mut out = String::with_capacity(source.len() + source.len() / 2);
    let mut in_placeable = 0_usize;
    for line in source.lines() {
        if line.trim_start().starts_with('#') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let (prefix, body) = split_pseudo_prefix(line, in_placeable);
        out.push_str(prefix);
        // `opened` restarts every line: text expands unless a placeable was
        // opened (and not yet closed) on THIS line. The carried depth only
        // tells whether we are inside a multiline select; it must not send
        // a freshly reopened `{ $var }` down the expansion path.
        let mut opened = false;
        let mut chunk = String::new();
        for ch in body.chars() {
            match ch {
                '{' => {
                    in_placeable += 1;
                    opened = true;
                    flush_pseudo_value(&mut out, &mut chunk);
                    out.push(ch);
                }
                '}' => {
                    in_placeable = in_placeable.saturating_sub(1);
                    opened = false;
                    out.push(ch);
                }
                _ if opened => out.push(ch),
                _ => chunk.push(ch),
            }
        }
        flush_pseudo_value(&mut out, &mut chunk);
        out.push('\n');
    }
    out
}

/// Split a line into (syntax prefix to keep, body to expand): `key = value`
/// keeps `key =`, select-variant lines (`[one]`/`*[other]`) keep the variant
/// key. Continuation lines inside a placeable expand whole.
#[cfg(test)]
fn split_pseudo_prefix(line: &str, in_placeable: usize) -> (&str, &str) {
    if in_placeable == 0 {
        if let Some(eq) = line.find('=') {
            let (key, rest) = line.split_at(eq + 1);
            return (key, rest);
        }
    }
    // Select-variant keys stay byte-identical at any depth; without this
    // the expansion would corrupt `[one]`/`*[other]` and break parsing.
    let trimmed = line.trim_start();
    if trimmed.starts_with('[') || trimmed.starts_with("*[") {
        if let Some(end) = trimmed.find(']') {
            let key_len = line.len() - trimmed.len() + end + 1;
            return line.split_at(key_len);
        }
    }
    ("", line)
}

#[cfg(test)]
fn flush_pseudo_value(out: &mut String, chunk: &mut String) {
    if chunk.trim().is_empty() {
        // Blank runs (indentation, empty values) pass through unbracketed.
        out.push_str(chunk);
        chunk.clear();
        return;
    }
    let expanded: String = chunk.chars().map(pseudo_char).collect();
    let padding = "〜".repeat(chunk.chars().filter(|ch| ch.is_alphabetic()).count() / 3);
    out.push('⟦');
    out.push_str(&expanded);
    out.push_str(&padding);
    out.push('⟧');
    chunk.clear();
}

#[cfg(test)]
fn pseudo_char(ch: char) -> char {
    match ch {
        'a' => 'á',
        'e' => 'é',
        'i' => 'í',
        'o' => 'ó',
        'u' => 'ú',
        'A' => 'Á',
        'E' => 'É',
        'O' => 'Ó',
        's' => 'š',
        'n' => 'ñ',
        _ => ch,
    }
}

/// Message contract: which attributes and variables each key exposes.
#[cfg(test)]
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Contract {
    entries: BTreeMap<String, ContractEntry>,
}

#[cfg(test)]
#[derive(Debug, Default, PartialEq, Eq)]
struct ContractEntry {
    has_value: bool,
    attributes: BTreeSet<String>,
    variables: BTreeSet<String>,
    /// One sorted variant-name set per select expression, in source
    /// order, across the value and attributes. Non-`ru` catalogs must
    /// mirror `en` exactly; `ru` must additionally carry one/few/many
    /// wherever `en` selects on a count. A flat string where `en`
    /// selects would silently drop pluralization.
    select_shapes: Vec<BTreeSet<String>>,
}

#[cfg(test)]
pub(crate) fn contract_of(source: &str) -> Result<Contract, Vec<String>> {
    let resource = match fluent_syntax::parser::parse(source) {
        Ok(resource) => resource,
        Err((_, errors)) => {
            return Err(errors.iter().map(ToString::to_string).collect());
        }
    };
    let mut contract = Contract::default();
    let mut problems = Vec::new();
    for entry in &resource.body {
        match entry {
            ast::Entry::Message(message) => {
                let id = message.id.name.to_owned();
                if contract.entries.contains_key(&id) {
                    problems.push(format!("duplicate key '{id}'"));
                    continue;
                }
                let mut variables = BTreeSet::new();
                let mut select_shapes = Vec::new();
                if let Some(pattern) = &message.value {
                    collect_pattern_vars(pattern, &mut variables);
                    collect_select_shapes_in_pattern(pattern, &mut select_shapes);
                }
                let mut attributes = BTreeSet::new();
                for attribute in &message.attributes {
                    attributes.insert(attribute.id.name.to_owned());
                    collect_pattern_vars(&attribute.value, &mut variables);
                    collect_select_shapes_in_pattern(&attribute.value, &mut select_shapes);
                }
                contract.entries.insert(
                    id,
                    ContractEntry {
                        has_value: message.value.is_some(),
                        attributes,
                        variables,
                        select_shapes,
                    },
                );
            }
            ast::Entry::Term(_) => {
                problems.push("terms are not part of the contract; use messages".to_owned());
            }
            ast::Entry::Junk { content } => {
                problems.push(format!("unparseable content: '{content}'"));
            }
            ast::Entry::Comment(_)
            | ast::Entry::GroupComment(_)
            | ast::Entry::ResourceComment(_) => {}
        }
    }
    if problems.is_empty() {
        Ok(contract)
    } else {
        Err(problems)
    }
}

#[cfg(test)]
fn collect_pattern_vars(pattern: &ast::Pattern<&str>, out: &mut BTreeSet<String>) {
    for element in &pattern.elements {
        if let ast::PatternElement::Placeable { expression } = element {
            collect_expression_vars(expression, out);
        }
    }
}

#[cfg(test)]
fn collect_expression_vars(expression: &ast::Expression<&str>, out: &mut BTreeSet<String>) {
    match expression {
        ast::Expression::Inline(inline) => collect_inline_vars(inline, out),
        ast::Expression::Select { selector, variants } => {
            collect_inline_vars(selector, out);
            for variant in variants {
                collect_pattern_vars(&variant.value, out);
            }
        }
    }
}

#[cfg(test)]
fn collect_select_shapes_in_pattern(pattern: &ast::Pattern<&str>, out: &mut Vec<BTreeSet<String>>) {
    for element in &pattern.elements {
        if let ast::PatternElement::Placeable { expression } = element {
            collect_select_shapes_in_expression(expression, out);
        }
    }
}

#[cfg(test)]
fn collect_select_shapes_in_expression(
    expression: &ast::Expression<&str>,
    out: &mut Vec<BTreeSet<String>>,
) {
    match expression {
        ast::Expression::Inline(inline) => collect_select_shapes_in_inline(inline, out),
        ast::Expression::Select { selector, variants } => {
            let mut names = BTreeSet::new();
            for variant in variants {
                match &variant.key {
                    ast::VariantKey::Identifier { name } => {
                        names.insert((*name).to_owned());
                    }
                    ast::VariantKey::NumberLiteral { value } => {
                        names.insert((*value).to_owned());
                    }
                }
            }
            out.push(names);
            collect_select_shapes_in_inline(selector, out);
            for variant in variants {
                collect_select_shapes_in_pattern(&variant.value, out);
            }
        }
    }
}

#[cfg(test)]
fn collect_select_shapes_in_inline(
    inline: &ast::InlineExpression<&str>,
    out: &mut Vec<BTreeSet<String>>,
) {
    match inline {
        ast::InlineExpression::FunctionReference { arguments, .. } => {
            for positional in &arguments.positional {
                collect_select_shapes_in_inline(positional, out);
            }
        }
        ast::InlineExpression::Placeable { expression } => {
            collect_select_shapes_in_expression(expression, out);
        }
        _ => {}
    }
}

#[cfg(test)]
fn collect_inline_vars(inline: &ast::InlineExpression<&str>, out: &mut BTreeSet<String>) {
    match inline {
        ast::InlineExpression::VariableReference { id } => {
            out.insert(id.name.to_owned());
        }
        ast::InlineExpression::FunctionReference { arguments, .. } => {
            for positional in &arguments.positional {
                collect_inline_vars(positional, out);
            }
        }
        ast::InlineExpression::Placeable { expression } => {
            collect_expression_vars(expression, out);
        }
        ast::InlineExpression::StringLiteral { .. }
        | ast::InlineExpression::NumberLiteral { .. }
        | ast::InlineExpression::MessageReference { .. }
        | ast::InlineExpression::TermReference { .. } => {}
    }
}

/// Russian must carry the full one/few/many set wherever English
/// selects on a count; every other locale must mirror `en` exactly.
#[cfg(test)]
fn select_shapes_match(tag: &str, base: &[BTreeSet<String>], entry: &[BTreeSet<String>]) -> bool {
    if base.len() != entry.len() {
        return false;
    }
    base.iter()
        .zip(entry.iter())
        .all(|(base_shape, entry_shape)| {
            if tag == "ru" && base_shape.contains("one") {
                ["one", "few", "many", "other"]
                    .into_iter()
                    .all(|required| entry_shape.contains(required))
            } else {
                entry_shape == base_shape
            }
        })
}

/// English key set for the code→catalog pin test: every id the UI
/// resolves must exist in `en` (a missing one renders the ⟦id⟧ marker
/// instead of failing loudly, so the test fails first).
///
/// Test-only helper: `expect` marks fixture bugs (a missing `en`
/// baseline), the same convention as the `mod tests` allows below.
#[cfg(test)]
#[allow(clippy::expect_used)]
pub(crate) fn embedded_en_keys() -> BTreeSet<String> {
    let (_, source) = SOURCES
        .iter()
        .find(|(tag, _)| *tag == FALLBACK_TAG)
        .expect("en source");
    contract_of(source)
        .expect("en parses")
        .entries
        .keys()
        .cloned()
        .collect()
}

/// Validate every embedded catalog against `en`. Returns human-readable
/// problems; empty means the contract holds. Fails on: parse errors, junk,
/// duplicate keys, missing/extra keys, attribute drift, variable drift.
#[cfg(test)]
pub(crate) fn validate_embedded() -> Vec<String> {
    let mut contracts: HashMap<&str, Contract> = HashMap::new();
    let mut problems = Vec::new();
    for (tag, source) in SOURCES {
        match contract_of(source) {
            Ok(contract) => {
                contracts.insert(tag, contract);
            }
            Err(errors) => {
                for error in errors {
                    problems.push(format!("{tag}: {error}"));
                }
            }
        }
    }
    let Some(baseline) = contracts.get(FALLBACK_TAG) else {
        problems.push("missing English baseline catalog".to_owned());
        return problems;
    };
    for (tag, contract) in &contracts {
        if *tag == FALLBACK_TAG {
            continue;
        }
        for key in baseline.entries.keys() {
            match contract.entries.get(key) {
                None => problems.push(format!("{tag}: missing key '{key}'")),
                Some(entry) => {
                    let base = &baseline.entries[key];
                    if entry.has_value != base.has_value {
                        problems.push(format!("{tag}: value presence drift on '{key}'"));
                    }
                    for attribute in base.attributes.difference(&entry.attributes) {
                        problems.push(format!("{tag}: missing attribute '{key}.{attribute}'"));
                    }
                    for attribute in entry.attributes.difference(&base.attributes) {
                        problems.push(format!("{tag}: unused attribute '{key}.{attribute}'"));
                    }
                    for variable in base.variables.difference(&entry.variables) {
                        problems.push(format!("{tag}: missing variable '${variable}' in '{key}'"));
                    }
                    for variable in entry.variables.difference(&base.variables) {
                        problems.push(format!("{tag}: unused variable '${variable}' in '{key}'"));
                    }
                    if !select_shapes_match(tag, &base.select_shapes, &entry.select_shapes) {
                        problems.push(format!(
                            "{tag}: select-shape drift on '{key}' (en {:?}, {tag} {:?})",
                            base.select_shapes, entry.select_shapes
                        ));
                    }
                }
            }
        }
        for key in contract.entries.keys() {
            if !baseline.entries.contains_key(key) {
                problems.push(format!("{tag}: unused key '{key}'"));
            }
        }
    }
    problems.sort();
    problems
}

/// Build a `FluentArgs` map from string pairs (enough for the seed keys;
/// clinical numbers use typed `FluentValue` at call sites).
pub(crate) fn args<'a>(pairs: &[(&'a str, &'a str)]) -> FluentArgs<'a> {
    let mut result = FluentArgs::new();
    for (key, value) in pairs {
        result.set(*key, FluentValue::from(*value));
    }
    result
}

/// A `usize` count as a plural operand. Saturates instead of wrapping;
/// real counts never approach the bound, and saturation keeps the lint
/// set (`cast_possible_wrap`) quiet without an `as` cast.
pub(crate) fn count(value: usize) -> FluentValue<'static> {
    FluentValue::from(i64::try_from(value).unwrap_or(i64::MAX))
}

/// Mixed string + count arguments for plural selects with data
/// (`export-layers-saved` and friends).
pub(crate) fn plural_args<'a>(
    strings: &[(&'a str, &'a str)],
    numbers: &[(&'a str, usize)],
) -> FluentArgs<'a> {
    let mut result = args(strings);
    for (key, value) in numbers {
        result.set(*key, count(*value));
    }
    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn embedded_catalogs_satisfy_the_contract() {
        assert_eq!(validate_embedded(), Vec::<String>::new());
    }

    #[test]
    fn keys_never_contain_dots() {
        // `Catalog::format` splits ids on the first `.` for `key.attribute`
        // lookup, so a dotted key could never resolve at runtime. The
        // contract validator compares keys opaquely and would not catch it.
        for (tag, source) in SOURCES {
            let contract = contract_of(source).expect("parses");
            for key in contract.entries.keys() {
                assert!(
                    !key.contains('.'),
                    "{tag}: dotted key '{key}' would never resolve"
                );
            }
        }
    }

    #[test]
    fn validator_catches_drift() {
        let base = "hello = Hello { $name }\n";
        let missing_var = "hello = Hallo\n";
        let extra_key = "hello = Hallo { $name }\nextra = Extra\n";
        let base_contract = contract_of(base).expect("base parses");
        let missing_contract = contract_of(missing_var).expect("parses");
        let extra_contract = contract_of(extra_key).expect("parses");
        // Dropped variable is visible as a set difference ...
        assert!(!base_contract.entries["hello"]
            .variables
            .difference(&missing_contract.entries["hello"].variables)
            .collect::<Vec<_>>()
            .is_empty());
        // ... and an unknown key is absent from the baseline.
        assert!(!base_contract.entries.contains_key("extra"));
        assert!(extra_contract.entries.contains_key("extra"));
    }

    #[test]
    fn validator_rejects_broken_sources() {
        assert!(contract_of("hello = Hello { $name \n").is_err());
        assert!(contract_of("dup = A\ndup = B\n").is_err());
    }

    #[test]
    fn validator_catches_flat_string_where_en_selects() {
        // Regression: ru repair toasts once shipped as flat strings while
        // en selected plurals. Variable sets matched, so only the select
        // count catches it.
        let base = contract_of(
            "n = { $count ->\n    [one] { $count } thing\n   *[other] { $count } things\n}\n",
        )
        .expect("parses");
        let flat = contract_of("n = Stuff: { $count }\n").expect("parses");
        assert_eq!(base.entries["n"].select_shapes.len(), 1);
        assert_eq!(flat.entries["n"].select_shapes.len(), 0);
        assert_eq!(base.entries["n"].variables, flat.entries["n"].variables);
    }

    #[test]
    fn validator_catches_variant_name_drift() {
        // Shape compares variant names, not just the select count: a `ru`
        // catalog missing `[many]` must fail even though counts match.
        let base = contract_of(
            "n = { $count ->\n    [one] { $count } thing\n   *[other] { $count } things\n}\n",
        )
        .expect("parses");
        let missing_many = contract_of(
            "n = { $count ->\n    [one] { $count } штука\n    [few] { $count } штуки\n   *[other] { $count } штук\n}\n",
        )
        .expect("parses");
        let full = contract_of(
            "n = { $count ->\n    [one] { $count } штука\n    [few] { $count } штуки\n    [many] { $count } штук\n   *[other] { $count } штук\n}\n",
        )
        .expect("parses");
        let base_shapes = &base.entries["n"].select_shapes;
        assert!(!select_shapes_match(
            "ru",
            base_shapes,
            &missing_many.entries["n"].select_shapes
        ));
        assert!(select_shapes_match(
            "ru",
            base_shapes,
            &full.entries["n"].select_shapes
        ));
        assert!(select_shapes_match(
            "de",
            base_shapes,
            &base.entries["n"].select_shapes
        ));
    }

    #[test]
    fn english_seed_formats_with_variables() {
        let catalog = Catalog::build("en").expect("en builds");
        assert_eq!(catalog.tag(), "en");
        assert_eq!(catalog.text("app-title").as_deref(), Some("OccluView"));
        let rendered = catalog
            .format("about-version", Some(&args(&[("version", "1.1.1")])))
            .expect("formats");
        // Fluent isolates interpolated variables with bidi marks
        // (U+2068/U+2069) by design — see the locale policy on RTL deferral.
        assert_eq!(rendered, "Version \u{2068}1.1.1\u{2069}");
    }

    #[test]
    fn missing_variable_yields_none_for_fallback() {
        let catalog = Catalog::build("en").expect("en builds");
        assert_eq!(catalog.format("about-version", None), None);
        assert_eq!(catalog.text("no-such-key"), None);
    }

    #[test]
    fn russian_plural_contract_is_expressible() {
        // Russian one/few/many/other via select on a numeric variable.
        let source = "found = { $count ->\n    [one] Найден { $count } контакт\n    [few] Найдено { $count } контакта\n    [many] Найдено { $count } контактов\n   *[other] Найдено { $count } контактов\n}\n";
        let contract = contract_of(source).expect("parses");
        assert!(contract.entries["found"].variables.contains("count"));
        assert_eq!(contract.entries["found"].select_shapes.len(), 1);
        assert!(contract.entries["found"].select_shapes[0].contains("many"));
        let resource = FluentResource::try_new(source.to_owned()).expect("resource");
        let langid: LanguageIdentifier = "ru".parse().expect("langid");
        let mut bundle = FluentBundle::new(vec![langid]);
        bundle.add_resource(resource).expect("add");
        let message = bundle.get_message("found").expect("message");
        let pattern = message.value().expect("value");
        // Interpolated numbers carry bidi isolation marks, so assert on
        // the surrounding word forms, not the digits.
        for (count, expected) in [
            // Trailing space: "Найден " is not a substring of "Найдено ".
            (1.0, "Найден "),
            (2.0, "контакта"),
            (5.0, "контактов"),
            (1.5, "контактов"),
        ] {
            let mut errors = Vec::new();
            let mut format_args = FluentArgs::new();
            format_args.set("count", FluentValue::from(count));
            let rendered = bundle.format_pattern(pattern, Some(&format_args), &mut errors);
            assert!(errors.is_empty());
            assert!(rendered.contains(expected), "{count} rendered {rendered}");
        }
    }

    #[test]
    fn pseudo_locale_expands_and_brackets() {
        let pseudo = Catalog::pseudo().expect("pseudo builds");
        let rendered = pseudo.text("settings-language-label").expect("renders");
        assert!(rendered.starts_with('⟦'));
        assert!(rendered.ends_with('⟧'));
        let plain = Catalog::build("en")
            .expect("en builds")
            .text("settings-language-label")
            .expect("renders");
        assert!(rendered.len() > plain.len());
        // Syntax survives: variables still format.
        let version = pseudo
            .format("about-version", Some(&args(&[("version", "9.9")])))
            .expect("formats");
        assert!(version.contains("9.9"));
    }

    /// Every English key renders in the pseudo-locale: bracketed, expanded,
    /// with all variables substituted. A key missing here means a surface
    /// that layout tests cannot exercise.
    #[test]
    fn pseudo_locale_covers_every_embedded_key() {
        // Pinned: adding a key without pseudo coverage must update this
        // number AND the loop below in the same change.
        const EXPECTED_EN_KEYS: usize = 619;
        let (_, source) = SOURCES
            .iter()
            .find(|(tag, _)| *tag == FALLBACK_TAG)
            .expect("en source");
        let contract = contract_of(source).expect("en parses");
        let pseudo = Catalog::pseudo().expect("pseudo builds");
        assert_eq!(
            contract.entries.len(),
            EXPECTED_EN_KEYS,
            "en key count moved; update EXPECTED_EN_KEYS and check pseudo coverage"
        );
        let mut failures = Vec::new();
        for (key, entry) in &contract.entries {
            let mut format_args = FluentArgs::new();
            for variable in &entry.variables {
                format_args.set(variable.as_str(), FluentValue::from("0"));
            }
            // Multiline selects bracket per variant line, so membership —
            // not a leading bracket — proves expansion happened.
            match pseudo.format(key, Some(&format_args)) {
                Some(rendered) if rendered.contains('⟦') => {}
                other => failures.push(format!("{key}: {other:?}")),
            }
        }
        assert!(failures.is_empty(), "pseudo gaps:\n{}", failures.join("\n"));
    }
}
