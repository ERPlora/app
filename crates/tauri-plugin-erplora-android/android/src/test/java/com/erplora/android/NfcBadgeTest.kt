package com.erplora.android

import android.nfc.NfcAdapter
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * The decidable half of reading a staff badge off the tablet's own NFC reader (hub#988).
 *
 * Same split as `BluetoothSppTest`: the radio can only be proven with a card in a hand, and that
 * validation is booked in pm#145. What CAN be pinned without hardware is every decision around the
 * tap — how a UID becomes the badge string the hub already stores, which UIDs are refused because
 * they would enrol a badge that never matches again, and how long the reader waits.
 */
class NfcBadgeTest {

    // ── The UID becomes the badge string ─────────────────────────────────────────────────────
    //
    // One spelling, decided here and nowhere else. Whatever it is, it has to survive the shell's
    // own `BADGE_SHAPE` (`[A-Za-z0-9_-]{4,64}`) and the runtime's HMAC index unchanged: a second
    // rendering of the same card is a badge that does not open the till.

    @Test
    fun `a uid becomes uppercase hex with no separators`() {
        // Upper case and unpunctuated is what Android's own tooling prints and what every reader
        // datasheet quotes, so the number on the screen matches the number on the card's label.
        assertEquals("04A23B5C6D7E80", NfcBadge.toBadge(bytes(0x04, 0xA2, 0x3B, 0x5C, 0x6D, 0x7E, 0x80)))
    }

    @Test
    fun `a byte below sixteen keeps its leading zero`() {
        // `0x0A` rendered as "A" would collide with a different card whose next byte starts with A.
        // The pad is what makes the string a faithful reading of the bytes.
        assertEquals("000A0B0C", NfcBadge.toBadge(bytes(0x00, 0x0A, 0x0B, 0x0C)))
    }

    @Test
    fun `the badge string fits what the hub accepts as a badge`() {
        // A 10-byte UID — the longest ISO 14443 triple-size — is 20 characters, well inside the 64
        // the enrolment field allows, and uses only characters the shape permits.
        val badge = NfcBadge.toBadge(bytes(0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99))
        assertEquals(20, badge.length)
        assertTrue(Regex("^[A-Za-z0-9_-]{4,64}$").matches(badge), badge)
    }

    // ── Which cards may be enrolled at all ───────────────────────────────────────────────────

    @Test
    fun `a random anti-collision uid is refused`() {
        // ISO 14443-3: a SINGLE-size (4 byte) UID whose first byte is 0x08 is a Random ID — the
        // card mints a new one on every tap. Modern DESFire, most Ultralight and every phone in
        // card-emulation mode do this. Accepting one would enrol a badge that can never match
        // again, and the employee would be locked out by a card that "worked when we set it up".
        assertNull(NfcBadge.badgeOf(bytes(0x08, 0x1A, 0x2B, 0x3C)))
        assertFalse(NfcBadge.isStableUid(bytes(0x08, 0x1A, 0x2B, 0x3C)))
    }

    @Test
    fun `0x08 only means random on a four-byte uid`() {
        // A 7-byte UID starting with 0x08 is a real manufacturer id, not a random one — refusing it
        // would reject perfectly good stock. The rule is about the SIZE as much as the byte.
        assertTrue(NfcBadge.isStableUid(bytes(0x08, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66)))
        assertEquals("08112233445566", NfcBadge.badgeOf(bytes(0x08, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66)))
    }

    @Test
    fun `an empty or absent uid is not a card`() {
        // A tag whose technology carries no id at all. There is nothing to enrol and nothing to
        // match, so it must not travel back as an empty badge the field would then "fill in".
        assertNull(NfcBadge.badgeOf(null))
        assertNull(NfcBadge.badgeOf(ByteArray(0)))
    }

    @Test
    fun `an all-zero uid is refused`() {
        // What a misread or a half-powered field produces. It is also the same "badge" for every
        // card that ever misreads, which is the one collision that must never reach the HMAC index.
        assertNull(NfcBadge.badgeOf(bytes(0x00, 0x00, 0x00, 0x00)))
    }

    @Test
    fun `a uid too short to be a card is refused`() {
        // The shortest real UID is 4 bytes (ISO 14443-3 single size). Anything under that is noise,
        // and it would also fall under the 4-character minimum the badge field enforces.
        assertNull(NfcBadge.badgeOf(bytes(0x04, 0x11, 0x22)))
    }

    @Test
    fun `an ordinary card is accepted`() {
        assertEquals("04A23B5C6D7E80", NfcBadge.badgeOf(bytes(0x04, 0xA2, 0x3B, 0x5C, 0x6D, 0x7E, 0x80)))
        assertEquals("DEADBEEF", NfcBadge.badgeOf(bytes(0xDE, 0xAD, 0xBE, 0xEF)))
    }

    // ── How long the reader waits ────────────────────────────────────────────────────────────

    @Test
    fun `an absent timeout falls back to the default`() {
        assertEquals(NfcBadge.DEFAULT_TIMEOUT_MS, NfcBadge.clampTimeout(null))
    }

    @Test
    fun `a timeout the caller asked for is honoured`() {
        assertEquals(8_000L, NfcBadge.clampTimeout(8_000L))
    }

    @Test
    fun `an absurd timeout is clamped instead of refused`() {
        // Refusing would leave the shell with no reader at all over an argument nobody chose by
        // hand. Zero would spin the radio up and down forever; ten minutes would hold reader mode
        // open long after the screen that asked for it is gone.
        assertEquals(NfcBadge.MIN_TIMEOUT_MS, NfcBadge.clampTimeout(0L))
        assertEquals(NfcBadge.MIN_TIMEOUT_MS, NfcBadge.clampTimeout(-1L))
        assertEquals(NfcBadge.MAX_TIMEOUT_MS, NfcBadge.clampTimeout(600_000L))
    }

    @Test
    fun `the default timeout is inside its own bounds`() {
        assertTrue(NfcBadge.DEFAULT_TIMEOUT_MS in NfcBadge.MIN_TIMEOUT_MS..NfcBadge.MAX_TIMEOUT_MS)
    }

    // ── Reader mode ──────────────────────────────────────────────────────────────────────────

    @Test
    fun `reader mode polls every technology a badge can be`() {
        // A venue buys whatever the wholesaler had: MIFARE (NFC-A), FeliCa (NFC-F) and ISO 15693
        // fobs (NFC-V) are all sold as "RFID staff cards". Polling only NFC-A is how a card that
        // works on the USB reader is invisible on the tablet.
        for (tech in listOf(
            NfcAdapter.FLAG_READER_NFC_A,
            NfcAdapter.FLAG_READER_NFC_B,
            NfcAdapter.FLAG_READER_NFC_F,
            NfcAdapter.FLAG_READER_NFC_V,
        )) {
            assertTrue(NfcBadge.READER_FLAGS and tech != 0, "technology $tech is not polled")
        }
    }

    @Test
    fun `reader mode skips the ndef check`() {
        // A badge carries no NDEF message. Reading for one costs a round trip per tap and, on some
        // tags, wakes an app-launch intent — for data we throw away: the id is on the wire already.
        assertTrue(NfcBadge.READER_FLAGS and NfcAdapter.FLAG_READER_SKIP_NDEF_CHECK != 0)
    }

    @Test
    fun `reader mode keeps the platform sound`() {
        // The beep is the only confirmation the person gets that the tap registered — the USB
        // reader has its own, and a silent tap is indistinguishable from a card held wrong.
        assertEquals(0, NfcBadge.READER_FLAGS and NfcAdapter.FLAG_READER_NO_PLATFORM_SOUNDS)
    }

    // ── The refusals, spelled once ───────────────────────────────────────────────────────────

    @Test
    fun `each refusal has its own code`() {
        // Three different things to do about it: buy a reader, switch NFC on, use another card.
        // One shared code would collapse them into "it did not work" (the hub#338 lesson).
        assertEquals(
            3,
            setOf(NfcBadge.NFC_UNAVAILABLE, NfcBadge.NFC_DISABLED, NfcBadge.NFC_RANDOM_UID).size,
        )
    }

    private fun bytes(vararg values: Int): ByteArray =
        ByteArray(values.size) { values[it].toByte() }
}
