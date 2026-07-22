use crate::repos::test_file::ExpectedLineExt;
use crate::repos::test_repo::TestRepo;
use git_ai::authorship::stats::CommitStats;
use insta::assert_debug_snapshot;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

/// Extract the first complete JSON object from mixed stdout/stderr output.
fn extract_json_object(output: &str) -> String {
    let start = output.find('{').unwrap_or(0);
    let end = output.rfind('}').unwrap_or(output.len().saturating_sub(1));
    output[start..=end].to_string()
}

fn stats_from_args(repo: &TestRepo, args: &[&str]) -> CommitStats {
    let raw = repo.git_ai(args).expect("git-ai stats should succeed");
    let json = extract_json_object(&raw);
    serde_json::from_str(&json).expect("valid stats json")
}

fn stats_json_from_args(repo: &TestRepo, args: &[&str]) -> serde_json::Value {
    let raw = repo.git_ai(args).expect("git-ai stats should succeed");
    serde_json::from_str(&extract_json_object(&raw)).expect("valid stats json")
}

#[test]
fn test_stats_json_includes_file_stats() {
    let repo = TestRepo::new();
    fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "base.txt"])
        .unwrap();
    repo.stage_all_and_commit("base").unwrap();
    fs::create_dir_all(repo.path().join("src")).unwrap();

    repo.git_ai(&["checkpoint", "human", "src/ai.rs"]).unwrap();
    fs::write(repo.path().join("src/ai.rs"), "ai one\nai two\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "src/ai.rs"])
        .unwrap();
    fs::write(repo.path().join("src/human.rs"), "human\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "src/human.rs"])
        .unwrap();
    fs::write(repo.path().join("src/unknown.rs"), "unknown\n").unwrap();
    repo.stage_all_and_commit("mixed files").unwrap();

    let json = stats_json_from_args(&repo, &["stats", "HEAD", "--json"]);
    assert_eq!(json["file_stats"]["src/ai.rs"]["ai_accepted"], 2);
    assert_eq!(json["file_stats"]["src/human.rs"]["unknown_additions"], 0);
    assert_eq!(json["file_stats"]["src/unknown.rs"]["unknown_additions"], 1);
    assert!(json.get("summary").is_none());
}

#[test]
fn test_stats_range_json_includes_file_stats() {
    let repo = TestRepo::new();
    fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "base.txt"])
        .unwrap();
    let base = repo.stage_all_and_commit("base").unwrap();
    fs::create_dir_all(repo.path().join("src")).unwrap();

    repo.git_ai(&["checkpoint", "human", "src/ai.rs"]).unwrap();
    fs::write(repo.path().join("src/ai.rs"), "ai one\nai two\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "src/ai.rs"])
        .unwrap();
    fs::write(repo.path().join("src/unknown.rs"), "unknown\n").unwrap();
    let end = repo.stage_all_and_commit("range files").unwrap();

    let range = format!("{}..{}", base.commit_sha, end.commit_sha);
    let json = stats_json_from_args(&repo, &["stats", &range, "--json"]);
    let range_stats = &json["range_stats"];
    assert_eq!(range_stats["file_stats"]["src/ai.rs"]["ai_accepted"], 2);
    assert_eq!(
        range_stats["file_stats"]["src/unknown.rs"]["unknown_additions"],
        1
    );
    let files = range_stats["file_stats"].as_object().unwrap();
    let ai_sum: u64 = files
        .values()
        .map(|value| value["ai_accepted"].as_u64().unwrap())
        .sum();
    let unknown_sum: u64 = files
        .values()
        .map(|value| value["unknown_additions"].as_u64().unwrap())
        .sum();
    assert_eq!(range_stats["ai_accepted"].as_u64().unwrap(), ai_sum);
    assert_eq!(
        range_stats["unknown_additions"].as_u64().unwrap(),
        unknown_sum
    );
}

#[test]
fn test_stats_range_utf8_ignore_keeps_aggregate_file_invariant() {
    let repo = TestRepo::new();
    fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    let base = repo.stage_all_and_commit("base").unwrap();
    fs::write(repo.path().join("keep.rs"), "keep\n").unwrap();
    fs::write(repo.path().join("忽.rs"), "ignored\n").unwrap();
    let end = repo.stage_all_and_commit("range utf8 files").unwrap();

    let range = format!("{}..{}", base.commit_sha, end.commit_sha);
    let json = stats_json_from_args(&repo, &["stats", &range, "--json", "--ignore", "忽.rs"]);
    let range_stats = &json["range_stats"];
    assert!(range_stats["file_stats"].get("忽.rs").is_none());
    assert_eq!(range_stats["file_stats"]["keep.rs"]["unknown_additions"], 1);
    assert_eq!(range_stats["git_diff_added_lines"], 1);
    assert_eq!(range_stats["unknown_additions"], 1);
}

#[test]
fn test_status_json_does_not_include_file_stats() {
    let repo = TestRepo::new();
    repo.filename("README.md")
        .set_contents(crate::lines!["base"]);
    repo.stage_all_and_commit("base").unwrap();
    let raw = repo.git_ai(&["status", "--json"]).unwrap();
    let json = extract_json_object(&raw);
    assert!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap()["stats"]
            .get("file_stats")
            .is_none()
    );
}

#[test]
fn test_stats_file_stats_missing_note() {
    let repo = TestRepo::new();
    fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    repo.stage_all_and_commit("base").unwrap();
    fs::write(repo.path().join("one.txt"), "one\ntwo\n").unwrap();
    fs::write(repo.path().join("two.txt"), "three\n").unwrap();
    repo.stage_all_and_commit("unknown files").unwrap();

    let json = stats_json_from_args(&repo, &["stats", "HEAD", "--json"]);
    assert_eq!(json["file_stats"]["one.txt"]["ai_accepted"], 0);
    assert_eq!(json["file_stats"]["one.txt"]["unknown_additions"], 2);
    assert_eq!(json["file_stats"]["two.txt"]["unknown_additions"], 1);
    assert_eq!(json["unknown_additions"], 3);
}

#[test]
fn test_stats_file_stats_respect_ignore() {
    let repo = TestRepo::new();
    fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    repo.stage_all_and_commit("base").unwrap();
    fs::write(repo.path().join("included.txt"), "included\n").unwrap();
    fs::write(repo.path().join("ignored.txt"), "ignored\n").unwrap();
    repo.stage_all_and_commit("two files").unwrap();

    let json = stats_json_from_args(
        &repo,
        &["stats", "HEAD", "--json", "--ignore", "ignored.txt"],
    );
    assert!(json["file_stats"].get("ignored.txt").is_none());
    assert_eq!(json["file_stats"]["included.txt"]["unknown_additions"], 1);
    assert_eq!(json["unknown_additions"], 1);
}

#[test]
fn test_stats_file_stats_preserve_utf8_paths_and_sorted_order() {
    let repo = TestRepo::new();
    fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    repo.stage_all_and_commit("base").unwrap();
    fs::create_dir_all(repo.path().join("目录")).unwrap();
    fs::write(repo.path().join("z.rs"), "z\n").unwrap();
    fs::write(repo.path().join("a.rs"), "a\n").unwrap();
    fs::write(repo.path().join("目录/统计.rs"), "统计\n").unwrap();
    repo.stage_all_and_commit("utf8 files").unwrap();

    let json = stats_json_from_args(&repo, &["stats", "HEAD", "--json"]);
    let files = json["file_stats"].as_object().unwrap();
    assert!(files.contains_key("目录/统计.rs"));
    let keys: Vec<&String> = files.keys().collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {:?} failed:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn configure_repo_external_diff_helper(repo: &TestRepo) -> String {
    let marker = "STATS_EXTERNAL_DIFF_MARKER";
    let helper_path = repo.path().join("stats-ext-diff-helper.sh");
    let helper_path_posix = helper_path
        .to_str()
        .expect("helper path must be valid UTF-8")
        .replace('\\', "/");

    fs::write(&helper_path, format!("#!/bin/sh\necho {marker}\nexit 0\n"))
        .expect("should write external diff helper");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&helper_path)
            .expect("helper metadata should exist")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&helper_path, perms).expect("helper should be executable");
    }

    repo.git_og(&["config", "diff.external", &helper_path_posix])
        .expect("configuring diff.external should succeed");

    marker.to_string()
}

fn configure_hostile_diff_settings(repo: &TestRepo) {
    let settings = [
        ("diff.noprefix", "true"),
        ("diff.mnemonicprefix", "true"),
        ("diff.srcPrefix", "SRC/"),
        ("diff.dstPrefix", "DST/"),
        ("diff.renames", "copies"),
        ("diff.relative", "true"),
        ("diff.algorithm", "histogram"),
        ("diff.indentHeuristic", "false"),
        ("diff.interHunkContext", "8"),
        ("color.diff", "always"),
        ("color.ui", "always"),
    ];
    for (key, value) in settings {
        repo.git_og(&["config", key, value])
            .unwrap_or_else(|err| panic!("setting {key}={value} should succeed: {err}"));
    }
}

#[test]
fn test_authorship_log_stats() {
    let repo = TestRepo::new();

    // Create an initial commit
    let mut readme = repo.filename("README.md");
    readme.set_contents(crate::lines!["# Project"]);
    repo.stage_all_and_commit("Initial commit").unwrap();

    // AI creates a brand new file with planets
    let mut file = repo.filename("planets.txt");
    file.set_contents(crate::lines![
        "Mercury".human(),
        "Venus".human(),
        "Earth".ai(),
        "Mars".ai(),
        "Jupiter".human(),
        "Saturn".ai(),
        "Uranus".ai(),
        "Neptune".ai(),
        "Pluto (dwarf)".ai(),
    ]);

    file.set_contents(crate::lines![
        "Mercury".human(),
        "Venus".human(),
        "Earth".ai(),
        "Mars".ai(),
        "Jupiter".human(),
        "Saturn".ai(),
        "Uranus".ai(),
        "Neptune (override)".human(),
        "Pluto (dwarf)".ai(),
    ]);

    // First commit should have all the planets
    let first_commit = repo.stage_all_and_commit("Add planets").unwrap();

    file.assert_lines_and_blame(crate::lines![
        "Mercury".human(),
        "Venus".human(),
        "Earth".ai(),
        "Mars".ai(),
        "Jupiter".human(),
        "Saturn".ai(),
        "Uranus".ai(),
        "Neptune (override)".human(),
        "Pluto (dwarf)".ai(),
    ]);

    assert_eq!(first_commit.authorship_log.attestations.len(), 1);

    let raw = repo.git_ai(&["stats", "--json"]).unwrap();
    let json = extract_json_object(&raw);
    let stats: CommitStats = serde_json::from_str(&json).unwrap();
    // The integration harness now uses mock_known_human (CheckpointKind::KnownHuman), which
    // produces h_-prefixed attestation entries for lines written under a human checkpoint.
    // Neptune (override) — human-overrides-AI line — gets h_<hash> attestation.
    // Mercury, Venus, Jupiter also get h_<hash> attestation from the KnownHuman checkpoint.
    // All 4 human-written lines now count as human_additions; unknown_additions = 0.
    // Neptune (override) is now h_<hash> attested, so it counts as human_additions only.
    assert_eq!(stats.human_additions, 4);
    assert_eq!(stats.unknown_additions, 0);
    assert_eq!(stats.ai_additions, 5); // Neptune (override) no longer counted as mixed AI
    assert_eq!(stats.ai_accepted, 5);
    assert_eq!(stats.git_diff_deleted_lines, 0);
    assert_eq!(stats.git_diff_added_lines, 9);

    assert_eq!(stats.tool_model_breakdown.len(), 1);
    // ai_additions = ai_accepted = 5
    assert_eq!(
        stats
            .tool_model_breakdown
            .get("mock_ai::unknown")
            .unwrap()
            .ai_additions,
        5
    );
    assert_eq!(
        stats
            .tool_model_breakdown
            .get("mock_ai::unknown")
            .unwrap()
            .ai_accepted,
        5
    );
}

#[test]
fn test_stats_cli_range() {
    let repo = TestRepo::new();

    // Initial human commit
    let mut file = repo.filename("range.txt");
    file.set_contents(crate::lines!["Line 1".human()]);
    let first = repo.stage_all_and_commit("Initial human").unwrap();

    // AI adds a line in a second commit
    file.set_contents(crate::lines!["Line 1".human(), "Line 2".ai()]);
    let second = repo.stage_all_and_commit("AI adds line").unwrap();

    // Sanity check individual commit stats
    let range = format!("{}..{}", first.commit_sha, second.commit_sha);
    let raw = repo
        .git_ai(&["stats", &range, "--json"])
        .expect("git-ai stats range should succeed");

    let output = extract_json_object(&raw);
    let stats: git_ai::authorship::range_authorship::RangeAuthorshipStats =
        serde_json::from_str(&output).unwrap();

    // Range should only include the AI commit's diff and report at least one AI-added line
    assert_eq!(stats.authorship_stats.total_commits, 1);
    assert!(
        stats.range_stats.ai_additions >= 1,
        "expected at least one AI addition in range, got {}",
        stats.range_stats.ai_additions
    );
    assert!(
        stats.range_stats.git_diff_added_lines >= stats.range_stats.ai_additions,
        "git diff added lines ({}) should be >= ai_additions ({})",
        stats.range_stats.git_diff_added_lines,
        stats.range_stats.ai_additions
    );
}

#[test]
fn test_stats_cli_range_ignores_repo_external_diff_helper() {
    let repo = TestRepo::new();

    let mut file = repo.filename("stats-range-ext.txt");
    file.set_contents(crate::lines!["base".human()]);
    let first = repo.stage_all_and_commit("initial").unwrap();

    file.set_contents(crate::lines!["base".human(), "ai line".ai()]);
    let second = repo.stage_all_and_commit("ai second").unwrap();

    let marker = configure_repo_external_diff_helper(&repo);
    let proxied_diff = repo
        .git(&["diff", &first.commit_sha, &second.commit_sha])
        .expect("proxied git diff should succeed");
    assert!(
        proxied_diff.contains(&marker),
        "sanity check: proxied git diff should use configured external helper"
    );

    let range = format!("{}..{}", first.commit_sha, second.commit_sha);
    let raw = repo
        .git_ai(&["stats", &range, "--json"])
        .expect("git-ai stats range should succeed with external diff configured");
    assert!(
        !raw.contains(&marker),
        "git-ai stats output should not include external helper output, got:\n{}",
        raw
    );

    let output = extract_json_object(&raw);
    let stats: git_ai::authorship::range_authorship::RangeAuthorshipStats =
        serde_json::from_str(&output).unwrap();
    assert_eq!(stats.authorship_stats.total_commits, 1);
    assert!(
        stats.range_stats.git_diff_added_lines >= 1,
        "expected at least one added line in range, got {}",
        stats.range_stats.git_diff_added_lines
    );
    assert!(stats.range_stats.ai_additions >= 1);
}

#[test]
fn test_stats_cli_range_with_hostile_diff_config() {
    let repo = TestRepo::new();

    let mut file = repo.filename("stats-range-hostile.txt");
    file.set_contents(crate::lines!["base".human()]);
    let first = repo.stage_all_and_commit("initial").unwrap();

    file.set_contents(crate::lines!["base".human(), "ai line".ai()]);
    let second = repo.stage_all_and_commit("ai second").unwrap();

    configure_hostile_diff_settings(&repo);

    let range = format!("{}..{}", first.commit_sha, second.commit_sha);
    let raw = repo
        .git_ai(&["stats", &range, "--json"])
        .expect("git-ai stats range should succeed with hostile diff config");
    let output = extract_json_object(&raw);
    let stats: git_ai::authorship::range_authorship::RangeAuthorshipStats =
        serde_json::from_str(&output).unwrap();

    assert_eq!(stats.authorship_stats.total_commits, 1);
    assert!(stats.range_stats.git_diff_added_lines >= 1);
    assert!(stats.range_stats.ai_additions >= 1);
}

#[test]
fn test_stats_cli_empty_tree_range() {
    let repo = TestRepo::new();

    // First commit: AI line
    let mut file = repo.filename("history.txt");
    file.set_contents(crate::lines!["AI Line 1".ai()]);
    let _first = repo.stage_all_and_commit("Initial AI").unwrap();

    // Second commit: human line
    file.set_contents(crate::lines!["AI Line 1".ai(), "Human Line 2".human()]);
    repo.stage_all_and_commit("Human adds line").unwrap();

    // Git's empty tree OID
    let empty_tree = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
    let head = repo
        .git(&["rev-parse", "HEAD"])
        .expect("rev-parse HEAD should succeed")
        .trim()
        .to_string();
    let range = format!("{}..{}", empty_tree, head);

    let raw = repo
        .git_ai(&["stats", &range, "--json"])
        .expect("git-ai stats empty-tree range should succeed");

    let output = extract_json_object(&raw);
    let stats: git_ai::authorship::range_authorship::RangeAuthorshipStats =
        serde_json::from_str(&output).unwrap();

    // Entire history from empty tree to HEAD:
    // - 2 commits in range
    // - 1 AI-added line, 1 human-added line in final diff
    assert_eq!(stats.authorship_stats.total_commits, 2);
    assert_eq!(stats.range_stats.git_diff_added_lines, 2);
    assert_eq!(stats.range_stats.ai_additions, 1);
    // Range stats use legacy Human checkpoints and pass known_human_accepted=0,
    // so human lines appear as unknown_additions (not human_additions).
    assert_eq!(stats.range_stats.human_additions, 0);
    assert_eq!(stats.range_stats.unknown_additions, 1);
}

#[test]
fn test_markdown_stats_deletion_only() {
    use git_ai::authorship::stats::write_stats_to_markdown;
    use std::collections::BTreeMap;

    let stats = CommitStats {
        human_additions: 0,
        unknown_additions: 0,
        ai_additions: 0,
        ai_accepted: 0,

        git_diff_deleted_lines: 5,
        git_diff_added_lines: 0,
        tool_model_breakdown: BTreeMap::new(),
    };

    let markdown = write_stats_to_markdown(&stats);

    assert_debug_snapshot!(markdown);
}

#[test]
fn test_markdown_stats_all_human() {
    use git_ai::authorship::stats::write_stats_to_markdown;
    use std::collections::BTreeMap;

    let stats = CommitStats {
        human_additions: 10,
        unknown_additions: 0,
        ai_additions: 0,
        ai_accepted: 0,

        git_diff_deleted_lines: 0,
        git_diff_added_lines: 10,
        tool_model_breakdown: BTreeMap::new(),
    };

    let markdown = write_stats_to_markdown(&stats);

    assert_debug_snapshot!(markdown);
}

#[test]
fn test_markdown_stats_all_ai() {
    use git_ai::authorship::stats::write_stats_to_markdown;
    use std::collections::BTreeMap;

    let stats = CommitStats {
        human_additions: 0,
        unknown_additions: 0,
        ai_additions: 15,
        ai_accepted: 15,

        git_diff_deleted_lines: 0,
        git_diff_added_lines: 15,
        tool_model_breakdown: BTreeMap::new(),
    };

    let markdown = write_stats_to_markdown(&stats);

    assert_debug_snapshot!(markdown);
}

#[test]
fn test_markdown_stats_mixed() {
    use git_ai::authorship::stats::write_stats_to_markdown;
    use std::collections::BTreeMap;

    let stats = CommitStats {
        human_additions: 10,
        unknown_additions: 0,
        ai_additions: 15,
        ai_accepted: 15,

        git_diff_deleted_lines: 5,
        git_diff_added_lines: 30,
        tool_model_breakdown: BTreeMap::new(),
    };

    let markdown = write_stats_to_markdown(&stats);

    assert_debug_snapshot!(markdown);
}

#[test]
fn test_markdown_stats_no_mixed() {
    use git_ai::authorship::stats::write_stats_to_markdown;
    use std::collections::BTreeMap;

    let stats = CommitStats {
        human_additions: 8,
        unknown_additions: 0,
        ai_additions: 12,
        ai_accepted: 12,

        git_diff_deleted_lines: 0,
        git_diff_added_lines: 20,
        tool_model_breakdown: BTreeMap::new(),
    };

    let markdown = write_stats_to_markdown(&stats);

    assert_debug_snapshot!(markdown);
}

#[test]
fn test_markdown_stats_minimal_human() {
    use git_ai::authorship::stats::write_stats_to_markdown;
    use std::collections::BTreeMap;

    // Test that humans get at least 2 visible blocks if they have more than 1 line
    let stats = CommitStats {
        human_additions: 2,
        unknown_additions: 0,
        ai_additions: 98,
        ai_accepted: 98,

        git_diff_deleted_lines: 0,
        git_diff_added_lines: 100,
        tool_model_breakdown: BTreeMap::new(),
    };

    let markdown = write_stats_to_markdown(&stats);

    assert_debug_snapshot!(markdown);
}

#[test]
fn test_markdown_stats_formatting() {
    use git_ai::authorship::stats::{ToolModelHeadlineStats, write_stats_to_markdown};
    use std::collections::BTreeMap;

    let mut tool_model_breakdown = BTreeMap::new();
    tool_model_breakdown.insert(
        "cursor::claude-3.5-sonnet".to_string(),
        ToolModelHeadlineStats {
            ai_additions: 6,
            ai_accepted: 6,
        },
    );

    let stats = CommitStats {
        human_additions: 5,
        unknown_additions: 0,
        ai_additions: 6,
        ai_accepted: 6,
        git_diff_deleted_lines: 2,
        git_diff_added_lines: 13,
        tool_model_breakdown,
    };

    let markdown = write_stats_to_markdown(&stats);
    println!("{}", markdown);
    assert_debug_snapshot!(markdown);
}

#[test]
fn test_stats_default_ignores_snapshot_files() {
    let repo = TestRepo::new();
    repo.filename("README.md")
        .set_contents(crate::lines!["# Repo"]);
    repo.stage_all_and_commit("Initial commit").unwrap();

    repo.filename("src/main.rs")
        .set_contents(crate::lines!["fn main() {}".ai()]);
    repo.filename("__snapshots__/main.snap")
        .set_contents(crate::lines![
            "snapshot line 1",
            "snapshot line 2",
            "snapshot line 3"
        ]);
    repo.stage_all_and_commit("Add source and snapshot")
        .unwrap();

    let stats = stats_from_args(&repo, &["stats", "HEAD", "--json"]);
    assert_eq!(stats.git_diff_added_lines, 1);
    assert_eq!(stats.ai_additions, 1);
}

#[test]
fn test_stats_default_ignores_lockfiles_and_generated_files() {
    let repo = TestRepo::new();
    repo.filename("README.md")
        .set_contents(crate::lines!["# Repo"]);
    repo.stage_all_and_commit("Initial commit").unwrap();

    repo.filename("src/lib.rs")
        .set_contents(crate::lines!["pub fn answer() -> u32 { 42 }".ai()]);
    repo.filename("Cargo.lock")
        .set_contents(vec!["lock".to_string().repeat(5); 650]);
    repo.filename("api.generated.ts")
        .set_contents(vec!["export type X = string;".to_string(); 500]);
    repo.stage_all_and_commit("Add source and generated artifacts")
        .unwrap();

    let stats = stats_from_args(&repo, &["stats", "HEAD", "--json"]);
    assert_eq!(stats.git_diff_added_lines, 1);
    assert_eq!(stats.ai_additions, 1);
}

#[test]
fn test_stats_ignores_linguist_generated_patterns() {
    let repo = TestRepo::new();
    repo.filename(".gitattributes")
        .set_contents(crate::lines!["generated/** linguist-generated=true"]);
    repo.filename("README.md")
        .set_contents(crate::lines!["# Repo"]);
    repo.stage_all_and_commit("Initial commit with gitattributes")
        .unwrap();

    repo.filename("src/main.rs")
        .set_contents(crate::lines!["fn run() {}".ai()]);
    repo.filename("generated/schema.ts")
        .set_contents(crate::lines!["export const schema = {};"]);
    repo.stage_all_and_commit("Add source and linguist-generated file")
        .unwrap();

    let stats = stats_from_args(&repo, &["stats", "HEAD", "--json"]);
    assert_eq!(stats.git_diff_added_lines, 1);
    assert_eq!(stats.ai_additions, 1);
}

#[test]
fn test_stats_keeps_negative_linguist_patterns_counted() {
    let repo = TestRepo::new();
    repo.filename(".gitattributes").set_contents(crate::lines![
        "generated/** linguist-generated=true",
        "manual/** linguist-generated=false"
    ]);
    repo.filename("README.md")
        .set_contents(crate::lines!["# Repo"]);
    repo.stage_all_and_commit("Initial commit with attrs")
        .unwrap();

    repo.filename("generated/out.ts")
        .set_contents(crate::lines!["export const ignored = true;"]);
    repo.filename("manual/kept.ts")
        .set_contents(crate::lines!["export const counted = true;".ai()]);
    repo.stage_all_and_commit("Add generated and manual files")
        .unwrap();

    let stats = stats_from_args(&repo, &["stats", "HEAD", "--json"]);
    assert_eq!(stats.git_diff_added_lines, 1);
    assert_eq!(stats.ai_additions, 1);
}

#[test]
fn test_stats_in_bare_clone_uses_root_gitattributes_linguist_generated() {
    let repo = TestRepo::new();
    repo.filename(".gitattributes")
        .set_contents(crate::lines!["generated/** linguist-generated=true"]);
    repo.filename("README.md")
        .set_contents(crate::lines!["# Repo"]);
    repo.stage_all_and_commit("Initial commit with gitattributes")
        .unwrap();

    repo.filename("src/main.rs")
        .set_contents(crate::lines!["fn run() {}".ai()]);
    repo.filename("generated/schema.ts")
        .set_contents(crate::lines!["export const schema = {};"]);
    repo.stage_all_and_commit("Add source and linguist-generated file")
        .unwrap();

    let temp = tempfile::tempdir().expect("tempdir");
    let bare = temp.path().join("repo.git");
    run_git(
        temp.path(),
        &[
            "clone",
            "--bare",
            repo.path().to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );

    let output = Command::new(crate::repos::test_repo::get_binary_path())
        .args(["stats", "HEAD", "--json"])
        .current_dir(&bare)
        .env(
            "GIT_AI_TEST_DB_PATH",
            temp.path().join("db").to_str().unwrap(),
        )
        .output()
        .expect("git-ai stats should run in bare repo");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "git-ai stats failed in bare clone:\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );

    let combined = if stdout.is_empty() {
        stderr.to_string()
    } else if stderr.is_empty() {
        stdout.to_string()
    } else {
        format!("{}{}", stdout, stderr)
    };
    let json = extract_json_object(&combined);
    let stats: CommitStats = serde_json::from_str(&json).expect("valid stats json");
    assert_eq!(stats.git_diff_added_lines, 1);
}

#[test]
fn test_stats_ignore_flag_is_additive_to_defaults() {
    let repo = TestRepo::new();
    repo.filename("README.md")
        .set_contents(crate::lines!["# Repo"]);
    repo.stage_all_and_commit("Initial commit").unwrap();

    repo.filename("src/main.rs")
        .set_contents(crate::lines!["fn main() {}".ai()]);
    repo.filename("docs/keep.txt")
        .set_contents(crate::lines!["this line is human"]);
    repo.stage_all_and_commit("Add docs and source").unwrap();

    let baseline = stats_from_args(&repo, &["stats", "HEAD", "--json"]);
    assert_eq!(baseline.git_diff_added_lines, 2);

    let ignored = stats_from_args(
        &repo,
        &["stats", "HEAD", "--json", "--ignore", "docs/keep.txt"],
    );
    assert_eq!(ignored.git_diff_added_lines, 1);
}

#[test]
fn test_stats_range_uses_default_ignores() {
    let repo = TestRepo::new();
    repo.filename("README.md")
        .set_contents(crate::lines!["# Repo"]);
    let first = repo.stage_all_and_commit("Initial commit").unwrap();

    repo.filename("src/main.rs")
        .set_contents(crate::lines!["fn main() {}".ai()]);
    repo.filename("Cargo.lock")
        .set_contents(vec!["lockdata".to_string(); 700]);
    let second = repo
        .stage_all_and_commit("Add source and lockfile")
        .unwrap();

    let range = format!("{}..{}", first.commit_sha, second.commit_sha);
    let raw = repo
        .git_ai(&["stats", &range, "--json"])
        .expect("git-ai stats range should succeed");
    let json = extract_json_object(&raw);
    let range_stats: git_ai::authorship::range_authorship::RangeAuthorshipStats =
        serde_json::from_str(&json).unwrap();

    assert_eq!(range_stats.range_stats.git_diff_added_lines, 1);
    assert_eq!(range_stats.range_stats.ai_additions, 1);
}

#[test]
fn test_post_commit_large_ignored_files_do_not_trigger_skip_warning() {
    let repo = TestRepo::new();
    repo.filename("README.md")
        .set_contents(crate::lines!["# Repo"]);
    repo.stage_all_and_commit("Initial commit").unwrap();

    repo.filename("Cargo.lock")
        .set_contents(vec!["lockfile-entry".to_string(); 7001]);
    let commit = repo
        .stage_all_and_commit("Large lockfile update")
        .expect("commit should succeed");

    assert!(
        !commit
            .stdout
            .contains("Skipped git-ai stats for large commit"),
        "large ignored files should not trigger post-commit skip warning: {}",
        commit.stdout
    );

    let stats = stats_from_args(&repo, &["stats", "HEAD", "--json"]);
    assert_eq!(stats.git_diff_added_lines, 0);
    assert_eq!(stats.ai_additions, 0);
    assert_eq!(stats.human_additions, 0);

    let json = stats_json_from_args(&repo, &["stats", "HEAD", "--json"]);
    assert!(json["file_stats"].as_object().unwrap().is_empty());
}

#[test]
fn test_stats_ignores_renamed_files() {
    // Test that stats correctly ignores pure renames (no content changes)
    // Reproduces issue #923
    let repo = TestRepo::new();

    // Initial commit with files in a directory
    repo.filename("misc/Development Notes.md")
        .set_contents(crate::lines![
            "# Development Notes",
            "",
            "Some content here",
            "More content",
            "Even more",
            "Line 6",
            "Line 7",
            "Line 8",
            "Line 9",
            "Line 10",
            "Line 11",
            "Line 12",
            "Line 13",
            "Line 14",
            "Line 15",
            "Line 16"
        ]);
    repo.filename("misc/Usage Guide.md")
        .set_contents(crate::lines!["# Usage Guide", "", "Usage info"]);
    repo.stage_all_and_commit("Initial commit with misc directory")
        .unwrap();

    // Rename the directory (pure rename, no content changes)
    let misc_dev = repo.path().join("misc/Development Notes.md");
    let misc_usage = repo.path().join("misc/Usage Guide.md");
    let new_dir = repo.path().join("Misc Docs");
    fs::create_dir(&new_dir).unwrap();
    fs::rename(&misc_dev, new_dir.join("Development Notes.md")).unwrap();
    fs::rename(&misc_usage, new_dir.join("Usage Guide.md")).unwrap();
    fs::remove_dir(repo.path().join("misc")).unwrap();

    repo.stage_all_and_commit("Rename misc to Misc Docs")
        .unwrap();

    // Verify that git ai diff recognizes this as a rename
    let diff_output = repo.git_ai(&["diff", "HEAD"]).unwrap();
    assert!(
        diff_output.contains("similarity index 100%") || diff_output.contains("rename from"),
        "git ai diff should recognize pure renames"
    );

    // Stats should show 0 additions and 0 deletions for pure renames
    let stats = stats_from_args(&repo, &["stats", "HEAD", "--json"]);
    assert_eq!(
        stats.git_diff_added_lines, 0,
        "Pure renames should not count as additions"
    );
    assert_eq!(
        stats.git_diff_deleted_lines, 0,
        "Pure renames should not count as deletions"
    );
    assert_eq!(stats.ai_additions, 0);
    assert_eq!(stats.human_additions, 0);

    let json = stats_json_from_args(&repo, &["stats", "HEAD", "--json"]);
    assert!(json["file_stats"].as_object().unwrap().is_empty());
}

crate::reuse_tests_in_worktree!(
    test_authorship_log_stats,
    test_stats_cli_range,
    test_stats_cli_empty_tree_range,
    test_markdown_stats_deletion_only,
    test_markdown_stats_all_human,
    test_markdown_stats_all_ai,
    test_markdown_stats_mixed,
    test_markdown_stats_no_mixed,
    test_markdown_stats_minimal_human,
    test_markdown_stats_formatting,
    test_stats_default_ignores_snapshot_files,
    test_stats_default_ignores_lockfiles_and_generated_files,
    test_stats_ignores_linguist_generated_patterns,
    test_stats_keeps_negative_linguist_patterns_counted,
    test_stats_in_bare_clone_uses_root_gitattributes_linguist_generated,
    test_stats_ignore_flag_is_additive_to_defaults,
    test_stats_range_uses_default_ignores,
    test_post_commit_large_ignored_files_do_not_trigger_skip_warning,
    test_stats_ignores_renamed_files,
);
