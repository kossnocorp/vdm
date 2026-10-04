use std::fs;
use vdm_fyi::VdmCheckout;

#[tokio::test]
async fn checkout_selects_files_directories_and_globs_without_vendoring() {
    let source = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(source.path()).unwrap();
    fs::create_dir(source.path().join("templates")).unwrap();
    fs::write(
        source.path().join("templates/hello.tera"),
        "Hello {{ name }}",
    )
    .unwrap();
    fs::write(source.path().join("data.bin"), [0, 255]).unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();
    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
        .unwrap();
    let url = reqwest::Url::from_file_path(source.path()).unwrap();
    for (selection, relative) in [
        ("templates", "hello.tera"),
        ("templates/hello.tera", ""),
        ("**/*.tera", "templates/hello.tera"),
        ("", "templates/hello.tera"),
    ] {
        let suffix = if selection.is_empty() {
            String::new()
        } else {
            format!(":{selection}")
        };
        let checkout = VdmCheckout::fetch(&format!("{url}{suffix}@{oid}"))
            .await
            .unwrap();
        let path = checkout.path().to_path_buf();
        let file = if relative.is_empty() {
            path.clone()
        } else {
            path.join(relative)
        };
        assert_eq!(fs::read_to_string(file).unwrap(), "Hello {{ name }}");
        assert!(!path.join(".git").exists());
        drop(checkout);
        assert!(!path.exists());
    }
    assert!(!source.path().join("vendor.toml").exists());
    assert!(!source.path().join("vendor.lock.toml").exists());
    assert!(!source.path().join("vendor").exists());
}
