# Pushing authorship notes to Gerrit

Gerrit can reject a direct push of `refs/notes/ai` when the local notes history
contains commits authored or committed by `git-ai <git-ai@local>`. Enable the
Gerrit push mode for the specific remote:

```bash
git config remote.origin.git-ai-notes-push gerrit
```

Set `user.name` and `user.email` to an identity registered with the Gerrit
account used for the push. The configured identity becomes both author and
committer of the exported notes commit. Normal source-code commits and local
notes retain their existing identities.

After a successful branch push, git-ai fetches the remote notes, merges any
remote-only notes, and exports the complete local notes tree in a new commit.
The new commit has the current remote notes tip as its only parent. When the
remote has no notes ref, the exported commit has no parent. If the note content
has not changed, git-ai skips the notes push. A non-fast-forward rejection
causes git-ai to fetch and prepare the export again, up to the existing retry
limit. No force push is used.

The setting applies only to the named remote. Remotes without this setting
continue to push their local notes ref directly. Before enabling the setting
on a repository with existing notes, inspect the local note count: the first
export includes the complete local notes tree.

```bash
git notes --ref=ai list | wc -l
```
