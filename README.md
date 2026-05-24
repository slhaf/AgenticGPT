# Agentic GPT

Agentic GPT is a Linux local agent plus Cloudflare Worker gateway for controlled GPT Actions execution.

## Layout

- `apps/worker`: Cloudflare Worker, Durable Object, KV-backed agent registry, OpenAPI schema.
- `crates/agentic-gpt`: Rust CLI local agent.
- `dist`: release artifact output.

## Local Agent

```bash
cargo run -p agentic-gpt -- config init
cargo run -p agentic-gpt -- config set workerUrl http://localhost:8787
cargo run -p agentic-gpt -- config set agentId laptop
cargo run -p agentic-gpt -- config set agentSecret '<agent-secret>'
cargo run -p agentic-gpt -- run
```

Config lives at `~/.agentic_gpt/config.json`; audit logs are JSONL at `~/.agentic_gpt/audit.log`.

## Worker

```bash
pnpm install
pnpm --filter worker build
pnpm --filter worker wrangler types
pnpm --filter worker wrangler deploy
```

Set the Worker secret `API_KEY` and create an `AGENT_REGISTRY` KV namespace. Agent entries are stored under `agent:<agentId>` and contain `secretHash`, where `secretHash` is SHA-256 hex of the local `agentSecret`.

## Verification

```bash
pnpm --filter worker build
pnpm --filter worker test
cargo test --workspace
```
