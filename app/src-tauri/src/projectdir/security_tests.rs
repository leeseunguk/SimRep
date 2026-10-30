//! Attack regressions exercised through the actual native project-directory API.
use super::super as projectdir;

#[test]
fn predictable_temporary_symlink_does_not_overwrite_outside_file() {
    let dir = std::env::temp_dir().join(format!("simrep-projectdir-attack-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.join("lens.simrep").to_string_lossy().into_owned();
    let data = projectdir::data_dir(&project).unwrap();
    std::fs::create_dir_all(data.join("ai")).unwrap();
    let sentinel = dir.join("outside.json");
    std::fs::write(&sentinel, b"KEEP").unwrap();
    std::os::unix::fs::symlink(&sentinel, data.join("ai/chat.json.tmp")).unwrap();
    let result = projectdir::write(&project, "ai/chat.json", b"SAFE");
    let outside = std::fs::read(&sentinel).unwrap();
    let destination_is_link = std::fs::symlink_metadata(data.join("ai/chat.json"))
        .is_ok_and(|m| m.file_type().is_symlink());
    std::fs::remove_dir_all(&dir).unwrap();
    result.unwrap();
    assert_eq!(outside, b"KEEP");
    assert!(!destination_is_link);
}

#[test]
fn sparse_oversized_read_is_refused_before_allocating() {
    let dir = std::env::temp_dir().join(format!("simrep-projectdir-size-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.join("lens.simrep").to_string_lossy().into_owned();
    let data = projectdir::data_dir(&project).unwrap();
    std::fs::create_dir_all(data.join("ai")).unwrap();
    for (name, limit) in [
        ("large.bin", projectdir::MAX_READ_BYTES),
        ("ai/large.json", projectdir::MAX_METADATA_READ_BYTES),
    ] {
        let file = std::fs::File::create(data.join(name)).unwrap();
        file.set_len(limit + 1).unwrap();
        assert!(projectdir::read(&project, name).is_err());
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn listing_entry_budget_includes_directories_and_ignored_files() {
    let dir = std::env::temp_dir().join(format!("simrep-projectdir-list-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.join("lens.simrep").to_string_lossy().into_owned();
    let data = projectdir::data_dir(&project).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    for i in 0..=projectdir::MAX_LIST_ENTRIES {
        let path = data.join(format!("{i}.tmp"));
        if i % 2 == 0 {
            std::fs::create_dir(path).unwrap();
        } else {
            std::fs::File::create(path).unwrap();
        }
    }
    let result = projectdir::list(&project, "");
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(result.is_err());
}

#[test]
fn listing_depth_budget_bounds_nested_projects() {
    let dir = std::env::temp_dir().join(format!("simrep-projectdir-depth-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.join("lens.simrep").to_string_lossy().into_owned();
    let mut nested = projectdir::data_dir(&project).unwrap();
    for _ in 0..=projectdir::MAX_LIST_DEPTH {
        nested.push("nested");
    }
    std::fs::create_dir_all(nested).unwrap();
    let result = projectdir::list(&project, "");
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(result.is_err());
}
