# File-Level AI Acceptance and Unknown Statistics

## Context

`git-ai stats` currently reports only aggregate commit or range statistics. The attribution pipeline already retains changed file paths and added line numbers while computing `ai_accepted`, known-human additions, and `unknown_additions`, but it discards the per-file grouping before serializing the result.

Users need the same AI acceptance and unknown-addition counts grouped by file so downstream tooling can identify which files contain accepted AI work or unattributed additions.

## Scope

The first release will:

- add file-level `ai_accepted` and `unknown_additions` to `git-ai stats --json`;
- support both a single commit and a commit range;
- preserve every existing aggregate JSON field and its current meaning;
- leave interactive terminal output, `git-ai status`, post-commit output, authorship notes, and telemetry payloads unchanged;
- honor the same ignore rules, path normalization, diff interpretation, and attribution rules as aggregate statistics.

The first release will not:

- add per-file human, added-line, deleted-line, tool, model, prompt, or percentage breakdowns;
- display a per-file table in terminal output;
- change existing range-statistics semantics for known-human lines;
- change the authorship note schema;
- persist file statistics separately from the existing attribution data.

## JSON Contract

Single-commit JSON remains a flat aggregate object and gains a `file_stats` field:

```json
{
  "human_additions": 4,
  "unknown_additions": 3,
  "ai_additions": 8,
  "ai_accepted": 8,
  "git_diff_deleted_lines": 2,
  "git_diff_added_lines": 15,
  "tool_model_breakdown": {},
  "file_stats": {
    "src/lib.rs": {
      "ai_accepted": 6,
      "unknown_additions": 1
    },
    "src/main.rs": {
      "ai_accepted": 2,
      "unknown_additions": 2
    }
  }
}
```

Range JSON keeps its existing outer shape and adds `file_stats` inside `range_stats`:

```json
{
  "authorship_stats": {
    "total_commits": 2,
    "commits_with_authorship": 2
  },
  "range_stats": {
    "human_additions": 0,
    "unknown_additions": 3,
    "ai_additions": 8,
    "ai_accepted": 8,
    "git_diff_deleted_lines": 2,
    "git_diff_added_lines": 11,
    "tool_model_breakdown": {},
    "file_stats": {
      "src/lib.rs": {
        "ai_accepted": 6,
        "unknown_additions": 1
      },
      "src/main.rs": {
        "ai_accepted": 2,
        "unknown_additions": 2
      }
    }
  }
}
```

`file_stats` is an object keyed by the repository-relative, POSIX-normalized file path. A sorted map will provide deterministic serialization.

Only files with at least one included added line appear. A file containing only known-human additions appears with both requested values set to zero. Pure deletion files and ignored files do not appear.

## Metric Semantics

For each included file:

- `ai_accepted` is the number of added lines in the selected diff that overlap an AI attestation under the existing acceptance rules.
- `unknown_additions` is the number of included added lines remaining after subtracting AI-accepted and known-human-attested added lines under the existing command mode's semantics.

For single-commit statistics:

```text
file unknown = file added - file AI accepted - file known-human accepted
```

For range statistics, the implementation must preserve the existing range behavior, which currently supplies zero known-human accepted lines to aggregate calculation. Therefore the per-file range calculation must mirror that behavior instead of changing aggregate results as part of this feature.

The following invariants must hold for every successful JSON result:

```text
aggregate ai_accepted = sum(file_stats[*].ai_accepted)
aggregate unknown_additions = sum(file_stats[*].unknown_additions)
```

When no authorship log or usable attribution exists, every included added line is unknown. Saturating arithmetic must prevent malformed or overlapping attribution data from producing negative-equivalent counts.

## Architecture

### Output isolation

The core aggregate `CommitStats` type is shared by stats, status, post-commit, and other internal paths. The feature must not make file-level data leak into those other serialized outputs.

Introduce a detailed stats result containing:

- the existing aggregate `CommitStats`;
- a deterministic map from file path to a small file-stat record.

Existing aggregate helpers remain available to current callers and return only `CommitStats`. Detailed helpers are used by the JSON stats command. A stats-specific serialization DTO flattens the aggregate fields and adds `file_stats`, preserving the current single-commit shape. The range JSON path uses an equivalent DTO so `file_stats` is nested inside `range_stats`.

This boundary avoids changing `status`, post-commit, terminal, or telemetry schemas and avoids relying on ad hoc mutation of `serde_json::Value`.

### Single-commit data flow

1. Obtain diff hunks using the existing line-number-aware diff implementation.
2. Apply the existing ignore matcher before counting or grouping lines.
3. Group and deduplicate added line numbers by normalized file path.
4. Intersect each file's added lines with that file's AI and known-human attestations.
5. Build per-file counts and derive aggregate counts from the same intermediate values.
6. Serialize the detailed result only for `git-ai stats --json`; terminal output consumes the aggregate portion.

The implementation should extend the existing hunk and attestation passes rather than invoking Git once per file.

### Range data flow

1. Obtain the range diff and its file-level added lines using the existing range endpoints.
2. Extend diff-based AI acceptance calculation to retain accepted counts by file while preserving the current total, tool/model, and prompt results.
3. Derive file-level unknown values from the same per-file diff counts and the current range semantics.
4. Build aggregate totals from the same detailed result and serialize through the range-specific JSON DTO.

No per-file `git blame` subprocess loop should be introduced beyond the existing file-oriented range blame behavior. The feature must retain the current asymptotic behavior and avoid adding a second complete stats calculation.

## Error and Edge-Case Behavior

- Missing authorship data: report zero AI accepted and all included additions as unknown for each file.
- Ignored files: omit them from both file and aggregate statistics.
- Duplicate hunk line numbers: sort and deduplicate before matching attestations.
- Renames: use the destination path emitted by the existing diff parser.
- UTF-8 paths: preserve the existing normalized path string without lossy conversion.
- Binary and pure deletion files: omit them when they have no countable added lines.
- Merge commits: preserve existing aggregate behavior; file statistics must be empty when the existing merge path supplies no hunks.
- Empty additions: return an empty `file_stats` object.
- Attribution overlap or inconsistent data: use the same matching precedence and saturating subtraction as aggregate statistics.

## Requirements

- REQ-001: When a user runs `git-ai stats --json` for a single commit, the system shall return `ai_accepted` and `unknown_additions` grouped by included file.
- REQ-002: When a user runs `git-ai stats --json` for a commit range, the system shall return the same file grouping inside `range_stats`.
- REQ-003: The system shall calculate aggregate and file statistics from one consistent set of diff and attribution inputs.
- REQ-004: The system shall preserve all existing aggregate field names and values for equivalent inputs.
- REQ-005: The system shall exclude ignored files from file-level and aggregate counts.
- REQ-006: The system shall not add file-level fields to terminal, status, post-commit, authorship-note, or telemetry output.
- REQ-007: The system shall serialize file paths deterministically.

## Acceptance Scenarios

### Multiple files with mixed attribution

```gherkin
Given a commit adds AI, known-human, and unattested lines across multiple files
When the user runs git-ai stats HEAD --json
Then each file with additions appears in file_stats
And each file reports the correct ai_accepted and unknown_additions values
And aggregate AI and unknown values equal the sums of their file values
```

### Missing authorship note

```gherkin
Given a commit adds lines to two files and has no authorship note
When the user runs git-ai stats HEAD --json
Then each file reports ai_accepted as zero
And each file reports all its added lines as unknown_additions
```

### Ignore patterns

```gherkin
Given a commit changes an included source file and an ignored generated file
When the user runs git-ai stats HEAD --json with the applicable ignore pattern
Then file_stats omits the ignored file
And aggregate values also omit the ignored file
```

### Commit range

```gherkin
Given a commit range leaves accepted AI and unknown lines in multiple files
When the user runs git-ai stats START..END --json
Then range_stats contains file_stats
And its per-file sums equal the existing range aggregate values
```

### Non-stats output compatibility

```gherkin
Given file-level statistics are available for a commit
When the user runs terminal stats, status JSON, or observes post-commit output
Then those outputs retain their previous schemas and formatting
```

## Verification Plan

Add focused unit and integration coverage for:

- detailed single-commit calculation across two or more files;
- AI, known-human, and unknown lines in the same file and in separate files;
- absent authorship notes;
- ignored paths;
- empty additions and pure deletions;
- range JSON shape and per-file aggregate invariants;
- rename and UTF-8 file paths using existing diff behavior;
- confirmation that status JSON and terminal output do not gain file-level data.

Run formatting and lint checks required by the repository, targeted stats tests during development, and the full optimized test suite before completion when the local Rust toolchain is available.

## Implementation Sequence

1. Add failing tests for JSON shape, values, invariants, and output isolation.
2. Introduce the file-stat and detailed-result types.
3. Extend single-commit hunk and attestation aggregation.
4. Extend range AI acceptance and unknown aggregation.
5. Add stats-specific single and range serialization DTOs.
6. Cover edge cases and compatibility paths.
7. Run format, lint, targeted tests, and the full suite.
