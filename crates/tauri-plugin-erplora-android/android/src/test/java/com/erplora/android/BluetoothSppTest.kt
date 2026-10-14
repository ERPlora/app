package com.erplora.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * The decidable half of the SPP transport (ADR-0204, hub#388), pure and JVM-testable.
 *
 * The socket half can only be proven on a real Android with a real bonded thermal printer (the
 * emulator has no Bluetooth) — that end-to-end print is the issue's own DoD. What CAN be pinned
 * without hardware is every decision around the socket: which bonded devices are offered as
 * printers, and under what id.
 */
class BluetoothSppTest {

    // ── Which bonded devices are offered as printers ─────────────────────────────────────────
    //
    // The bonded list holds headsets, cars and watches. Offering all of them as print
    // destinations buries the real printer under a page of nonsense; offering none hides it.
    // Inherited from the archived ERPlora-Bridge-android, where the heuristics were validated
    // against real hardware.

    @Test
    fun `an imaging-class device is a printer whatever its name`() {
        assertTrue(BluetoothSpp.looksLikePrinter(name = "XP-58IIH", majorDeviceClass = 0x600))
        assertTrue(BluetoothSpp.looksLikePrinter(name = null, majorDeviceClass = 0x600))
    }

    @Test
    fun `a device whose name gives it away is a printer even without the class`() {
        // Cheap thermal printers routinely misreport their class; the name list is what caught
        // them on real hardware.
        assertTrue(BluetoothSpp.looksLikePrinter(name = "POS-5805", majorDeviceClass = 0))
        assertTrue(BluetoothSpp.looksLikePrinter(name = "Rongta RPP02N", majorDeviceClass = 0))
        assertTrue(BluetoothSpp.looksLikePrinter(name = "MUNBYN Printer", majorDeviceClass = 0))
    }

    @Test
    fun `a headset is not a print destination`() {
        assertFalse(BluetoothSpp.looksLikePrinter(name = "WH-1000XM5", majorDeviceClass = 0x400))
        assertFalse(BluetoothSpp.looksLikePrinter(name = null, majorDeviceClass = 0))
    }

    // ── The id a bonded printer is announced under ───────────────────────────────────────────

    @Test
    fun `the printer id is the bluetooth variant with the mac uppercased`() {
        // Must parse on the Rust side (`parse_print_target`), which normalizes MACs to uppercase:
        // two spellings of one printer would be two devices in the registry.
        assertEquals("bluetooth:AA:BB:CC:DD:EE:FF", BluetoothSpp.printerId("aa:bb:cc:dd:ee:ff"))
    }

    @Test
    fun `a nameless bonded entry still gets a name the user can recognise`() {
        assertEquals("Bluetooth (AA:BB:CC:DD:EE:FF)", BluetoothSpp.displayName(null, "AA:BB:CC:DD:EE:FF"))
        assertEquals("Bluetooth (AA:BB:CC:DD:EE:FF)", BluetoothSpp.displayName("  ", "AA:BB:CC:DD:EE:FF"))
        assertEquals("Kitchen printer", BluetoothSpp.displayName("Kitchen printer", "AA:BB:CC:DD:EE:FF"))
    }
}
