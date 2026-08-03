/**
 * Pure helpers for Auto ARQ (VOX-only radios that cannot manually PTT a
 * second repair burst). Kept free of Android framework types for unit tests.
 */
object AutoArq {
    /** Wait after last shard progress before auto-asking for repair. */
    const val STALL_MS = 3_000L

    /** Let the peer unkey before we VOX up a repair response. */
    const val RESPONSE_DELAY_MS = 1_200L

    /**
     * Whether the receiver should fire one automatic repair request for a
     * stalled in-flight assembly. Stalled receives sit in Receiving (not Idle);
     * only local TX / RX mute should block the request.
     */
    fun shouldAutoRequest(
        autoArqEnabled: Boolean,
        transmitting: Boolean,
        rxMuted: Boolean,
        alreadyRequested: Boolean,
        received: UInt,
        total: UInt,
        msSinceProgress: Long,
        stallMs: Long = STALL_MS,
    ): Boolean {
        if (!autoArqEnabled || transmitting || rxMuted) return false
        if (alreadyRequested || received >= total) return false
        return msSinceProgress >= stallMs
    }
}
