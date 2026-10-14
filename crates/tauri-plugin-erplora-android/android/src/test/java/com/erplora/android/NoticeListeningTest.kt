package com.erplora.android

import android.app.Service
import android.content.pm.ServiceInfo
import android.view.View
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

/**
 * hub#2307 — the decisions behind keeping the app listening for notices while the screen is off.
 *
 * The service itself is Android glue; what can go wrong without anyone noticing lives here, pure:
 * the service type Android 14 demands (without it `startForeground` throws and nothing listens),
 * that the system does not bring the service back on its own after killing the app (there would be
 * no page behind it to listen with), and that a notification is never shown with empty words.
 */
class NoticeListeningTest {

    @Test
    fun `from Android 14 the service declares the special use it is started for`() {
        assertEquals(ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE, NoticeListening.serviceType(34))
        assertEquals(ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE, NoticeListening.serviceType(36))
    }

    @Test
    fun `before Android 14 there is no type to declare`() {
        assertEquals(0, NoticeListening.serviceType(33))
        assertEquals(0, NoticeListening.serviceType(26))
    }

    @Test
    fun `the system does not restart the service by itself after killing the app`() {
        // A restarted service would bring back the "listening" notification with no page behind it:
        // the promise on the screen with nothing keeping it.
        assertEquals(Service.START_NOT_STICKY, NoticeListening.START_MODE)
    }

    @Test
    fun `the notification words travel from the page in the app's language`() {
        assertEquals(
            NoticeListening.Texts("ERPlora is on", "You will be warned", "Listening"),
            NoticeListening.textsOf("ERPlora is on", "You will be warned", "Listening"),
        )
    }

    @Test
    fun `a notification with missing words is refused instead of shown empty`() {
        assertNull(NoticeListening.textsOf(null, "b", "c"))
        assertNull(NoticeListening.textsOf("a", "  ", "c"))
        assertNull(NoticeListening.textsOf("a", "b", ""))
    }

    @Test
    fun `the notification does not reuse an id the notices of the shell could take`() {
        // The shell's notices use ids from the clock (hub#2305), always positive and under 1e9.
        assertEquals(true, NoticeListening.NOTIFICATION_ID < 0)
    }

    @Test
    fun `while listening the page is shown again as soon as Android hides it`() {
        // Hidden, Chromium freezes the page after a minute (measured: the `freeze` event at 60 s, with
        // the foreground service running), and the socket, the bell and the diary freeze with it.
        assertEquals(true, NoticeListening.keepsPageShown(listening = true, windowVisibility = View.GONE))
        assertEquals(true, NoticeListening.keepsPageShown(listening = true, windowVisibility = View.INVISIBLE))
    }

    @Test
    fun `the page is left alone when it is on screen or nobody asked to listen`() {
        assertEquals(false, NoticeListening.keepsPageShown(listening = true, windowVisibility = View.VISIBLE))
        assertEquals(false, NoticeListening.keepsPageShown(listening = false, windowVisibility = View.GONE))
    }
}
