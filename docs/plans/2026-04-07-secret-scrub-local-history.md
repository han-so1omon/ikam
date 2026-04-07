# Local Secret Scrub Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove the committed secret-bearing `.env` file from git history, prevent future recommits, restore the local `.env` as untracked, and verify the repository no longer contains the exposed token values.

**Architecture:** This cleanup keeps `.env.example` as the tracked template and treats `.env` as local-only runtime state. Because the repository is local-only with no collaborator resync constraints, the cleanup uses `git-filter-repo` to rewrite history in place, then restores the local `.env` outside git tracking.

**Tech Stack:** git, git-filter-repo, zsh, pytest, local filesystem

---

### Task 1: Preserve the current local `.env` outside git history

**Files:**
- Read: `packages/test/ikam-perf-report/.env`
- Create: local backup path outside repository root

**Step 1: Verify the source file exists**

Run: `ls "packages/test/ikam-perf-report/.env"`
Expected: file exists

**Step 2: Copy it to a safe path outside the repo**

Run: `cp "packages/test/ikam-perf-report/.env" "/tmp/ikam-perf-report.env.backup"`
Expected: backup file created

**Step 3: Verify the backup exists**

Run: `ls "/tmp/ikam-perf-report.env.backup"`
Expected: backup file exists

### Task 2: Ignore `.env` files while keeping examples tracked

**Files:**
- Modify: `.gitignore`

**Step 1: Add failing policy expectation mentally before edit**

Expected end state:
- `.env` ignored repo-wide
- `.env.*` ignored repo-wide
- `.env.example` remains tracked
- explicit package path ignored: `packages/test/ikam-perf-report/.env`

**Step 2: Update `.gitignore`**

Add rules such as:

```gitignore
.env
.env.*
!.env.example
packages/test/ikam-perf-report/.env
```

**Step 3: Verify ignore behavior**

Run: `git status --short -- "packages/test/ikam-perf-report/.env"`
Expected: no tracked-change output after restore step later

### Task 3: Rewrite history to remove the committed secret file

**Files:**
- Rewrite history for: `packages/test/ikam-perf-report/.env`

**Step 1: Confirm current history includes the file**

Run: `git log --all --oneline -- "packages/test/ikam-perf-report/.env"`
Expected: at least one commit references the file before rewrite

**Step 2: Run `git-filter-repo`**

Run:

```bash
git filter-repo --invert-paths --path "packages/test/ikam-perf-report/.env" --force
```

Expected: history rewritten successfully

**Step 3: Verify the file is gone from history**

Run: `git log --all --oneline -- "packages/test/ikam-perf-report/.env"`
Expected: no output

### Task 4: Restore the local `.env` as untracked runtime state

**Files:**
- Restore: `packages/test/ikam-perf-report/.env`
- Source: `/tmp/ikam-perf-report.env.backup`

**Step 1: Copy the backup back into place**

Run: `cp "/tmp/ikam-perf-report.env.backup" "packages/test/ikam-perf-report/.env"`
Expected: local `.env` restored

**Step 2: Verify git ignores it**

Run: `git status --short -- "packages/test/ikam-perf-report/.env"`
Expected: no output

### Task 5: Verify the repository no longer exposes the token values

**Files:**
- Verify history and working tree

**Step 1: Search git history for the exact OpenAI token**

Run: `git grep -n "<exact-openai-token>" $(git rev-list --all)`
Expected: no matches

**Step 2: Search git history for the exact Hugging Face token**

Run: `git grep -n "<exact-hf-token>" $(git rev-list --all)`
Expected: no matches

**Step 3: Search current tracked tree for the same strings**

Run: `git grep -n "<exact-openai-token>"` and `git grep -n "<exact-hf-token>"`
Expected: no matches

**Step 4: Re-run targeted secret audit sanity check**

Run a focused grep for common token patterns across first-party files.
Expected: no committed provider tokens remain; only local/test defaults or placeholders may remain.

### Task 6: Record final git state

**Files:**
- No file modifications required unless additional ignore fixes are needed

**Step 1: Check status**

Run: `git status --short --branch`
Expected: only intended tracked changes, or clean status after rewrite depending on git-filter-repo behavior

**Step 2: Check log**

Run: `git log --oneline --decorate --graph --all -n 5`
Expected: rewritten history present without the scrubbed file

### Notes

- This plan deliberately does **not** rotate keys first, per the chosen path: scrub first, skip rotation if local-only.
- If verification finds the token strings anywhere in history after rewrite, stop and rotate immediately.
- If a remote is added later, push only the rewritten history.
