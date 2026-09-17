#[macro_use]
#[path = "integration/repos/mod.rs"]
mod repos;

use repos::test_file::ExpectedLineExt;
use repos::test_repo::TestRepo;
use serde_json::json;
use std::fs;

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for process evidence"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(unix)]
#[test]
fn synchronized_notes_serializes_real_fetches_across_worktrees() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};
    let (repo, _remote) = TestRepo::new_with_remote();
    let peer_dir = tempfile::tempdir().unwrap();
    let peer = peer_dir.path().join("peer");
    repo.git(&[
        "worktree",
        "add",
        "-b",
        "notes-peer",
        peer.to_str().unwrap(),
    ])
    .unwrap();
    let gate = repo.path().join("fetch.fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&gate)
            .status()
            .unwrap()
            .success()
    );
    let entries = repo.path().join("fetch.entries");
    let helper = repo.path().join("upload-pack-gate.sh");
    fs::write(&helper, format!("#!/bin/sh\nset -eu\nprintf 'entered\\n' >> '{}'\nread permission < '{}'\nexec '{}' upload-pack \"$@\"\n",
        entries.display(), gate.display(), repos::test_repo::real_git_executable())).unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    repo.git_og(&[
        "config",
        "remote.origin.uploadpack",
        helper.to_str().unwrap(),
    ])
    .unwrap();
    let args = ["fetch-notes", "origin", "--json", "--synchronized"];
    let mut first = repo
        .git_ai_command_without_pre_sync_for_test(&args, &[])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_until(|| entries.exists());
    let second = repo
        .git_ai_command_without_pre_sync_for_test(&args, &[])
        .current_dir(&peer)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let log = repo.daemon_home_path().join(".git-ai/internal/daemon");
    wait_until(|| {
        fs::read_dir(&log)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|e| fs::read_to_string(e.path()).ok())
            .any(|s| {
                s.lines()
                    .filter(|line| {
                        line.contains("notes sync requested")
                            && (line.contains(repo.path().to_str().unwrap())
                                || line.contains(peer.to_str().unwrap()))
                    })
                    .count()
                    >= 2
            })
    });
    assert_eq!(fs::read_to_string(&entries).unwrap().lines().count(), 1);
    fs::OpenOptions::new()
        .write(true)
        .open(&gate)
        .unwrap()
        .write_all(b"continue\n")
        .unwrap();
    wait_until(|| fs::read_to_string(&entries).unwrap().lines().count() == 2);
    wait_until(|| first.try_wait().unwrap().is_some());
    fs::OpenOptions::new()
        .write(true)
        .open(&gate)
        .unwrap()
        .write_all(b"continue\n")
        .unwrap();
    for output in [
        first.wait_with_output().unwrap(),
        second.wait_with_output().unwrap(),
    ] {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let data: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(data["synchronized"], true);
    }
}

#[cfg(unix)]
#[test]
fn synchronized_notes_timeout_terminates_remote_helper_and_releases_writer() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};
    let (repo, _remote) = TestRepo::new_with_remote();
    let gate = repo.path().join("timeout.fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&gate)
            .status()
            .unwrap()
            .success()
    );
    let pid_file = repo.path().join("upload-pack.pid");
    let helper = repo.path().join("blocked-upload-pack.sh");
    fs::write(&helper, format!("#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nread permission < '{}'\nexec '{}' upload-pack \"$@\"\n",
        pid_file.display(), gate.display(), repos::test_repo::real_git_executable())).unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    repo.git_og(&[
        "config",
        "remote.origin.uploadpack",
        helper.to_str().unwrap(),
    ])
    .unwrap();
    let started = std::time::Instant::now();
    let output = repo
        .git_ai_command_without_pre_sync_for_test(
            &["fetch-notes", "origin", "--json", "--synchronized"],
            &[],
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["status"], "notes_sync_timeout");
    assert!(started.elapsed() < std::time::Duration::from_secs(36));
    let pid: i32 = fs::read_to_string(pid_file).unwrap().parse().unwrap();
    wait_until(|| unsafe { libc::kill(pid, 0) } != 0);
    repo.git_og(&["config", "--unset", "remote.origin.uploadpack"])
        .unwrap();
    repo.git_ai_without_pre_sync_for_test(&["fetch-notes", "origin", "--json", "--synchronized"])
        .unwrap();
}

#[test]
fn synchronized_notes_restores_real_remote_authorship() {
    let (repo, _remote) = TestRepo::new_with_remote();
    let before = json!({"type": "human", "repo_working_dir": repo.path(),
        "will_edit_filepaths": ["note.txt"]});
    repo.git_ai(&[
        "checkpoint",
        "agent-v1",
        "--hook-input",
        &before.to_string(),
    ])
    .unwrap();
    fs::write(repo.path().join("note.txt"), "AI authored line\n").unwrap();
    let after = json!({"type": "ai_agent", "repo_working_dir": repo.path(),
        "edited_filepaths": ["note.txt"], "agent_name": "codex",
        "model": "gpt-6", "conversation_id": "notes-sync-integration"});
    repo.git_ai(&["checkpoint", "agent-v1", "--hook-input", &after.to_string()])
        .unwrap();
    repo.stage_all_and_commit("AI contribution").unwrap();
    repo.filename("note.txt")
        .assert_committed_lines(lines!["AI authored line".ai()]);
    let sha = repo
        .git_og(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();
    let expected = repo.read_authorship_note(&sha).unwrap();
    repo.git_og(&["push", "origin", "HEAD:refs/heads/main", "refs/notes/ai"])
        .unwrap();
    repo.git_og(&["update-ref", "-d", "refs/notes/ai"]).unwrap();
    assert!(repo.read_authorship_note(&sha).is_none());
    let missing = repo.git_ai(&["stats", &sha, "--json"]).unwrap();
    assert!(missing.contains("\"unknown_additions\":1"), "{missing}");
    let output = repo
        .git_ai_without_pre_sync_for_test(&["fetch-notes", "origin", "--json", "--synchronized"])
        .unwrap();
    assert!(output.contains("\"synchronized\":true"), "{output}");
    assert_eq!(repo.read_authorship_note(&sha).unwrap(), expected);
    repo.filename("note.txt")
        .assert_committed_lines(lines!["AI authored line".ai()]);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        repo.git_og(&["update-ref", "-d", "refs/notes/ai"]).unwrap();
        let hooks = repo.path().join(".sync-hooks");
        fs::create_dir(&hooks).unwrap();
        let hook = hooks.join("pre-push");
        fs::write(&hook, "#!/bin/sh\nset -eu\nprintf '[\"%s\"]' \"$SYNC_SHA\" | \"$SYNC_BINARY\" fetch-notes origin --json --synchronized --commits-stdin > .sync-hooks/readiness.json\n\"$SYNC_BINARY\" stats \"$SYNC_SHA\" --json > .sync-hooks/stats.json\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        let prepared = repo.git_ai_command_without_pre_sync_for_test(&[], &[]);
        let mut env: Vec<(String, String)> = prepared
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|value| {
                    (
                        key.to_string_lossy().into_owned(),
                        value.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect();
        env.push(("SYNC_SHA".into(), sha.clone()));
        env.push((
            "SYNC_BINARY".into(),
            repos::test_repo::get_binary_path()
                .to_string_lossy()
                .into_owned(),
        ));
        let env_refs: Vec<_> = env
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let pushed = repo.git_with_env(
            &[
                "-c",
                &format!("core.hooksPath={}", hooks.display()),
                "push",
                "origin",
                "HEAD:refs/heads/from-hook",
            ],
            &env_refs,
            None,
        );
        assert!(
            pushed.is_ok(),
            "{pushed:?}; readiness={:?}",
            fs::read_to_string(hooks.join("readiness.json"))
        );
        let readiness: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(hooks.join("readiness.json")).unwrap())
                .unwrap();
        assert_eq!(readiness["commits"][&sha], "ready");
        let stats: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(hooks.join("stats.json")).unwrap()).unwrap();
        assert_eq!(stats["ai_additions"], 1);
        assert_eq!(stats["unknown_additions"], 0);
        if let Ok(driver) = std::env::var("GIT_AI_HOOK_REAL_DRIVER") {
            repo.git_og(&["update-ref", "-d", "refs/notes/ai"]).unwrap();
            let package = std::path::Path::new(&driver)
                .parent()
                .unwrap()
                .parent()
                .unwrap();
            let mut command = std::process::Command::new("uv");
            command.args([
                "run",
                "--project",
                package.to_str().unwrap(),
                "python",
                &driver,
                &sha,
            ]);
            command
                .current_dir(repo.path())
                .envs(env.iter().map(|(k, v)| (k, v)));
            command.env("REAL_GIT", repos::test_repo::real_git_executable());
            command.env(
                "GIT_TRACE2_EVENT",
                git_ai::daemon::DaemonConfig::trace2_event_target_for_path(
                    &repo.daemon_trace_socket_path(),
                ),
            );
            let mut paths = vec![
                repos::test_repo::get_binary_path()
                    .parent()
                    .unwrap()
                    .to_path_buf(),
            ];
            paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
            command.env("PATH", std::env::join_paths(paths).unwrap());
            let result = command.output().unwrap();
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            let before = json!({"type": "human", "repo_working_dir": repo.path(),
                "will_edit_filepaths": ["after-diagnosis.txt"]});
            repo.git_ai(&[
                "checkpoint",
                "agent-v1",
                "--hook-input",
                &before.to_string(),
            ])
            .unwrap();
            fs::write(
                repo.path().join("after-diagnosis.txt"),
                "AI after diagnosis\n",
            )
            .unwrap();
            let after = json!({"type": "ai_agent", "repo_working_dir": repo.path(),
                "edited_filepaths": ["after-diagnosis.txt"], "agent_name": "codex",
                "model": "gpt-6", "conversation_id": "notes-after-diagnosis"});
            repo.git_ai(&["checkpoint", "agent-v1", "--hook-input", &after.to_string()])
                .unwrap();
            repo.git_og(&["add", "after-diagnosis.txt"]).unwrap();
            repo.commit("AI after diagnosis").unwrap();
            repo.filename("after-diagnosis.txt")
                .assert_committed_lines(lines!["AI after diagnosis".ai()]);
            repo.filename("note.txt")
                .assert_committed_lines(lines!["AI authored line".ai()]);
            let stats = repo
                .git_ai(&["stats", "HEAD", "--json", "--require-note"])
                .unwrap();
            if let Ok(evidence) = std::env::var("GIT_AI_HOOK_EVIDENCE_DIR") {
                fs::write(
                    std::path::Path::new(&evidence).join("post-diagnostic-stats.json"),
                    stats,
                )
                .unwrap();
            }
        }
    }
    fs::write(
        repo.path().join("untracked.txt"),
        "Untracked contribution\n",
    )
    .unwrap();
    repo.git_og(&["add", "untracked.txt"]).unwrap();
    repo.git_og(&["commit", "-m", "Untracked contribution"])
        .unwrap();
    repo.filename("untracked.txt")
        .assert_committed_lines(lines!["Untracked contribution".unattributed_human()]);
    repo.filename("note.txt")
        .assert_committed_lines(lines!["AI authored line".ai()]);
    let parent = repo
        .git_og(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();
    repo.git_og(&["notes", "--ref=ai", "remove", &parent])
        .unwrap();
    let targets = json!([sha, parent]).to_string();
    let args = [
        "fetch-notes",
        "origin",
        "--json",
        "--synchronized",
        "--commits-stdin",
    ];
    let ready = repo.git_ai_with_stdin(&args, targets.as_bytes()).unwrap();
    let ready: serde_json::Value = serde_json::from_str(ready.trim()).unwrap();
    assert_eq!(ready["commits"][&sha], "ready");
    assert_eq!(ready["commits"][&parent], "note_missing");
    repo.git_og(&[
        "notes",
        "--ref=ai",
        "add",
        "-f",
        "-m",
        "{invalid note",
        &sha,
    ])
    .unwrap();
    let damaged = repo.git_ai_with_stdin(&args, targets.as_bytes()).unwrap();
    let damaged: serde_json::Value = serde_json::from_str(damaged.trim()).unwrap();
    assert_eq!(damaged["commits"][&sha], "note_parse_failed");
    assert!(
        repo.git_ai(&["stats", &sha, "--json", "--require-note"])
            .is_err()
    );
    let failed = repo
        .git_ai(&["fetch-notes", "missing-remote", "--json", "--synchronized"])
        .unwrap_err();
    assert!(failed.contains("fetch_failed"), "{failed}");
}
