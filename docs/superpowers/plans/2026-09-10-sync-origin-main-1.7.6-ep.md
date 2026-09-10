# Sync `origin/main` and release `v1.7.6-ep`

## Goal

Make `fork/main` the maintained EP mainline, merge the current `origin/main`, preserve the fork-only attribution recovery and file-level statistics work, and publish a traceable `v1.7.6-ep` release to the `rebasefix` S3 channel.

## Starting point

- Fork main: `8726112af97d3d87ddab7b6f7de4b310d883079e` (`1.6.24-ep`)
- Upstream main: `0670e7ef27590af0e8ff5409267f3f4b09b8fcb4` (`1.7.6`)
- Merge base: `c57c6c24be03ba89ce7d47783841d2f49ba8ee45` (`1.6.22`)
- Divergence: 33 fork-only commits and 227 upstream-only commits
- Integration branch: `codex/sync-origin-main-1.7.6-ep`

## Execution checklist

- [x] Run focused pre-merge regression tests on `fork/main`.
- [x] Merge `origin/main` with a merge commit.
- [x] Resolve textual conflicts without dropping EP or upstream behavior.
- [x] Review critical-path auto-merges in rewrite and trace2 code.
- [x] Run focused post-merge regression tests.
- [x] Bump `Cargo.toml`, `Cargo.lock`, and `flake.nix` to `1.7.6-ep`.
- [x] Run format, lint, build, version, and full test verification.
- [x] Push the integration branch and fast-forward `fork/main` after verification.
- [x] Create and push annotated tag `v1.7.6-ep` at the verified release commit.
- [x] Build and verify four GitHub Actions release artifacts.
- [x] Dry-run packaging, upload to S3, update `rebasefix`, and reinstall to verify.

## Conflict policy

- Preserve upstream architecture and new functionality by default.
- Reapply fork-only behavior narrowly where upstream has not superseded it.
- Never resolve `src/daemon.rs` or trace2/rewrite code by choosing an entire side.
- Do not introduce git work on trace2 ingestion paths or unbounded/per-item git spawns.
- Keep the merge commit at upstream version `1.7.6`; make the EP suffix a separate release commit.

## Progress log

- 2026-09-10: Created a clean clone because the original checkout has `core.bare=true` and a divergent index/worktree.
- 2026-09-10: Refreshed both remotes and created the integration branch from `fork/main`.
- 2026-09-10: Pre-merge rebase diagnostics, cold trace2 rebase recovery, and file-level statistics tests passed.
- 2026-09-10: Resolved six textual conflicts by combining behavior; kept upstream `1.7.6` for the merge commit.
- 2026-09-10: Reviewed fork-only deltas against `origin/main`; no new git work was introduced on ingestion paths.
- 2026-09-10: Post-merge diagnostics, recovery, file-statistics, and update-ref stdin regression tests passed.
- 2026-09-10: Created the separate EP version change for `1.7.6-ep` in all three version sources.
- 2026-09-10: `task fmt`, `task lint`, `task build`, the debug version check, the remaining standalone test binaries, and doc tests passed.
- 2026-09-10: The complete integration target finished with 3,377 passed, 89 ignored, and two checked-out `update-ref` timing failures. Both failures reproduce unchanged on a clean `origin/main` baseline under the local Homebrew Git 2.48.0 environment, so they are not merge regressions; hosted CI remains the release gate.
- 2026-09-10: The first hosted macOS core run exposed a fork-only interaction: rewrite diagnostics were enabled by default and interfered with metrics reingestion. Changed diagnostics to explicit opt-in, kept the diagnostics test enabled through a scoped daemon environment, and verified both regression tests plus lint locally.
- 2026-09-10: Hosted run `34434606490` exposed a separate macOS test-fixture race: the mock API serialized connections while metrics and fire-and-forget daemon-log uploads are intentionally concurrent. Updated the mock to serve connections concurrently; the reingest regression passed 10 consecutive runs and the complete 117-test `daemon_mode` target passed locally.
- 2026-09-10: Hosted Test run `34435793053` passed all 17 jobs on source commit `e5b0778a7c4b74558b3d89214e3775cc67068363`, including Ubuntu, macOS, and Windows core/integration shards.
- 2026-09-10: Fast-forwarded `fork/main` and pushed the integration branch through documentation commit `f1ed32dd682c77f6bd5f3d6c318ee3c3fb02ba94`; pushed annotated tag `v1.7.6-ep`, which peels to that verified release commit.
- 2026-09-10: S3 Release Build run `34438108023` passed all platform jobs on `f1ed32dd682c77f6bd5f3d6c318ee3c3fb02ba94`. The aggregate artifact and four independently downloaded artifacts matched byte-for-byte, and the workflow-provided `SHA256SUMS` verified every binary and installer.
- 2026-09-10: Verified release binary SHA-256 values: Linux ARM64 `c7c3e1aa17ec94e82cb99e6bbc7d3f61dcb3bea0c6203f6e0c5629144791c541`, Linux x64 `71b3dc4a35cf6ad9a18b231c2f60ab138ff1aaf4942b08ef020cffd2769bee18`, macOS ARM64 `99550aa8ab05f21061a73d7a263359715cec8ccf646e051b856917a8a10fb94a`, and macOS x64 `345768fd894bed0802e32cd888bc579e89e1cecf8729b278fa18087d2dfe2c4d`.
- 2026-09-10: Dry-run packaging verified both immutable-release and channel checksums and confirmed both installers pin `v1.7.6-ep`. Uploaded `v1.7.6-ep/` and refreshed `channels/rebasefix/` in `ep-zadig-prod`, then read both manifests back through the public endpoint.
- 2026-09-10: Installed from the public `rebasefix` channel. `/Users/congziqi/.git-ai/bin/git-ai`, `/Users/congziqi/.local/bin/git-ai`, the active `git-ai`, and the running daemon all verified as healthy version `1.7.6-ep`.
