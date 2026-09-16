use std::path::PathBuf;

/// A release build must never look in the build machine's checkout for a
/// sidecar: on the owner's PC that made the installed app run whatever
/// `target/release/buzz-acp.exe` the last `cargo build` left behind.
#[test]
fn release_builds_look_for_sidecars_only_next_to_the_executable() {
    let exe_dir = PathBuf::from("/app/bin");
    let cwd = PathBuf::from("/somewhere/else");
    assert_eq!(
        super::ordered_search_dirs(true, Some(exe_dir.clone()), Some(cwd.clone())),
        vec![exe_dir.clone()]
    );
    assert!(super::ordered_search_dirs(true, None, Some(cwd.clone())).is_empty());

    let debug = super::ordered_search_dirs(false, Some(exe_dir.clone()), Some(cwd.clone()));
    assert_eq!(
        debug.last(),
        Some(&exe_dir),
        "debug builds fall back to the exe dir last"
    );
    assert!(debug.contains(&cwd.join("target/debug")));
    assert!(debug.contains(&cwd.join("target/release")));
}
