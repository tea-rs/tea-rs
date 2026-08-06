use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use tea_coding::ProjectAccess;
use tea_coding::resources::ResourceCatalog;

static ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn skill_metadata_is_eager_body_is_explicit_and_references_are_confined() {
    let root = std::env::temp_dir().join(format!(
        "coding-skills-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let workspace = root.join("workspace");
    let skill = root.join("global-skills/review");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("guide.md"), "guide").unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: review\ndescription: Review code safely\n---\nFollow guide.md.\n",
    )
    .unwrap();
    let catalog = ResourceCatalog::discover(
        &root,
        &workspace,
        ProjectAccess::Ignored,
        &[root.join("global-skills")],
        &[],
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        catalog.skill_metadata()[0].description(),
        "Review code safely"
    );
    let loaded = catalog.invoke_skill("/skill:review src/lib.rs").unwrap();
    assert_eq!(loaded.arguments(), "src/lib.rs");
    assert!(catalog.invoke_skill("/skill:reviewer").is_err());
    assert!(loaded.content().contains("guide.md"));
    assert_eq!(
        loaded.resolve_reference("guide.md").unwrap(),
        fs::canonicalize(skill.join("guide.md")).unwrap()
    );
    assert!(loaded.resolve_reference("../outside").is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn duplicate_skill_names_fail_deterministically() {
    let root = std::env::temp_dir().join(format!(
        "coding-skills-dup-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let workspace = root.join("workspace");
    for path in [
        root.join("skills/a"),
        root.join("skills/b"),
        workspace.clone(),
    ] {
        fs::create_dir_all(path).unwrap();
    }
    for dir in ["a", "b"] {
        fs::write(
            root.join(format!("skills/{dir}/SKILL.md")),
            "---\nname: same\ndescription: duplicate\n---\nbody\n",
        )
        .unwrap();
    }
    assert!(
        ResourceCatalog::discover(
            &root,
            &workspace,
            ProjectAccess::Ignored,
            &[root.join("skills")],
            &[],
            None,
            None
        )
        .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}
