# Progress

- Chose `docs/documentation-standard.md` as canonical editorial guide and a concise AGENTS.md trigger section. Initial repository worktree was clean.
- User clarified that documentation must use Chinese and requested a dedicated tool-definition section. Added the Chinese standard and Chinese `AGENTS.md` section; existing English product docs remain untouched, with future new/revised prose in Chinese.
- Tool guidance links official OpenAI plugin tool-planning, Anthropic tool definitions and the versioned MCP specification; general editing links Google, Microsoft and GitHub. The versioned MCP source URL resolves.
- Verified local Markdown links: AGENTS.md 1 valid relative link; the standard has no local links. Focused checks confirmed Chinese title/language rule, model-facing tool section and three cited sources, and AGENTS read trigger. First ad hoc assertion used `新建` instead of the actual `新增` wording and failed; correcting the assertion passed without changing the product files. `git diff HEAD^ HEAD --check` passed with a clean worktree.
