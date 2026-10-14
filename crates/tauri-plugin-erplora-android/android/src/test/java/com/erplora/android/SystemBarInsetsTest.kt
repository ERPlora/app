package com.erplora.android

import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * How much room the window must reserve at the top so nothing paints over the system clock
 * (hub#1719).
 *
 * The activity opts into edge-to-edge, which on `targetSdk = 36` is not a choice: from Android 15
 * the framework lays every window out behind the system bars whether the app asks for it or not.
 * Edge-to-edge alone is fine; what was missing is the other half — reserving the inset — and
 * without it the WebView starts at the physical top of the screen. QA reproduced it twice on
 * 2026-09-09 (emulator `Pixel_10_Pro` / Android 16, APK 1.1.21): the back arrow of the sign-in
 * page the SaaS serves inside the app landed on top of the system clock.
 *
 * The amount is not simply the status bar. Two insets can occupy that strip and they do not always
 * agree:
 *
 *  - the status bar, which the user can hide (immersive mode, some launchers) — then it reports 0;
 *  - the display cutout, which is carved out of the panel and never goes away.
 *
 * Reserving the status bar alone leaves the notch painting over the content on a cutout device;
 * reserving the cutout alone leaves the clock covered on a phone without one. The window needs
 * whichever is taller.
 */
class SystemBarInsetsTest {

    @Test
    fun `reserves the status bar on a phone without a cutout`() {
        assertEquals(66, SystemBarInsets.topInset(systemBarsTop = 66, displayCutoutTop = 0))
    }

    @Test
    fun `reserves the cutout when it reaches further down than the status bar`() {
        assertEquals(
            118,
            SystemBarInsets.topInset(systemBarsTop = 66, displayCutoutTop = 118),
            "a notch taller than the status bar would paint over the content",
        )
    }

    @Test
    fun `still reserves the cutout when the status bar is hidden`() {
        assertEquals(
            118,
            SystemBarInsets.topInset(systemBarsTop = 0, displayCutoutTop = 118),
            "the cutout is carved out of the panel: hiding the status bar does not fill it back in",
        )
    }

    @Test
    fun `reserves nothing when there is nothing up there`() {
        assertEquals(
            0,
            SystemBarInsets.topInset(systemBarsTop = 0, displayCutoutTop = 0),
            "landscape with the bars hidden must not grow a blank strip",
        )
    }

    @Test
    fun `never reserves a negative amount`() {
        assertEquals(
            0,
            SystemBarInsets.topInset(systemBarsTop = -1, displayCutoutTop = -8),
            "a negative padding is a crash on setPadding, not a smaller margin",
        )
    }

    /**
     * The caller reads the view out of `findViewById`, which can hand back nothing. Painting a bit
     * high is a cosmetic problem; a till that will not start is not, so an empty view must be a
     * logged no-op rather than an exception on the way up from `onCreate`.
     */
    @Test
    fun `does not bring the app down when there is no view to pad`() {
        SystemBarInsets.applyTopSystemBarPadding(null)
    }
}
