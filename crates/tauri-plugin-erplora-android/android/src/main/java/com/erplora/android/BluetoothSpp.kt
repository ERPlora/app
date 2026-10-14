package com.erplora.android

import android.bluetooth.BluetoothAdapter
import java.util.UUID

/**
 * Bluetooth Classic SPP transport — Android only (ADR-0204, hub#388).
 *
 * Ported from the archived `ERPlora-Bridge-android` app (`PrinterConnection.kt` /
 * `PrinterDiscovery.kt`), whose SPP path was the one real reason that app still existed. This is
 * TRANSPORT only, on purpose: the ESC/POS rendering stays in Rust
 * (`erplora-peripherals::escpos`), and what crosses into Kotlin is finished bytes plus the MAC of
 * a bonded printer. Keeping the boundary there is what makes a bluetooth ticket identical to a
 * network one everywhere above the socket.
 *
 * Discovery here means the BONDED list — the printers the user already paired in Android's own
 * settings — never an over-the-air scan: pairing is the OS's job (with its own UI and PIN
 * handling), and skipping the scan is also what keeps `BLUETOOTH_SCAN` out of the permission
 * batch — one dialog fewer, in the spirit of hub#758.
 *
 * The connection is opened per job and closed after it. A held-open socket saves ~1s per ticket
 * but turns "printer switched off between tickets" into a stale handle that fails the NEXT job;
 * phase 1 of ADR-0204 buys the simple failure mode. Everything decidable is a pure function of
 * plain values, testable on the JVM without an emulator.
 */
object BluetoothSpp {

    /** The well-known Serial Port Profile UUID — the one RFCOMM service ESC/POS printers expose. */
    val SPP_UUID: UUID = UUID.fromString("00001101-0000-1000-8000-00805f9b34fb")

    /** Bluetooth major device class IMAGING (0x600): printers, scanners. */
    const val IMAGING_MAJOR_CLASS = 0x600

    /**
     * Name fragments that give away a thermal printer when the device class does not.
     * Inherited from the archived bridge, where the list was validated against real hardware.
     */
    val PRINTER_NAME_HINTS = listOf(
        "print", "pos", "thermal", "escpos", "star", "epson",
        "bixolon", "citizen", "sewoo", "rongta", "munbyn",
    )

    /**
     * Is this bonded device worth offering as a printer? Pure: the bonded list holds headsets,
     * cars and watches; offering those as print destinations would bury the real printer.
     */
    @JvmStatic
    fun looksLikePrinter(name: String?, majorDeviceClass: Int): Boolean =
        majorDeviceClass == IMAGING_MAJOR_CLASS ||
            PRINTER_NAME_HINTS.any { it in (name ?: "").lowercase() }

    /** The `printer_id` a bonded device is announced under: `bluetooth:{MAC}` (ADR-0204). */
    @JvmStatic
    fun printerId(mac: String): String = "bluetooth:${mac.uppercase()}"

    /** What to call a printer whose bonded entry has no name. */
    @JvmStatic
    fun displayName(name: String?, mac: String): String =
        if (name.isNullOrBlank()) "Bluetooth ($mac)" else name

    /**
     * Sends already-rendered ESC/POS bytes to the bonded device at [mac] over RFCOMM.
     *
     * Blocking — the caller runs it off the main thread. Errors propagate: a ticket that did not
     * come out must be reported as failed, never resolved (the hub#475 lesson).
     */
    @JvmStatic
    fun send(adapter: BluetoothAdapter, mac: String, payload: ByteArray) {
        @Suppress("MissingPermission")
        val device = adapter.getRemoteDevice(mac)
        @Suppress("MissingPermission")
        val socket = device.createRfcommSocketToServiceRecord(SPP_UUID)
        try {
            @Suppress("MissingPermission")
            socket.connect()
            socket.outputStream.use { out ->
                out.write(payload)
                out.flush()
            }
        } finally {
            try {
                socket.close()
            } catch (_: Exception) {
            }
        }
    }
}
