package com.erplora.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * The A4 document through Android's own print service.
 *
 * `PrintManager` and the WebView cannot run without a device, so what is pinned here is what the
 * printer DECIDES: the name of the print job, which is also the file name «Save as PDF» proposes,
 * and the page margin the document is printed with. The glue — the offscreen WebView and the
 * `print` call — is verified on the emulator.
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

    @Test
    fun `a page that asks for no margin gets the one centimetre a browser leaves`() {
        // «Save as PDF» lets the WebView print right up to the edge: an invoice came out touching all
        // four edges. Chrome leaves 1 cm on each side when the page does not ask for its own margin.
        val page = "<!doctype html><html lang=\"es\"><head><meta charset=\"utf-8\"><title>F-1</title>" +
            "<style>body { margin: 0; }</style></head><body><p>x</p></body></html>"

        val printed = HtmlPrinter.withDefaultPageMargin(page)

        assertEquals(
            "<!doctype html><html lang=\"es\"><head><style>@page { margin: 10mm; }</style>" +
                "<meta charset=\"utf-8\"><title>F-1</title>" +
                "<style>body { margin: 0; }</style></head><body><p>x</p></body></html>",
            printed,
        )
    }

    @Test
    fun `the page's own margin still wins`() {
        // Two `@page` rules give the margin of the LAST one, so the default goes before the page's.
        val page = "<html><HEAD class=\"x\"><style>@page { size: A4; margin: 14mm; }</style></HEAD></html>"

        val printed = HtmlPrinter.withDefaultPageMargin(page)

        assertTrue(printed.indexOf("margin: 10mm") in 0 until printed.indexOf("margin: 14mm"), printed)
        assertTrue(printed.startsWith("<html><HEAD class=\"x\"><style>@page { margin: 10mm; }</style>"), printed)
    }

    @Test
    fun `a document without a head gets the default before anything else`() {
        // The parser puts a leading <style> in the head it opens itself; a <header> is not a head.
        val rule = "<style>@page { margin: 10mm; }</style>"
        assertEquals("<!DOCTYPE html>$rule<body><header>h</header></body>",
            HtmlPrinter.withDefaultPageMargin("<!DOCTYPE html><body><header>h</header></body>"))
        assertEquals("$rule<p>x</p>", HtmlPrinter.withDefaultPageMargin("<p>x</p>"))
    }
}
