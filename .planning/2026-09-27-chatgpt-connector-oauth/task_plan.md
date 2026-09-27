# ChatGPT Custom Connector OAuth compatibility

## Goal
Compare current official ChatGPT Custom Connector OAuth requirements with HTTP MCP implementation, identify demonstrable deviations and likely connection blockers, and report verification limits.

## Phases
1. Research — complete: official contract, server route inventory, demonstrated mismatch and conditional deployment blocker.
2. Verification — complete: real-worker integration test passed; public ChatGPT connection not observable from local environment. Research and verification committed separately.

## Decisions
- Official OpenAI sources govern ChatGPT behavior; preserve MCP protocol and existing authentication safety.
- This is an implementation audit, not an explicit fix request; the demonstrated CIMD validation defect is security-relevant but permissive, so cannot explain failed connection by itself. Do not alter OAuth behavior without a proven connection failure stage; report the distinction clearly.

## Errors Encountered
- Planning helper absent from usual ~/.claude skill path; use task-local files directly.
