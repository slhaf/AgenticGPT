# Deterministic tool-contract cases

`cases.json` is the provider-neutral corpus for Phase E. Each case records a
real model-misuse shape, the public tool/argument form, and the expected
descriptor or dispatch outcome. The in-tree Agent test loads this file and
exercises the actual descriptor, serde, and dispatch/dry-run path.

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

The optional evaluator accepts model predictions but never calls a provider,
reads credentials, or runs in required CI. See
`scripts/evaluate_tool_contracts.py --help`.
