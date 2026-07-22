# File-Level Stats Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add deterministic per-file `ai_accepted` and `unknown_additions` data to single-commit and range `git-ai stats --json` output without changing other output schemas.

**Architecture:** Keep the shared aggregate `CommitStats` schema unchanged and introduce a stats-specific `DetailedCommitStats` that flattens the aggregate fields and adds a sorted `file_stats` map. Extend the existing per-file hunk/attestation pass for single commits and the existing per-file diff/blame pass for ranges, then serialize the detailed type only from stats JSON paths.

**Tech Stack:** Rust 2024, serde/serde_json, `BTreeMap`/`HashMap`, existing Git CLI diff and blame adapters, existing `TestRepo` integration harness.

## Global Constraints

- Support both a single commit and a commit range.
- Add only per-file `ai_accepted` and `unknown_additions`.
- Preserve all existing aggregate field names, values, and meanings.
- Keep terminal stats, `git-ai status`, post-commit output, authorship notes, and telemetry schemas unchanged.
- Keep the existing range behavior that treats known-human accepted lines as zero during range aggregation.
- Include only non-ignored files with at least one added line; pure deletion and ignored files are absent.
- Use repository-relative POSIX-normalized destination paths and deterministic key ordering.
- Do not add dependencies or change the authorship note schema.
- Aggregate AI and unknown counts must equal the corresponding sums across `file_stats`.

---

## File Structure

- Modify `src/authorship/stats.rs`: own file-level public types, detailed serialization, single-commit per-file aggregation, and compatibility wrappers returning aggregate `CommitStats`.
- Modify `src/authorship/diff_ai_accepted.rs`: retain per-file added-line and AI-accepted counts during range blame.
- Modify `src/authorship/range_authorship.rs`: return detailed range stats and derive per-file unknown counts using existing range semantics.
- Modify `src/commands/git_ai_handlers.rs`: serialize the detailed range DTO while leaving non-JSON rendering unchanged.
- Modify `tests/integration/stats_unit.rs`: focused calculation and serialization tests.
- Modify `tests/integration/stats.rs`: CLI JSON contract, range behavior, ignore handling, missing-note behavior, and output-isolation tests.

### Task 1: Define the Detailed Stats Contract

**Files:**
- Modify: `src/authorship/stats.rs:9-34`
- Test: `tests/integration/stats_unit.rs`

**Interfaces:**
- Consumes: existing `CommitStats`.
- Produces: `FileStats`, `DetailedCommitStats`, and `Deref<Target = CommitStats>` for existing read-only aggregate field access.

- [ ] **Step 1: Write failing serialization tests**

Add imports and tests to `tests/integration/stats_unit.rs`:

```rust
use std::ops::Deref;

#[test]
fn test_detailed_commit_stats_serializes_flat_file_stats() {
    let summary = CommitStats {
        ai_accepted: 2,
        ai_additions: 2,
        unknown_additions: 1,
        git_diff_added_lines: 3,
        ..CommitStats::default()
    };
    let detailed = DetailedCommitStats {
        summary,
        file_stats: BTreeMap::from([
            (
                "src/a.rs".to_string(),
                FileStats {
                    ai_accepted: 2,
                    unknown_additions: 0,
                },
            ),
            (
                "src/b.rs".to_string(),
                FileStats {
                    ai_accepted: 0,
                    unknown_additions: 1,
                },
            ),
        ]),
    };

    let json = serde_json::to_value(&detailed).unwrap();
    assert_eq!(json["ai_accepted"], 2);
    assert_eq!(json["file_stats"]["src/a.rs"]["ai_accepted"], 2);
    assert_eq!(json["file_stats"]["src/b.rs"]["unknown_additions"], 1);
    assert!(json.get("summary").is_none());
}

#[test]
fn test_commit_stats_serialization_remains_aggregate_only() {
    let json = serde_json::to_value(CommitStats::default()).unwrap();
    assert!(json.get("file_stats").is_none());
}
```

- [ ] **Step 2: Run the tests and verify they fail**

Run:

```bash
task test TEST_FILTER=test_detailed_commit_stats_serializes_flat_file_stats
```

Expected: compilation fails because `DetailedCommitStats` and `FileStats` do not exist.

- [ ] **Step 3: Add the data types**

Add to `src/authorship/stats.rs` after `CommitStats`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct FileStats {
    #[serde(default)]
    pub ai_accepted: u32,
    #[serde(default)]
    pub unknown_additions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DetailedCommitStats {
    #[serde(flatten)]
    pub summary: CommitStats,
    #[serde(default)]
    pub file_stats: BTreeMap<String, FileStats>,
}

impl std::ops::Deref for DetailedCommitStats {
    type Target = CommitStats;

    fn deref(&self) -> &Self::Target {
        &self.summary
    }
}
```

Do not add a field to `CommitStats`.

- [ ] **Step 4: Run both serialization tests**

Run:

```bash
task test TEST_FILTER=test_detailed_commit_stats_serializes_flat_file_stats
task test TEST_FILTER=test_commit_stats_serialization_remains_aggregate_only
```

Expected: both tests pass.

- [ ] **Step 5: Commit the contract**

```bash
git add src/authorship/stats.rs tests/integration/stats_unit.rs
git commit -m "feat: define detailed file stats output"
```

### Task 2: Compute Single-Commit File Statistics

**Files:**
- Modify: `src/authorship/stats.rs:329-588`
- Test: `tests/integration/stats_unit.rs`

**Interfaces:**
- Consumes: `DetailedCommitStats`, `FileStats`, `DiffHunk`, `AuthorshipLog`, and the existing `line_range_overlap_len` helper.
- Produces: `stats_for_commit_detailed(repo, commit_sha, ignore_patterns) -> Result<DetailedCommitStats, GitAiError>` and an internal detailed hunk helper. Existing `stats_for_commit_stats*` functions continue returning `CommitStats`.

- [ ] **Step 1: Add a failing multi-file calculation test**

Add to `tests/integration/stats_unit.rs`:

```rust
#[test]
fn test_detailed_stats_group_ai_human_and_unknown_by_file() {
    let repo = TestRepo::new();
    std::fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "base.txt"])
        .unwrap();
    repo.stage_all_and_commit("base").unwrap();

    std::fs::create_dir_all(repo.path().join("src")).unwrap();

    repo.git_ai(&["checkpoint", "human", "src/ai.rs"])
        .unwrap();
    std::fs::write(repo.path().join("src/ai.rs"), "ai one\nai two\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "src/ai.rs"])
        .unwrap();

    std::fs::write(repo.path().join("src/human.rs"), "human\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "src/human.rs"])
        .unwrap();

    std::fs::write(repo.path().join("src/unknown.rs"), "unknown\n").unwrap();
    let commit = repo.stage_all_and_commit("mixed files").unwrap();
    let gitai_repo = find_repository_in_path(repo.path().to_str().unwrap()).unwrap();

    let detailed = stats_for_commit_detailed(&gitai_repo, &commit.commit_sha, &[]).unwrap();

    assert_eq!(detailed.file_stats["src/ai.rs"].ai_accepted, 2);
    assert_eq!(detailed.file_stats["src/ai.rs"].unknown_additions, 0);
    assert_eq!(detailed.file_stats["src/human.rs"], FileStats::default());
    assert_eq!(detailed.file_stats["src/unknown.rs"].ai_accepted, 0);
    assert_eq!(detailed.file_stats["src/unknown.rs"].unknown_additions, 1);
    assert_eq!(
        detailed.ai_accepted,
        detailed.file_stats.values().map(|value| value.ai_accepted).sum()
    );
    assert_eq!(
        detailed.unknown_additions,
        detailed
            .file_stats
            .values()
            .map(|value| value.unknown_additions)
            .sum()
    );
}
```

- [ ] **Step 2: Run the test and verify it fails**

Run:

```bash
task test TEST_FILTER=test_detailed_stats_group_ai_human_and_unknown_by_file NO_CAPTURE=true
```

Expected: compilation fails because `stats_for_commit_detailed` does not exist.

- [ ] **Step 3: Add detailed attestation accounting**

Add private accounting types in `src/authorship/stats.rs`:

```rust
#[derive(Debug, Default)]
struct FileAcceptedCounts {
    ai_accepted: u32,
    known_human_accepted: u32,
}

#[derive(Debug, Default)]
struct AcceptedCounts {
    ai_accepted: u32,
    known_human_accepted: u32,
    ai_accepted_by_tool: BTreeMap<String, u32>,
    by_file: BTreeMap<String, FileAcceptedCounts>,
}
```

Extract the body of `accepted_lines_from_attestations` into a new private `accepted_counts_from_attestations` that updates both totals and `by_file[file_path]`. Keep `accepted_lines_from_attestations` as a compatibility wrapper returning the existing tuple:

```rust
let counts = accepted_counts_from_attestations(
    authorship_log,
    added_lines_by_file,
    is_merge_commit,
);
(
    counts.ai_accepted,
    counts.known_human_accepted,
    counts.ai_accepted_by_tool,
)
```

For every `h_` entry, increment both `known_human_accepted` totals. For every accepted AI entry, increment both AI totals and preserve the existing tool/model lookup unchanged.

- [ ] **Step 4: Add the detailed hunk helper and compatibility wrappers**

Implement the following flow in `src/authorship/stats.rs`:

```rust
fn detailed_stats_from_hunks_with_merge_flag(
    ignore_patterns: &[String],
    hunks: &[crate::commands::diff::DiffHunk],
    authorship_log: Option<&crate::authorship::authorship_log_serialization::AuthorshipLog>,
    is_merge_commit: bool,
) -> DetailedCommitStats {
    // Build total additions/deletions and deduplicated added_lines_by_file exactly once.
    // Compute AcceptedCounts from the same map.
    // Build FileStats for every non-empty entry in added_lines_by_file.
    // unknown = added_count.saturating_sub(ai).saturating_sub(known_human).
    // Build summary through stats_from_authorship_log using AcceptedCounts totals.
}

pub fn stats_for_commit_detailed(
    repo: &Repository,
    commit_sha: &str,
    ignore_patterns: &[String],
) -> Result<DetailedCommitStats, GitAiError> {
    let authorship_log = read_authorship(repo, commit_sha);
    let commit = repo.revparse_single(commit_sha)?.peel_to_commit()?;
    let parent_count = commit.parent_count()?;
    if parent_count > 1 {
        return Ok(detailed_stats_from_hunks_with_merge_flag(
            ignore_patterns,
            &[],
            authorship_log.as_ref(),
            true,
        ));
    }
    let from_ref = if parent_count == 0 {
        "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()
    } else {
        commit.parent(0)?.id()
    };
    let hunks = crate::commands::diff::get_diff_with_line_numbers(repo, &from_ref, commit_sha)?;
    Ok(detailed_stats_from_hunks_with_merge_flag(
        ignore_patterns,
        &hunks,
        authorship_log.as_ref(),
        false,
    ))
}
```

Make `stats_for_commit_stats` return `Ok(stats_for_commit_detailed(...)?.summary)`. Make `stats_for_commit_stats_from_hunks_with_merge_flag` return `.summary` from the detailed helper. Preserve the existing with-authorship helpers for callers that supply in-memory logs by routing them through the same detailed core.

- [ ] **Step 5: Run focused and existing stats tests**

Run:

```bash
task test TEST_FILTER=test_detailed_stats_group_ai_human_and_unknown_by_file NO_CAPTURE=true
task test TEST_FILTER=test_authorship_log_stats
task test TEST_FILTER=test_stats_for_mixed_commit
```

Expected: all tests pass and existing aggregate assertions remain unchanged.

- [ ] **Step 6: Commit single-commit calculation**

```bash
git add src/authorship/stats.rs tests/integration/stats_unit.rs
git commit -m "feat: calculate file-level commit stats"
```

### Task 3: Expose Single-Commit JSON Without Output Leakage

**Files:**
- Modify: `src/authorship/stats.rs:35-77`
- Modify: `tests/integration/stats.rs:11-22`
- Test: `tests/integration/stats.rs`

**Interfaces:**
- Consumes: `stats_for_commit_detailed` and `DetailedCommitStats` from Task 2.
- Produces: flat single-commit stats JSON containing `file_stats`; all aggregate-only serializers remain unchanged.

- [ ] **Step 1: Add a raw JSON helper and failing CLI assertions**

Keep the existing `stats_from_args` helper and add:

```rust
fn stats_json_from_args(repo: &TestRepo, args: &[&str]) -> serde_json::Value {
    let raw = repo.git_ai(args).expect("git-ai stats should succeed");
    serde_json::from_str(&extract_json_object(&raw)).expect("valid stats json")
}
```

Add an integration test using explicit checkpoints:

```rust
#[test]
fn test_stats_json_includes_file_stats() {
    let repo = TestRepo::new();
    std::fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "base.txt"])
        .unwrap();
    repo.stage_all_and_commit("base").unwrap();
    std::fs::create_dir_all(repo.path().join("src")).unwrap();

    repo.git_ai(&["checkpoint", "human", "src/ai.rs"]).unwrap();
    std::fs::write(repo.path().join("src/ai.rs"), "ai one\nai two\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "src/ai.rs"])
        .unwrap();
    std::fs::write(repo.path().join("src/human.rs"), "human\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "src/human.rs"])
        .unwrap();
    std::fs::write(repo.path().join("src/unknown.rs"), "unknown\n").unwrap();
    repo.stage_all_and_commit("mixed files").unwrap();

    let json = stats_json_from_args(&repo, &["stats", "HEAD", "--json"]);
    assert_eq!(json["file_stats"]["src/ai.rs"]["ai_accepted"], 2);
    assert_eq!(json["file_stats"]["src/human.rs"]["unknown_additions"], 0);
    assert_eq!(json["file_stats"]["src/unknown.rs"]["unknown_additions"], 1);
    assert!(json.get("summary").is_none());
}
```

Also add:

```rust
#[test]
fn test_status_json_does_not_include_file_stats() {
    let repo = TestRepo::new();
    repo.filename("README.md").set_contents(crate::lines!["base"]);
    repo.stage_all_and_commit("base").unwrap();
    let raw = repo.git_ai(&["status", "--json"]).unwrap();
    let json = extract_json_object(&raw);
    assert!(serde_json::from_str::<serde_json::Value>(&json).unwrap()["stats"]
        .get("file_stats")
        .is_none());
}
```

- [ ] **Step 2: Run the tests and verify the contract test fails**

Run:

```bash
task test TEST_FILTER=test_stats_json_includes_file_stats
task test TEST_FILTER=test_status_json_does_not_include_file_stats
```

Expected: the stats test fails because `file_stats` is absent; status isolation already passes.

- [ ] **Step 3: Serialize detailed data only for stats JSON**

In `stats_command`, replace the unconditional aggregate calculation with:

```rust
if json {
    let stats = stats_for_commit_detailed(repo, &target, ignore_patterns)?;
    println!("{}", serde_json::to_string(&stats)?);
} else {
    let stats = stats_for_commit_stats(repo, &target, ignore_patterns)?;
    write_stats_to_terminal(&stats, true);
}
```

Do not modify `StatusOutput`, `CommitStats`, post-commit rendering, or telemetry types.

- [ ] **Step 4: Run JSON, terminal, and status tests**

Run:

```bash
task test TEST_FILTER=test_stats_json_includes_file_stats
task test TEST_FILTER=test_status_json_does_not_include_file_stats
task test TEST_FILTER=test_stats_command_default_to_head
task test TEST_FILTER=test_terminal_output
```

Expected: all matching tests pass. Existing terminal snapshots contain no per-file table.

- [ ] **Step 5: Commit single-commit JSON output**

```bash
git add src/authorship/stats.rs tests/integration/stats.rs
git commit -m "feat: expose file stats in commit json"
```

### Task 4: Compute and Serialize Range File Statistics

**Files:**
- Modify: `src/authorship/diff_ai_accepted.rs:8-79`
- Modify: `src/authorship/range_authorship.rs:27-31,395-447`
- Modify: `src/commands/git_ai_handlers.rs:1029-1038`
- Test: `tests/integration/stats.rs`

**Interfaces:**
- Consumes: `DetailedCommitStats` and `FileStats` from Task 1.
- Produces: `DiffAiAcceptedFileStats { added_lines, ai_accepted }`, `DiffAiAcceptedStats.per_file`, and `RangeAuthorshipStats.range_stats: DetailedCommitStats`.

- [ ] **Step 1: Add a failing range JSON test**

Add a complete range test to `tests/integration/stats.rs`:

```rust
#[test]
fn test_stats_range_json_includes_file_stats() {
    let repo = TestRepo::new();
    std::fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "base.txt"])
        .unwrap();
    let base = repo.stage_all_and_commit("base").unwrap();
    std::fs::create_dir_all(repo.path().join("src")).unwrap();

    repo.git_ai(&["checkpoint", "human", "src/ai.rs"]).unwrap();
    std::fs::write(repo.path().join("src/ai.rs"), "ai one\nai two\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "src/ai.rs"])
        .unwrap();
    std::fs::write(repo.path().join("src/unknown.rs"), "unknown\n").unwrap();
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
    assert_eq!(range_stats["unknown_additions"].as_u64().unwrap(), unknown_sum);
}
```

- [ ] **Step 2: Run the range test and verify it fails**

Run:

```bash
task test TEST_FILTER=test_stats_range_json_includes_file_stats NO_CAPTURE=true
```

Expected: failure because `range_stats.file_stats` is absent.

- [ ] **Step 3: Retain range counts by file**

In `src/authorship/diff_ai_accepted.rs`, add:

```rust
#[derive(Debug, Default)]
pub struct DiffAiAcceptedFileStats {
    pub added_lines: u32,
    pub ai_accepted: u32,
}
```

Add `pub per_file: BTreeMap<String, DiffAiAcceptedFileStats>` to `DiffAiAcceptedStats`.

After sorting/deduplicating each file's `lines`, insert its `added_lines`. When a blamed line is AI accepted, increment both `total_ai_accepted` and the matching file's `ai_accepted`. Insert the file record before blame so a blame failure still produces an all-unknown file.

- [ ] **Step 4: Build detailed range stats**

Change `RangeAuthorshipStats.range_stats` to `DetailedCommitStats`. In `calculate_range_stats_direct`, construct:

```rust
let summary = stats_from_authorship_log(
    Some(&authorship_log),
    git_diff_added_lines,
    git_diff_deleted_lines,
    diff_ai_stats.total_ai_accepted,
    0,
    &diff_ai_stats.per_tool_model,
);
let file_stats = diff_ai_stats
    .per_file
    .into_iter()
    .map(|(path, file)| {
        (
            path,
            FileStats {
                ai_accepted: file.ai_accepted,
                unknown_additions: file.added_lines.saturating_sub(file.ai_accepted),
            },
        )
    })
    .collect();
let stats = DetailedCommitStats { summary, file_stats };
```

For the `start_sha == end_sha` branch, return `stats_for_commit_detailed`. In `print_range_authorship_stats`, pass `&stats.range_stats.summary` to `write_stats_to_terminal`.

Keep `RangeAuthorshipStats` deriving `Serialize` and `Deserialize`; flattened `DetailedCommitStats` makes `file_stats` appear inside `range_stats`, so the JSON branch in `git_ai_handlers.rs` can continue serializing `RangeAuthorshipStats` directly. Update imports and existing typed tests to access summary fields through `Deref` or `.summary` where required.

- [ ] **Step 5: Run range and compatibility tests**

Run:

```bash
task test TEST_FILTER=test_stats_range_json_includes_file_stats NO_CAPTURE=true
task test TEST_FILTER=test_stats_cli_range
task test TEST_FILTER=test_stats_range_uses_default_ignores
task test TEST_FILTER=test_stats_cli_empty_tree_range
```

Expected: all tests pass and existing range aggregate values are unchanged.

- [ ] **Step 6: Commit range support**

```bash
git add src/authorship/diff_ai_accepted.rs src/authorship/range_authorship.rs src/commands/git_ai_handlers.rs tests/integration/stats.rs
git commit -m "feat: expose file stats for commit ranges"
```

### Task 5: Cover Edge Cases and Verify the Feature

**Files:**
- Modify: `tests/integration/stats.rs`
- Modify if test findings require a fix: `src/authorship/stats.rs`
- Modify if test findings require a fix: `src/authorship/diff_ai_accepted.rs`
- Modify if test findings require a fix: `src/authorship/range_authorship.rs`

**Interfaces:**
- Consumes: completed single-commit and range detailed statistics.
- Produces: regression coverage for missing notes, ignored files, pure deletions/renames, UTF-8 paths, deterministic ordering, and schema isolation.

- [ ] **Step 1: Add missing-note and ignore tests**

Add these tests:

```rust
#[test]
fn test_stats_file_stats_missing_note() {
    let repo = TestRepo::new();
    std::fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    repo.stage_all_and_commit("base").unwrap();
    std::fs::write(repo.path().join("one.txt"), "one\ntwo\n").unwrap();
    std::fs::write(repo.path().join("two.txt"), "three\n").unwrap();
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
    std::fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    repo.stage_all_and_commit("base").unwrap();
    std::fs::write(repo.path().join("included.txt"), "included\n").unwrap();
    std::fs::write(repo.path().join("ignored.txt"), "ignored\n").unwrap();
    repo.stage_all_and_commit("two files").unwrap();

    let json = stats_json_from_args(
        &repo,
        &["stats", "HEAD", "--json", "--ignore", "ignored.txt"],
    );
    assert!(json["file_stats"].get("ignored.txt").is_none());
    assert_eq!(json["file_stats"]["included.txt"]["unknown_additions"], 1);
    assert_eq!(json["unknown_additions"], 1);
}
```

- [ ] **Step 2: Add path and no-addition tests**

Add a pure-rename JSON assertion to the existing rename test and add a UTF-8 ordering test:

```rust
let json = stats_json_from_args(&repo, &["stats", "HEAD", "--json"]);
assert!(json["file_stats"].as_object().unwrap().is_empty());

#[test]
fn test_stats_file_stats_preserve_utf8_paths_and_sorted_order() {
    let repo = TestRepo::new();
    std::fs::write(repo.path().join("base.txt"), "base\n").unwrap();
    repo.stage_all_and_commit("base").unwrap();
    std::fs::create_dir_all(repo.path().join("目录")).unwrap();
    std::fs::write(repo.path().join("z.rs"), "z\n").unwrap();
    std::fs::write(repo.path().join("a.rs"), "a\n").unwrap();
    std::fs::write(repo.path().join("目录/统计.rs"), "统计\n").unwrap();
    repo.stage_all_and_commit("utf8 files").unwrap();

    let json = stats_json_from_args(&repo, &["stats", "HEAD", "--json"]);
    let files = json["file_stats"].as_object().unwrap();
    assert!(files.contains_key("目录/统计.rs"));
    let keys: Vec<&String> = files.keys().collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
}
```

- [ ] **Step 3: Run edge-case tests first**

Run:

```bash
task test TEST_FILTER=test_stats_file_stats_missing_note NO_CAPTURE=true
task test TEST_FILTER=test_stats_file_stats_respect_ignore
task test TEST_FILTER=test_stats_ignores_renamed_files
task test TEST_FILTER=utf8_filenames
```

Expected: all tests pass. If any test fails, make the smallest production correction and rerun that exact test before continuing.

- [ ] **Step 4: Run formatting and lint**

Run:

```bash
task fmt
task lint
```

Expected: both commands exit successfully with no remaining formatting or lint findings.

- [ ] **Step 5: Run focused stats suites**

Run:

```bash
task test TEST_FILTER=stats
task test TEST_FILTER=diff_ai_accepted
```

Expected: all matching tests pass.

- [ ] **Step 6: Run the full suite**

Run:

```bash
task test
```

Expected: the full optimized suite passes. If the local Rust toolchain is unavailable, record the exact missing executable or setup failure and do not claim the suite passed.

- [ ] **Step 7: Commit final regressions**

```bash
git add src/authorship/stats.rs src/authorship/diff_ai_accepted.rs src/authorship/range_authorship.rs tests/integration/stats.rs
git commit -m "test: cover file-level stats edge cases"
```
