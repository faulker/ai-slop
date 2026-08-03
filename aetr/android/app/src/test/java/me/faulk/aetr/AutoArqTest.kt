package me.faulk.aetr

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for Auto ARQ stall gating (VOX-only repair path). */
class AutoArqTest {

    /** Builds a shouldAutoRequest call with defaults matching a stalled receive. */
    private fun check(
        autoArqEnabled: Boolean = true,
        transmitting: Boolean = false,
        rxMuted: Boolean = false,
        alreadyRequested: Boolean = false,
        received: UInt = 2u,
        total: UInt = 5u,
        msSinceProgress: Long = AutoArq.STALL_MS,
    ): Boolean = AutoArq.shouldAutoRequest(
        autoArqEnabled = autoArqEnabled,
        transmitting = transmitting,
        rxMuted = rxMuted,
        alreadyRequested = alreadyRequested,
        received = received,
        total = total,
        msSinceProgress = msSinceProgress,
    )

    @Test
    fun requestsWhenStalledEvenWhileReceiving() {
        // Pending shards put rxState at Receiving; that must not block Auto ARQ.
        assertTrue(check())
    }

    @Test
    fun skipsWhenDisabledOrBusyOrComplete() {
        assertFalse(check(autoArqEnabled = false))
        assertFalse(check(transmitting = true))
        assertFalse(check(rxMuted = true))
        assertFalse(check(alreadyRequested = true))
        assertFalse(check(received = 5u, total = 5u))
        assertFalse(check(msSinceProgress = AutoArq.STALL_MS - 1))
    }
}
