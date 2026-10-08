# Kotlin Test Cases

All 7 Kotlin declarations are listed once. `commonTest` cases are not multiplied by JVM, JS, Wasm, or Android target execution.

| Source path | Test function | Tier | Asserted behavior |
|---|---|---:|---|
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt` | `overdue_restore_claim_is_trigger_once_at_due_boundary` | T3 | At the due boundary, restore chooses Trigger; triggering produces Triggered; a second trigger is rejected and the terminal/triggered item is ignored on restore. |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt` | `future_restore_schedules_until_snoozed_due_boundary` | T3 | Future restore schedules; snoozing moves the due time; restore schedules just before the boundary and triggers exactly at it. |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt` | `degraded_item_stays_pending_until_due_and_can_be_terminal` | T3 | A Degraded item schedules before due, triggers at due, and is then excluded from pending restoration after becoming Failed. |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt` | `terminal_transition_clears_actions_and_rejects_duplicate_transition` | T3 | Transitioning to Done clears actions and updates status; a repeated terminal transition returns null. |
| `console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/SharedCommonTest.kt` | `example` | T1 | Asserts the fixed arithmetic expression `1 + 2 == 3`; no application component is exercised. |
| `console/shared/src/jvmTest/kotlin/work/slhaf/agentic/console/SharedLogicDesktopTest.kt` | `example` | T1 | Asserts the fixed arithmetic expression `1 + 2 == 3`; no application component is exercised. |
| `console/shared/src/androidHostTest/kotlin/work/slhaf/agentic/console/SharedLogicAndroidHostTest.kt` | `example` | T1 | Asserts the fixed arithmetic expression `1 + 2 == 3`; no application component is exercised. |

Evidence: declarations and assertions in the listed source files; policy implementation in `console/shared/src/commonMain/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicy.kt`. The source-set and Gradle target reconciliation is recorded in `findings.md`.