use mcp::skills::SkillCatalog;

pub(crate) fn catalog() -> mcp::Result<SkillCatalog> {
    SkillCatalog::new()
        .with_skill(
            "smarthome/inspect-home",
            &[
                (
                    "SKILL.md",
                    include_bytes!("../skills/inspect-home/SKILL.md"),
                ),
                (
                    "references/queries.md",
                    include_bytes!("../skills/inspect-home/references/queries.md"),
                ),
            ],
        )?
        .with_skill(
            "smarthome/control-home",
            &[
                (
                    "SKILL.md",
                    include_bytes!("../skills/control-home/SKILL.md"),
                ),
                (
                    "references/controls.md",
                    include_bytes!("../skills/control-home/references/controls.md"),
                ),
            ],
        )?
        .with_skill(
            "smarthome/author-home-config",
            &[
                (
                    "SKILL.md",
                    include_bytes!("../skills/author-home-config/SKILL.md"),
                ),
                (
                    "references/authoring.md",
                    include_bytes!("../skills/author-home-config/references/authoring.md"),
                ),
            ],
        )?
        .with_skill(
            "smarthome/maintain-home-integration",
            &[
                (
                    "SKILL.md",
                    include_bytes!("../skills/maintain-home-integration/SKILL.md"),
                ),
                (
                    "references/lifecycle.md",
                    include_bytes!("../skills/maintain-home-integration/references/lifecycle.md"),
                ),
            ],
        )?
        .with_skill(
            "smarthome/inspect-thread-matter",
            &[
                (
                    "SKILL.md",
                    include_bytes!("../skills/inspect-thread-matter/SKILL.md"),
                ),
                (
                    "references/networks.md",
                    include_bytes!("../skills/inspect-thread-matter/references/networks.md"),
                ),
            ],
        )
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, path::Path};

    use super::*;

    fn collect_files(root: &Path, path: &Path, files: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect_files(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                );
            }
        }
    }

    #[test]
    fn explicit_catalog_includes_every_authored_file_and_no_payloads() {
        let catalog = catalog().unwrap();
        let listed = catalog.list(None).unwrap();
        let mut embedded = BTreeSet::new();
        for skill in listed.skills {
            let mcp::skills::McpSkillResources::Files(files) = skill.resources else {
                panic!("static manifest required");
            };
            for file in files {
                embedded.insert(
                    file.uri
                        .strip_prefix("skill://smarthome/")
                        .unwrap()
                        .to_owned(),
                );
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("skills");
        let mut authored = BTreeSet::new();
        collect_files(&root, &root, &mut authored);
        assert_eq!(authored, embedded);
        assert_eq!(authored.len(), 10);
        assert!(authored.iter().all(|path| path.ends_with(".md")));
    }
}
