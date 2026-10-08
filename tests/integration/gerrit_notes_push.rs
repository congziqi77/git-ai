use crate::repos::test_file::ExpectedLineExt;
use crate::repos::test_repo::TestRepo;
use serde_json::json;
use std::fs;

#[test]
fn gerrit_notes_push_exports_content_without_local_history() {
    let (mirror, upstream) = TestRepo::new_with_remote();
    mirror
        .git_og(&["config", "remote.origin.git-ai-notes-push", "gerrit"])
        .unwrap();

    fs::write(mirror.path().join("first.txt"), "first\n").unwrap();
    let first = mirror.stage_all_and_commit("first").unwrap();
    mirror
        .filename("first.txt")
        .assert_committed_lines(lines!["first".unattributed_human()]);
    let local_tip = mirror.git_og(&["rev-parse", "refs/notes/ai"]).unwrap();
    let first_blob = mirror
        .git_og(&["notes", "--ref=ai", "list", &first.commit_sha])
        .unwrap();

    let request = json!({"remote_name": "origin"}).to_string();
    mirror
        .git_ai(&["push-authorship-notes", "--json", &request])
        .unwrap();

    let first_remote_tip = upstream.git_og(&["rev-parse", "refs/notes/ai"]).unwrap();
    assert_ne!(first_remote_tip.trim(), local_tip.trim());
    assert_eq!(
        upstream
            .git_og(&["rev-list", "--count", "refs/notes/ai"])
            .unwrap()
            .trim(),
        "1"
    );
    assert_eq!(
        upstream
            .git_og(&["log", "-1", "--format=%an <%ae>|%cn <%ce>", "refs/notes/ai"])
            .unwrap()
            .trim(),
        "Test User <test@example.com>|Test User <test@example.com>"
    );
    assert_eq!(
        upstream
            .git_og(&["notes", "--ref=ai", "list", &first.commit_sha])
            .unwrap()
            .trim(),
        first_blob.trim()
    );

    mirror
        .git_ai(&["push-authorship-notes", "--json", &request])
        .unwrap();
    assert_eq!(
        upstream.git_og(&["rev-parse", "refs/notes/ai"]).unwrap(),
        first_remote_tip
    );

    fs::write(mirror.path().join("second.txt"), "second\n").unwrap();
    let second = mirror.stage_all_and_commit("second").unwrap();
    mirror
        .filename("first.txt")
        .assert_committed_lines(lines!["first".unattributed_human()]);
    mirror
        .filename("second.txt")
        .assert_committed_lines(lines!["second".unattributed_human()]);
    mirror
        .git_ai(&["push-authorship-notes", "--json", &request])
        .unwrap();

    assert_eq!(
        upstream
            .git_og(&["rev-list", "--count", "refs/notes/ai"])
            .unwrap()
            .trim(),
        "2"
    );
    assert_eq!(
        upstream
            .git_og(&["rev-parse", "refs/notes/ai^"])
            .unwrap()
            .trim(),
        first_remote_tip.trim()
    );
    for commit in [&first, &second] {
        assert_eq!(
            upstream
                .git_og(&["notes", "--ref=ai", "list", &commit.commit_sha])
                .unwrap(),
            mirror
                .git_og(&["notes", "--ref=ai", "list", &commit.commit_sha])
                .unwrap()
        );
    }
}

#[test]
fn default_notes_push_preserves_existing_history() {
    let (mirror, upstream) = TestRepo::new_with_remote();
    fs::write(mirror.path().join("default.txt"), "default\n").unwrap();
    mirror.stage_all_and_commit("default").unwrap();
    mirror
        .filename("default.txt")
        .assert_committed_lines(lines!["default".unattributed_human()]);

    let local_tip = mirror.git_og(&["rev-parse", "refs/notes/ai"]).unwrap();
    let request = json!({"remote_name": "origin"}).to_string();
    mirror
        .git_ai(&["push-authorship-notes", "--json", &request])
        .unwrap();
    assert_eq!(
        upstream.git_og(&["rev-parse", "refs/notes/ai"]).unwrap(),
        local_tip
    );
}

#[test]
fn gerrit_setting_only_applies_to_selected_remote() {
    let (mirror, gerrit) = TestRepo::new_with_remote();
    let gitlab = TestRepo::new_bare();
    mirror
        .git_og(&["remote", "add", "gitlab", gitlab.path().to_str().unwrap()])
        .unwrap();
    mirror
        .git_og(&["config", "remote.origin.git-ai-notes-push", "gerrit"])
        .unwrap();
    fs::write(mirror.path().join("remote.txt"), "remote\n").unwrap();
    mirror.stage_all_and_commit("remote-specific mode").unwrap();
    mirror
        .filename("remote.txt")
        .assert_committed_lines(lines!["remote".unattributed_human()]);

    let local_tip = mirror.git_og(&["rev-parse", "refs/notes/ai"]).unwrap();
    for remote_name in ["origin", "gitlab"] {
        let request = json!({"remote_name": remote_name}).to_string();
        mirror
            .git_ai(&["push-authorship-notes", "--json", &request])
            .unwrap();
    }

    assert_ne!(
        gerrit.git_og(&["rev-parse", "refs/notes/ai"]).unwrap(),
        local_tip
    );
    assert_eq!(
        gitlab.git_og(&["rev-parse", "refs/notes/ai"]).unwrap(),
        local_tip
    );
}

#[test]
fn gerrit_notes_push_preserves_remote_only_note() {
    let (mirror, upstream) = TestRepo::new_with_remote();
    mirror
        .git_og(&["config", "remote.origin.git-ai-notes-push", "gerrit"])
        .unwrap();

    fs::write(mirror.path().join("local.txt"), "local\n").unwrap();
    let local = mirror.stage_all_and_commit("local").unwrap();
    mirror
        .filename("local.txt")
        .assert_committed_lines(lines!["local".unattributed_human()]);
    fs::write(mirror.path().join("remote.txt"), "remote\n").unwrap();
    let remote = mirror.stage_all_and_commit("remote").unwrap();
    mirror
        .filename("local.txt")
        .assert_committed_lines(lines!["local".unattributed_human()]);
    mirror
        .filename("remote.txt")
        .assert_committed_lines(lines!["remote".unattributed_human()]);
    mirror.git_og(&["push", "origin", "main"]).unwrap();
    mirror
        .git_og(&["notes", "--ref=ai", "remove", &remote.commit_sha])
        .unwrap();

    upstream
        .git_og(&["config", "user.name", "Remote User"])
        .unwrap();
    upstream
        .git_og(&["config", "user.email", "remote@example.com"])
        .unwrap();
    upstream
        .git_og(&[
            "notes",
            "--ref=ai",
            "add",
            "-m",
            "remote note",
            &remote.commit_sha,
        ])
        .unwrap();
    let previous_remote_tip = upstream.git_og(&["rev-parse", "refs/notes/ai"]).unwrap();
    let remote_blob = upstream
        .git_og(&["notes", "--ref=ai", "list", &remote.commit_sha])
        .unwrap();

    let request = json!({"remote_name": "origin"}).to_string();
    mirror
        .git_ai(&["push-authorship-notes", "--json", &request])
        .unwrap();

    assert_eq!(
        upstream
            .git_og(&["rev-parse", "refs/notes/ai^"])
            .unwrap()
            .trim(),
        previous_remote_tip.trim()
    );
    assert_eq!(
        upstream
            .git_og(&["notes", "--ref=ai", "list", &remote.commit_sha])
            .unwrap(),
        remote_blob
    );
    assert_eq!(
        upstream
            .git_og(&["notes", "--ref=ai", "list", &local.commit_sha])
            .unwrap(),
        mirror
            .git_og(&["notes", "--ref=ai", "list", &local.commit_sha])
            .unwrap()
    );
}

#[test]
fn gerrit_notes_push_reports_remote_fetch_failure() {
    let (mirror, _upstream) = TestRepo::new_with_remote();
    mirror
        .git_og(&["config", "remote.origin.git-ai-notes-push", "gerrit"])
        .unwrap();
    let missing_remote = mirror.path().join("missing-remote.git");
    mirror
        .git_og(&[
            "remote",
            "set-url",
            "origin",
            missing_remote.to_str().unwrap(),
        ])
        .unwrap();
    fs::write(mirror.path().join("fetch.txt"), "fetch\n").unwrap();
    mirror.stage_all_and_commit("fetch").unwrap();
    mirror
        .filename("fetch.txt")
        .assert_committed_lines(lines!["fetch".unattributed_human()]);

    let request = json!({"remote_name": "origin"}).to_string();
    let error = mirror
        .git_ai(&["push-authorship-notes", "--json", &request])
        .expect_err("remote fetch failure must stop the notes push");
    assert!(error.contains("push_authorship_notes failed"), "{error}");
    assert!(error.contains("missing-remote.git"), "{error}");
}

#[test]
fn regular_branch_push_triggers_gerrit_notes_export() {
    let (mirror, upstream) = TestRepo::new_with_remote();
    mirror
        .git_og(&["config", "remote.origin.git-ai-notes-push", "gerrit"])
        .unwrap();
    fs::write(mirror.path().join("branch.txt"), "branch\n").unwrap();
    let commit = mirror.stage_all_and_commit("branch").unwrap();
    mirror
        .filename("branch.txt")
        .assert_committed_lines(lines!["branch".unattributed_human()]);

    mirror.git(&["push", "origin", "main"]).unwrap();
    mirror.sync_daemon();

    assert_eq!(
        upstream
            .git_og(&["rev-list", "--count", "refs/notes/ai"])
            .unwrap()
            .trim(),
        "1"
    );
    assert_eq!(
        upstream
            .git_og(&["log", "-1", "--format=%ae|%ce", "refs/notes/ai"])
            .unwrap()
            .trim(),
        "test@example.com|test@example.com"
    );
    assert!(
        upstream
            .git_og(&["notes", "--ref=ai", "list", &commit.commit_sha])
            .is_ok()
    );
}
