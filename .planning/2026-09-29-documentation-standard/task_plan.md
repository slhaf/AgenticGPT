# Documentation standard

## Goal
Write a project-specific documentation standard that itself follows the standard; add concise AGENTS.md guidance for when to read and update it and when to maintain affected documentation.

## Phases
1. Scope: choose standard path, rules and AGENTS handoff — complete
2. Implement: add Chinese standard and AGENTS guidance — complete
3. Verify: inspect cross-references, run a focused documentation smoke check, commit verification — complete

## Contract
`docs/documentation-standard.md` is the sole detailed convention, including a dedicated section for model-facing tool definitions with primary-source links (OpenAI, Anthropic, MCP) and distinction from human prose. `AGENTS.md` links it and lists triggers (writing/reviewing docs; changes to public behavior, CLI/config, API/tool descriptors and schemas, security/policy, release/migration, console UI or examples). Standards apply prospectively; no mass rewrite of historical docs. Human guides and machine contracts have separate authorities; paired translations stay factually aligned; release/history clearly scoped; commands/snippets copy-safe. Standard uses its own declared page shape and links to vendor sources as references, not requirements.

## Ownership
One worker owns each file: standard author owns only the new doc; AGENTS author owns only AGENTS.md. Integration owner validates and commits stages. Workers do not run build/lint/tests/formatters mid-flight.
