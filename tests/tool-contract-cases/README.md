# Deterministic tool-contract cases

`cases.json` is the provider-neutral corpus for Phase E. Each case records a
real model-misuse shape, the public tool/argument form, and the expected
descriptor or dispatch outcome. The in-tree Agent test loads this file and
exercises the actual descriptor, serde, and dispatch/dry-run path. That
deterministic runtime corpus is separate from the optional prediction-shape
probe below.

Use `$fixtureRevision` only when a guarded edit needs the revision of the
temporary fixture created by the test harness. Keep cases bounded and free of
credentials, machine paths, network URLs, or raw secrets.

To add a regression:

1. Reproduce the invalid selection, argument, or outcome with a public tool
   call and record the smallest safe JSON shape.
2. Add a case with a stable `id`, `kind`, and expected typed code/fields.
3. Extend the harness only when a new setup or assertion shape is necessary;
   prefer existing descriptor/serde/dispatch assertions.
4. Run the focused contract test and the package/workspace gates.

## Optional prediction-shape probe

`scripts/evaluate_tool_contracts.py` is a provider-neutral **prediction-shape
probe**, not a runtime or schema gate. It reads predictions only; it never
calls a provider, reads credentials, validates JSON Schema, or dispatches a
tool. Its wildcard (`$...`) and object-subset/list-prefix matching semantics
are intentional. `--strict` means that a missing or mismatched prediction
returns exit status 1; it does not enable strict JSON Schema or runtime
validation. Probe output is labeled `probe: prediction-shape` and
`runtimeValidation: not-performed`.

Use the deterministic Agent corpus and the live/schema parity gate for those
separate guarantees. `scripts/check_contract_parity.py` is the supported
cross-surface gate: by default it uses `target/debug/agentic-gpt` and
`target/debug/agentic-gpt-hub`; `--agent-bin` and `--hub-bin` select explicit
artifacts. It validates the supported OpenAPI artifact and exercises isolated
loopback/private-home live processes. Schema validation and live behavior are
separate checks; neither is supplied by the prediction probe.
