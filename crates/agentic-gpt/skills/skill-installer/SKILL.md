---
name: skill-installer
description: Install and manage Room-scoped skills through the skills MCP tools.
version: 1
tags:
  - skills
  - installation
  - room
---

# Skill installer

Use this skill to guide installation of another skill in the active Room Agent.
Skills are managed by the Room Agent, so these tools do not take an `agentId`.

## Recommended workflow

1. Inspect the available skills with `skills.list` or read this guide with
   `skills.read`.
2. Start one installation with `skills.install`. Always provide the target
   `id` explicitly. Use a structured public GitHub source when installing from
   a repository, or use the `files` source for URL-backed and inline files.
3. Treat the immediate response as an acknowledgement. Keep its `installId`
   and poll `skills.install.get`; status lookup waits up to five seconds by
   default and accepts `waitSeconds` from 0 through 30.
4. Use `skills.install.cancel` when queued or cancellable work should stop.
   Cancellation is cooperative and cannot interrupt an atomic commit.
5. After completion, call `skills.read` to inspect `SKILL.md` or a safe
   package-relative resource. A newly installed skill is active by default.

## Replacement and retry rules

Existing skills are not replaced unless `replaceExisting: true` is explicit.
Replacement archives the old package under `skills/.archive` so it can be
restored if the commit fails. An optional Room-scoped `idempotencyKey` makes a
fresh retry return the original installation instead of creating a duplicate.

Only public GitHub repositories and public HTTPS file URLs are supported in
the initial version. Do not put credentials in URLs. Inline `content` and
`contentBase64` remain available for generated files. Installation validates
the package and never executes its scripts.

## Running installed scripts

For an active workspace-backed skill, `skills.run` accepts the skill `id`, a
package-relative executable path below `scripts/`, optional direct `args`, an
optional policy-validated `workingDirectory`, and a bounded `waitSeconds`.
It returns a managed Job envelope. Fast terminal executions are included in
the response; otherwise use `job.get` with bounded `waitSeconds` and
`job.cancel` with that `jobId`.
