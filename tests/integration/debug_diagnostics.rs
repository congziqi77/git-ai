use crate::repos::test_repo::{TestRepo, real_git_executable};
use git_ai::{daemon::DaemonConfig, diagnostic_sentinels::DEBUG_SELF_CHECK_DIR_NAME};
use std::fs;

#[test]
fn attribution_self_checks_do_not_timeout() {
    let mut repo = TestRepo::new();
    let slow_git = repo.test_home_path().join(if cfg!(windows) {
        "slow-git.exe"
    } else {
        "slow-git"
    });
    fs::copy(env!("CARGO_BIN_EXE_git-ai-test-git-shim"), &slow_git).unwrap();
    repo.patch_git_ai_config(|patch| {
        patch.git_path = Some(slow_git.to_string_lossy().into_owned());
        patch.disable_version_checks = Some(true);
    });
    let trace2_target =
        DaemonConfig::trace2_event_target_for_path(&repo.daemon_trace_socket_path());

    let report = repo
        .git_ai_with_env(
            &["debug", "--skip-trace2-checks"],
            &[
                ("GIT_TRACE2_EVENT", trace2_target.as_str()),
                ("GIT_TRACE2_EVENT_NESTING", "0"),
                ("GIT_AI_TEST_GIT_SHIM_TARGET", real_git_executable()),
                (
                    "GIT_AI_TEST_GIT_SHIM_FALLBACK_TARGET",
                    real_git_executable(),
                ),
                // Each Git command stays well below the three-second timeout,
                // but their cumulative delay exceeds the old shared deadline.
                ("GIT_AI_TEST_GIT_SHIM_DELAY_MS", "550"),
            ],
        )
        .expect("git-ai debug should complete");

    assert!(
        report.contains(&format!("configured git (program: {})", slow_git.display())),
        "configured self-check should use the delayed Git shim:\n{report}"
    );
    let passed_checks = report
        .lines()
        .filter(|line| line.contains("Attribution self-check: passed"))
        .count();
    assert_eq!(
        passed_checks, 2,
        "configured and terminal git attribution checks should pass:\n{report}"
    );
    let daemon_self_check_root = repo
        .daemon_home_path()
        .join(".git-ai")
        .join("internal")
        .join(DEBUG_SELF_CHECK_DIR_NAME);
    let expected_repo_prefix = format!("    repo: {}", daemon_self_check_root.display());
    assert_eq!(
        report.matches(&expected_repo_prefix).count(),
        2,
        "both self-check repositories should use the active daemon home:\n{report}"
    );
    for expected_line in [
        "line 1: untracked (expected untracked",
        "line 2: known_human (expected known_human",
        "line 3: ai (expected ai",
    ] {
        assert_eq!(
            report.matches(expected_line).count(),
            2,
            "both attribution checks should validate {expected_line}:\n{report}"
        );
    }
}

#[test]
fn debug_self_checks_isolate_inherited_git_environment() {
    let repo = TestRepo::new();
    let mut file = repo.filename("example.txt");
    file.set_contents(crate::lines!["Original line"]);
    repo.stage_all_and_commit("Original commit").unwrap();
    file.assert_committed_lines(crate::lines!["Original line"]);

    let trace2_target =
        DaemonConfig::trace2_event_target_for_path(&repo.daemon_trace_socket_path());
    repo.git_og(&["config", "--global", "trace2.eventTarget", &trace2_target])
        .unwrap();
    repo.git_og(&["config", "--global", "trace2.eventNesting", "0"])
        .unwrap();

    let git_dir = repo.path().join(".git");
    let config_before = fs::read(git_dir.join("config")).unwrap();
    let index_before = fs::read(git_dir.join("index")).unwrap();
    let refs_before = repo.git(&["show-ref"]).unwrap();
    let config_override = repo.test_home_path().join("alternate.gitconfig");
    let alternate_contents = "[user]\n\temail = outside@example.invalid\n";
    fs::write(&config_override, alternate_contents).unwrap();

    let report = repo
        .git_ai_with_env(
            &["debug"],
            &[
                ("GIT_DIR", git_dir.to_str().unwrap()),
                ("GIT_WORK_TREE", repo.path().to_str().unwrap()),
                ("GIT_INDEX_FILE", git_dir.join("index").to_str().unwrap()),
                ("GIT_CONFIG", config_override.to_str().unwrap()),
            ],
        )
        .unwrap();

    assert_eq!(fs::read(git_dir.join("config")).unwrap(), config_before);
    assert_eq!(fs::read(git_dir.join("index")).unwrap(), index_before);
    assert_eq!(repo.git(&["show-ref"]).unwrap(), refs_before);
    assert_eq!(
        fs::read_to_string(config_override).unwrap(),
        alternate_contents
    );
    assert!(
        report.contains("daemon is ready for debug self-checks"),
        "{report}"
    );
    assert_eq!(
        report.matches("Attribution self-check: passed").count(),
        2,
        "{report}"
    );
    assert_eq!(
        report.matches("Trace2 file self-check: passed").count(),
        2,
        "{report}"
    );
    assert_eq!(
        repo.git_og(&["config", "--global", "--get-all", "trace2.eventTarget"])
            .unwrap()
            .trim(),
        trace2_target
    );
}
