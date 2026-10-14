package com.erplora.android

import android.nfc.NfcAdapter

/**
 * Reading a staff badge off the tablet's OWN NFC reader — Android only (hub#988, follow-up of the
 * employee badge, hub#658).
 *
 * At a counter the badge arrives through a €15 USB reader that behaves as a keyboard: it types the
 * number and presses Enter, and the shell catches the burst by its speed
 * (`apps/web/src/lib/badge-scanner.ts`). On a tablet there is no USB reader and no keyboard — but
 * the reader is already inside the device and, until this existed, unused. A salon on a tablet
 * simply could not enrol or read a card.
 *
 * **One badge path, two origins** is the design condition of the issue, and it is what this file
 * exists to respect: the tap produces the same kind of string the wedge produces, and it is handed
 * to the same subscribers. Nothing above — the login screen, the approval dialog, the staff form —
 * learns where a card came from.
 *
 * Two things are decided here rather than at the call site, and both are the difference between a
 * badge that works next week and one that does not:
 *
 *  - **the spelling of the UID** — uppercase hex, no separators. It has to satisfy the shell's
 *    `BADGE_SHAPE` and be stable forever, because the runtime stores a HMAC INDEX of it
 *    (`hub_user.badge_index`): a second rendering of the same card is a different badge;
 *  - **which UIDs are refused** — a card that answers with a fresh id on every tap can be enrolled
 *    once and never matched again, which locks the employee out with a card that demonstrably
 *    "worked when we set it up".
 *
 * The radio half can only be proven with a card in a hand, and that validation is booked in pm#145.
 * Everything decidable is a pure function of plain bytes, tested on the JVM without a device —
 * the same split as [BluetoothSpp].
 */
object NfcBadge {

    /** This device has no NFC reader at all. Nothing to switch on; the venue needs a USB reader. */
    const val NFC_UNAVAILABLE = "nfc_unavailable"

    /** There is a reader and it is switched OFF. The one refusal the user can act on. */
    const val NFC_DISABLED = "nfc_disabled"

    /** The card answers with a new id on every tap, so it cannot be anybody's badge. */
    const val NFC_RANDOM_UID = "nfc_random_uid"

    /**
     * How long reader mode stays open on one call, when the caller names no timeout.
     *
     * 15 s is the walk from pressing «enrol» to finding the card in a pocket. Longer holds the
     * radio open under a screen the user has already left; much shorter turns enrolment into a
     * race the user loses and repeats.
     */
    const val DEFAULT_TIMEOUT_MS = 15_000L

    /** Below this the radio would spend its time starting and stopping instead of polling. */
    const val MIN_TIMEOUT_MS = 1_000L

    /** Above this reader mode outlives the screen that asked for it, and drains the battery. */
    const val MAX_TIMEOUT_MS = 60_000L

    /** The shortest real UID: ISO 14443-3 single size. */
    private const val MIN_UID_BYTES = 4

    /**
     * ISO 14443-3 marks a single-size UID starting with `0x08` as a **Random ID**: the card mints
     * a new one on every tap. DESFire configured for privacy, most Ultralight and every phone in
     * card-emulation mode do this.
     */
    private const val RANDOM_UID_PREFIX = 0x08.toByte()

    /**
     * How reader mode is opened.
     *
     * Every technology sold as a "staff card" is polled — a venue buys whatever the wholesaler had,
     * and NFC-A alone would leave a FeliCa or ISO 15693 fob invisible on the tablet while it works
     * on the USB reader. NDEF is skipped: a badge carries no message, and looking for one costs a
     * round trip per tap and can wake a tag's app-launch intent. The platform sound is KEPT — the
     * beep is the only confirmation the person gets that the tap registered.
     */
    val READER_FLAGS: Int =
        NfcAdapter.FLAG_READER_NFC_A or
            NfcAdapter.FLAG_READER_NFC_B or
            NfcAdapter.FLAG_READER_NFC_F or
            NfcAdapter.FLAG_READER_NFC_V or
            NfcAdapter.FLAG_READER_SKIP_NDEF_CHECK

    /**
     * The badge string of a UID: uppercase hex, no separators, one byte per two characters.
     *
     * Pure rendering — [badgeOf] is what decides whether the card may be used at all.
     */
    @JvmStatic
    fun toBadge(uid: ByteArray): String {
        val out = StringBuilder(uid.size * 2)
        for (byte in uid) {
            val value = byte.toInt() and 0xFF
            out.append(HEX[value ushr 4]).append(HEX[value and 0x0F])
        }
        return out.toString()
    }

    /**
     * Can this UID be somebody's badge next week as well as today?
     *
     * The refusals are not tidiness. An all-zero id is what a half-powered field reads, and it is
     * the SAME id for every misread — one collision straight into the HMAC index. A random id
     * enrols fine and never matches again.
     */
    @JvmStatic
    fun isStableUid(uid: ByteArray?): Boolean {
        if (uid == null || uid.size < MIN_UID_BYTES) return false
        if (uid.all { it == 0.toByte() }) return false
        // Only a SINGLE-size UID carries the random marker: on a 7- or 10-byte id the same byte is
        // a real manufacturer code, and refusing it would reject good stock.
        return !(uid.size == MIN_UID_BYTES && uid[0] == RANDOM_UID_PREFIX)
    }

    /** The badge a tapped card enrols under, or `null` when the card cannot be one. */
    @JvmStatic
    fun badgeOf(uid: ByteArray?): String? =
        if (isStableUid(uid)) toBadge(uid!!) else null

    /**
     * The timeout to actually use. Clamped, never refused: an argument nobody typed by hand must
     * not be the reason a till has no reader.
     */
    @JvmStatic
    fun clampTimeout(requested: Long?): Long =
        (requested ?: DEFAULT_TIMEOUT_MS).coerceIn(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS)

    private const val HEX = "0123456789ABCDEF"
}
