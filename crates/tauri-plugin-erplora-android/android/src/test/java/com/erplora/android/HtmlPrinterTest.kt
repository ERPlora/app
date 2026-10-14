package com.erplora.android

import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * The A4 document through Android's own print service (hub#2008).
 *
 * `PrintManager` and the WebView cannot run without a device, so what is pinned here is what the
 * printer DECIDES: the name of the print job, which is also the file name «Save as PDF» proposes.
 * The glue — the offscreen WebView and the `print` call — is verified on the emulator.
 */
class HtmlPrinterTest {

    @Test
    fun `the job is named after the document title`() {
        // The invoice's own <title> is what the user sees in the print queue and as the PDF name.
        assertEquals("Factura F-2026-0001", HtmlPrinter.jobNameOf("Factura F-2026-0001"))
        assertEquals("Factura F-1", HtmlPrinter.jobNameOf("  Factura F-1  "))
    }

    @Test
    fun `a document without a title still gets a name`() {
        // An empty job name makes `PrintManager.print` throw; the WebView also reports the page
        // address as its title when the document has none, which is no name for a PDF.
        for (title in listOf(null, "", "   ", "about:blank", "data:text/html,<p>x</p>", "https://x.example/a")) {
            assertEquals(HtmlPrinter.DEFAULT_JOB_NAME, HtmlPrinter.jobNameOf(title), "title=$title")
        }
    }

    @Test
    fun `the name is safe as a file name`() {
        // «Save as PDF» proposes the job name as the file name: no path separators or reserved
        // characters, and nothing longer than a file system will take.
        assertEquals("Factura A-1-2026 - Cliente", HtmlPrinter.jobNameOf("Factura A/1\\2026 : Cliente"))
        assertEquals("a-b-c-d-e-f", HtmlPrinter.jobNameOf("a*b?c\"d<e>f"))
        assertEquals(HtmlPrinter.MAX_JOB_NAME, HtmlPrinter.jobNameOf("x".repeat(500)).length)
    }
}
