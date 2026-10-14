package com.erplora.android

import android.app.Activity
import android.content.Context
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.print.PageRange
import android.print.PrintAttributes
import android.print.PrintDocumentAdapter
import android.print.PrintManager
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import java.util.concurrent.atomic.AtomicBoolean

/**
 * An A4 document through Android's own print service (hub#2008).
 *
 * Inside the app's WebView `window.print()` prints nothing and wry has no `print` on Android, so an
 * invoice could only leave the tablet on the till roll. Android prints a WebView natively:
 * [WebView.createPrintDocumentAdapter] handed to [PrintManager.print] opens the system print
 * screen, with every printer the device knows and «Save as PDF». That is what this does, with a
 * WebView of its own whose ONLY content is the document — the desktop print window of hub#2006,
 * Android's way.
 *
 * The html comes from a remote page, so the WebView that renders it runs no script, exposes no
 * bridge, reads no files and never navigates: a link inside the invoice must not turn it into a
 * browser.
 *
 * The decisions live in the pure functions, tested on the JVM; [print] is the glue, verified on
 * the emulator.
 */
object HtmlPrinter {

    /** The job name when the document has no title of its own. */
    const val DEFAULT_JOB_NAME = "ERPlora"

    /** Longest job name kept: «Save as PDF» proposes it as a file name. */
    const val MAX_JOB_NAME = 80

    /**
     * How long the document may take to load before the call gives up. A document with remote
     * images waits for them; one that never finishes must not leave the shell waiting forever.
     */
    const val LOAD_TIMEOUT_MS = 30_000L

    /** Characters no file system takes in a name, plus the path separators. */
    private val RESERVED = Regex("[\\\\/:*?\"<>|\\p{Cntrl}]")

    /**
     * The name of the print job for a document titled [title].
     *
     * It is what the user sees in the print queue and the file name «Save as PDF» proposes, so it
     * is the invoice's own title, made safe as a file name. The WebView reports the page ADDRESS
     * as the title of a document without one, which is no name for a PDF: that, like no title at
     * all, gets [DEFAULT_JOB_NAME] — `PrintManager` refuses an empty name.
     */
    @JvmStatic
    fun jobNameOf(title: String?): String {
        val trimmed = title?.trim().orEmpty()
        if (trimmed.isEmpty() || looksLikeAnAddress(trimmed)) return DEFAULT_JOB_NAME
        val safe = trimmed.replace(RESERVED, "-").trim().take(MAX_JOB_NAME).trim()
        return safe.ifEmpty { DEFAULT_JOB_NAME }
    }

    private fun looksLikeAnAddress(title: String): Boolean =
        title.startsWith("about:") || title.startsWith("data:") || title.contains("://")

    /**
     * Renders [html] in an offscreen WebView and opens the system print screen with it, preset to
     * A4. [onOpened] runs once the print screen has been asked for; [onFailed] when it could not
     * be — exactly one of the two, once.
     *
     * The WebView is held until the print job is finished: Android's own guidance, because the
     * adapter keeps drawing from it while the user changes printer or pages.
     */
    fun print(activity: Activity, html: String, onOpened: () -> Unit, onFailed: (Exception) -> Unit) {
        val answered = AtomicBoolean(false)
        val main = Handler(Looper.getMainLooper())
        main.post {
            val webView: WebView
            try {
                webView = WebView(activity)
                webView.settings.javaScriptEnabled = false
                webView.settings.allowFileAccess = false
                webView.settings.allowContentAccess = false
            } catch (e: Exception) {
                if (answered.compareAndSet(false, true)) onFailed(e)
                return@post
            }
            held.add(webView)

            webView.webViewClient = object : WebViewClient() {
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest) = true

                override fun onPageFinished(view: WebView, url: String?) {
                    if (!answered.compareAndSet(false, true)) return
                    try {
                        val name = jobNameOf(view.title)
                        val manager = activity.getSystemService(Context.PRINT_SERVICE) as PrintManager
                        val adapter = Releasing(view.createPrintDocumentAdapter(name)) {
                            release(view)
                        }
                        manager.print(name, adapter, a4())
                        onOpened()
                    } catch (e: Exception) {
                        release(view)
                        onFailed(e)
                    }
                }
            }
            webView.loadDataWithBaseURL(null, html, "text/html", "UTF-8", null)

            main.postDelayed({
                if (answered.compareAndSet(false, true)) {
                    release(webView)
                    onFailed(IllegalStateException("the document did not load in ${LOAD_TIMEOUT_MS} ms"))
                }
            }, LOAD_TIMEOUT_MS)
        }
    }

    /** WebViews whose print job is still running. Main thread only. */
    private val held = mutableSetOf<WebView>()

    private fun release(view: WebView) {
        if (held.remove(view)) view.destroy()
    }

    private fun a4(): PrintAttributes =
        PrintAttributes.Builder().setMediaSize(PrintAttributes.MediaSize.ISO_A4).build()

    /** The WebView's own adapter, telling us when the job is over so its WebView can go. */
    private class Releasing(
        private val inner: PrintDocumentAdapter,
        private val onDone: () -> Unit,
    ) : PrintDocumentAdapter() {
        override fun onStart() = inner.onStart()

        override fun onLayout(
            oldAttributes: PrintAttributes?,
            newAttributes: PrintAttributes,
            cancellationSignal: CancellationSignal?,
            callback: LayoutResultCallback,
            extras: Bundle?,
        ) = inner.onLayout(oldAttributes, newAttributes, cancellationSignal, callback, extras)

        override fun onWrite(
            pages: Array<out PageRange>,
            destination: ParcelFileDescriptor,
            cancellationSignal: CancellationSignal?,
            callback: WriteResultCallback,
        ) = inner.onWrite(pages, destination, cancellationSignal, callback)

        override fun onFinish() {
            try {
                inner.onFinish()
            } finally {
                onDone()
            }
        }
    }
}
