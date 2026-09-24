use super::*;

#[test]
fn readme_lists_every_embedded_interface_language() {
    let readme = repo_source_file("../../README.md");
    for tag in i18n::catalog::EMBEDDED_TAGS {
        let name = i18n::endonym(tag);
        assert!(
            readme.contains(name),
            "README must list the embedded interface language {name} ({tag})"
        );
    }
}
