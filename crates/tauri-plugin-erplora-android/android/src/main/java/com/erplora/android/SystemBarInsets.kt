package com.erplora.android

import android.util.Log
import android.view.View
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.updatePadding

/**
 * Reserves the strip the system bars sit on — once, and only once (hub#1719, hub#1895).
 *
 * The activity lays itself out edge-to-edge, and on `targetSdk = 36` that is not a choice: since
 * Android 15 the framework puts every window behind the system bars whether the app opts in or
 * not. Edge-to-edge is only half a contract, though — the other half is reserving the strip the
 * bars occupy — and until this file there was no second half at all. The window's content started
 * at the physical top of the screen, so every page loaded inside it painted over the status bar.
 * QA hit it twice on 2026-09-09 (emulator `Pixel_10_Pro` / Android 16, APK 1.1.21): the back arrow
 * of the sign-in page sat on top of the clock, and the search layer of the till blacked the whole
 * bar out.
 *
 * It is reserved on the WINDOW, not in a page's stylesheet, and that is the point: the app hosts
 * pages served by the SaaS, by the hub and by any installed module, so a per-template fix would
 * have to be repeated in every one of them and would still miss the next.
 *
 * Reserving it on the window is not enough on its own, though, because the WebView does not stop
 * reporting the strip just because the window already moved out of its way. QA measured it with
 * CDP on 2026-09-16 (APK of `fb3b48a`): the window already started 52 CSS px down on
 * `Pixel_10_Pro` (`screen.height` 952 against `innerHeight` 900) and 24 down on `Pixel_Tablet`
 * (800 against 776) — and `env(safe-area-inset-top)` still reported those same 52 / 24, which
 * Ionic dutifully added again as `padding-top` on the first toolbar of every page. The header
 * floated half a bar below the clock everywhere (hub#1895). Note what the tablet says about the
 * WebView here: it has no cutout at all and still reported 24, so under edge-to-edge these insets
 * carry the system bars, not just the notch.
 *
 * So the strip is both reserved AND spent: whatever is padded here is subtracted from the insets
 * handed down, and what reaches the page is only what nobody has reserved yet.
 */
object SystemBarInsets {

    private const val TAG = "ErploraSystemBars"

    /**
     * How far down the top of the screen is unusable, given both insets that can occupy it.
     *
     * The status bar is the obvious one, but it can be hidden (immersive mode, some launchers) and
     * then reports zero, while a display cutout is carved out of the panel and never goes away.
     * Reserving only one of them leaves content painting under the other, so the window takes
     * whichever reaches further down.
     */
    fun topInset(systemBarsTop: Int, displayCutoutTop: Int): Int =
        maxOf(systemBarsTop, displayCutoutTop).coerceAtLeast(0)

    /**
     * Pads [view] so whatever it hosts starts below the status bar and the cutout.
     *
     * Pass the activity's content view rather than the WebView itself. The strip left above is
     * painted by whatever the padded view's background is, and the WebView's is an opaque white
     * that no theme touches — on a phone in dark mode `enableEdgeToEdge` turns the status bar
     * icons white, so padding the WebView would hide the clock on a white strip instead of under
     * an arrow. The content view is transparent, so the strip shows the window background, which
     * is `DayNight` and therefore flips with the same system setting the icons follow.
     *
     * What the listener hands back is what the WebView below sees. It is the incoming insets with
     * the reserved strip taken off the top: `WindowInsetsCompat.inset` subtracts it from the top of
     * every inset type — the status bar and the cutout — clamping at zero, and leaves left, right,
     * bottom and the IME exactly as they came. That is the difference from consuming them
     * outright, which would also flatten `env(safe-area-inset-bottom)` and drop the tabbar under
     * the gesture bar. The amount is always the one this device reports, so a phone reserving 52
     * and a 10" tablet reserving 24 both come out flush against the clock: a fixed number would fit
     * one and break the other.
     *
     * [view] is nullable because the caller gets it from `findViewById`. If it ever comes back
     * empty the app is still perfectly usable — it just paints high — so this says so in the log
     * and returns, rather than taking the till down on startup over a margin.
     */
    fun applyTopSystemBarPadding(view: View?) {
        if (view == null) {
            Log.w(TAG, "no content view to pad: the app will paint under the status bar (hub#1719)")
            return
        }

        ViewCompat.setOnApplyWindowInsetsListener(view) { target, insets ->
            val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
            val cutout = insets.getInsets(WindowInsetsCompat.Type.displayCutout())
            val reserved = topInset(bars.top, cutout.top)
            target.updatePadding(top = reserved)
            insets.inset(0, reserved, 0, 0)
        }
        // The view may already be attached with its insets dispatched, in which case the listener
        // above would not run again until the next layout change — a first frame under the clock.
        ViewCompat.requestApplyInsets(view)
    }
}
