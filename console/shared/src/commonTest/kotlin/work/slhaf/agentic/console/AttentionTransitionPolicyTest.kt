package work.slhaf.agentic.console

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import work.slhaf.agentic.console.domain.attention.AttentionAction
import work.slhaf.agentic.console.domain.attention.AttentionDemoData
import work.slhaf.agentic.console.domain.attention.AttentionRestoreAction
import work.slhaf.agentic.console.domain.attention.AttentionStatus
import work.slhaf.agentic.console.domain.attention.AttentionTransitionPolicy
import kotlin.time.Duration.Companion.minutes

class AttentionTransitionPolicyTest {
    @Test
    fun overdue_restore_claim_is_trigger_once_at_due_boundary() {
        val now = 1_000_000L
        val item = AttentionDemoData.createMockReminder(afterMinutes = 0, nowEpochMillis = now)

        assertEquals(
            AttentionRestoreAction.Trigger,
            AttentionTransitionPolicy.restoreAction(item, now),
        )

        val triggered = AttentionTransitionPolicy.triggered(item, now)
        assertEquals(AttentionStatus.Triggered, triggered?.status)
        assertNull(AttentionTransitionPolicy.triggered(triggered!!, now))
        assertEquals(
            AttentionRestoreAction.Ignore,
            AttentionTransitionPolicy.restoreAction(triggered, now),
        )
    }

    @Test
    fun future_restore_schedules_until_snoozed_due_boundary() {
        val createdAt = 2_000_000L
        val item = AttentionDemoData.createMockAlarm(afterMinutes = 10, nowEpochMillis = createdAt)
        val snoozeAt = createdAt + 1.minutes.inWholeMilliseconds
        val snoozed = requireNotNull(AttentionTransitionPolicy.snoozed(item, 5.minutes, snoozeAt))

        assertEquals(AttentionRestoreAction.Schedule, AttentionTransitionPolicy.restoreAction(item, createdAt))
        assertEquals(AttentionStatus.Snoozed, snoozed.status)
        assertEquals(
            AttentionRestoreAction.Schedule,
            AttentionTransitionPolicy.restoreAction(snoozed, snoozed.dueAtEpochMillis - 1),
        )
        assertEquals(
            AttentionRestoreAction.Trigger,
            AttentionTransitionPolicy.restoreAction(snoozed, snoozed.dueAtEpochMillis),
        )
    }

    @Test
    fun degraded_item_stays_pending_until_due_and_can_be_terminal() {
        val now = 4_000_000L
        val item = AttentionDemoData.createMockReminder(
            afterMinutes = 1,
            nowEpochMillis = now,
        ).copy(status = AttentionStatus.Degraded)

        assertEquals(AttentionRestoreAction.Schedule, AttentionTransitionPolicy.restoreAction(item, now))
        val overdue = item.copy(dueAtEpochMillis = now)
        assertEquals(AttentionRestoreAction.Trigger, AttentionTransitionPolicy.restoreAction(overdue, now))

        val failed = requireNotNull(AttentionTransitionPolicy.terminal(item, AttentionStatus.Failed, now))
        assertEquals(AttentionStatus.Failed, failed.status)
        assertEquals(AttentionRestoreAction.Ignore, AttentionTransitionPolicy.restoreAction(failed, now))
    }

    @Test
    fun terminal_transition_clears_actions_and_rejects_duplicate_transition() {
        val now = 3_000_000L
        val item = AttentionDemoData.createMockReminder(afterMinutes = 1, nowEpochMillis = now)
        val done = AttentionTransitionPolicy.terminal(item, AttentionStatus.Done, now)

        assertEquals(emptyList<AttentionAction>(), done?.actions)
        assertEquals(AttentionStatus.Done, done?.status)
        assertNull(AttentionTransitionPolicy.terminal(done!!, AttentionStatus.Done, now + 1))
    }
}
