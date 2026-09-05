use std::fs;

use oxc_checker::{CheckOptions, check};

#[test]
fn checks_arkts_1_1_roots_and_component_members() {
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("Index.ets");
    fs::write(
        &source,
        r#"
@Component
struct Demo {
  count: number = "bad";

  value(): string {
    return 1;
  }
}

const bad: string = 1;
"#,
    )
    .unwrap();

    let result = check(CheckOptions {
        project_root: project.path().to_path_buf(),
        root_files: vec![source],
        tsconfig_path: None,
        module_paths: Vec::new(),
        aliases: Vec::new(),
        strict_null_checks: false,
        skip_lib_check: true,
        report_isolated_declaration_diagnostics: false,
    })
    .unwrap();

    let messages = result
        .files
        .iter()
        .flat_map(|file| file.diagnostics.iter())
        .map(|diagnostic| &*diagnostic.message)
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 3, "{messages:#?}");
    assert!(
        messages.iter().all(|message| message.contains("is not assignable to type")),
        "{messages:#?}"
    );
}
