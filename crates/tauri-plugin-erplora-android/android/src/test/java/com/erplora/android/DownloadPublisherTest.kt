package com.erplora.android


import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * Saving a file where the user will find it, on Android (hub#499).
 *
 * `MediaStore` cannot be touched without a device, so what is pinned here is what the publisher
 * DECIDES: which Android versions have somewhere public to save at all, what type the file enters
 * the collection under, and what place the user is told. The impure half — the insert and the byte
 * copy — is half a dozen lines of glue, and it is verified on the emulator.
 */
class DownloadPublisherTest {

    // ── Which Androids have a public Downloads collection ────────────────────────────────────────

    @Test
    fun `Android 10 and up can publish into the Downloads collection`() {
        // `MediaStore.Downloads` arrived with scoped storage (API 29). It is the PUBLIC collection:
        // it shows up in the Files app and in every file manager, and needs no permission at all.
        for (sdkInt in listOf(29, 33, 36, 37)) {
            assertTrue(DownloadPublisher.canPublish(sdkInt), "API $sdkInt has MediaStore.Downloads")
        }
    }

    @Test
    fun `below Android 10 there is no collection to publish into`() {
        // The project's minSdk is 24, and below 29 the only public place is
        // `getExternalStoragePublicDirectory` + `WRITE_EXTERNAL_STORAGE` — a RUNTIME permission,
        // which makes it a different design (a dialog the save has to wait for). Until that exists
        // those versions are REFUSED: the user hears "open your business in a browser", which is
        // true and actionable, instead of getting a file hidden where they cannot reach it.
        for (sdkInt in listOf(24, 26, 28)) {
            assertFalse(DownloadPublisher.canPublish(sdkInt), "API $sdkInt does not have it")
        }
    }

    // ── The MIME type the file enters the collection under ───────────────────────────────────────

    @Test
    fun `the three files a till actually saves get their real type`() {
        // Not cosmetic: the type is what makes a tap in the Files app open the PDF viewer instead
        // of a text editor. And these are exactly the three of hub#480 — the invoice, the backup
        // and a document out of `/files`.
        assertEquals("application/pdf", DownloadPublisher.mimeTypeOf("factura-2026-0042.pdf"))
        assertEquals("application/zip", DownloadPublisher.mimeTypeOf("hub.blueprint.zip"))
        assertEquals("text/csv", DownloadPublisher.mimeTypeOf("ventas.csv"))
    }

    @Test
    fun `the extension is read case-insensitively`() {
        assertEquals("application/pdf", DownloadPublisher.mimeTypeOf("FACTURA.PDF"))
    }

    @Test
    fun `a name with no extension is still saveable`() {
        // `application/octet-stream` means "some bytes": the file is saved all the same. Refusing
        // would be losing the backup over not recognising an extension.
        for (name in listOf("LICENSE", "backup", "informe.")) {
            assertEquals("application/octet-stream", DownloadPublisher.mimeTypeOf(name))
        }
    }

    @Test
    fun `an extension nobody listed is bytes, not a refusal`() {
        assertEquals("application/octet-stream", DownloadPublisher.mimeTypeOf("cierre.qqq"))
    }

    @Test
    fun `only the LAST dot names the type`() {
        // `2026.08.08-backup.zip` split at the first dot would give `08.08-backup.zip` as the
        // extension and the file would enter the collection as loose bytes.
        assertEquals("application/zip", DownloadPublisher.mimeTypeOf("2026.08.08-backup.zip"))
    }

    // ── What the user is told ────────────────────────────────────────────────────────────────────

    @Test
    fun `the user is told a folder and a name, not a content URI`() {
        // What the insert returns is `content://media/external/downloads/1234`, which says nothing
        // to anybody. Inside the installed app there is no download shelf and no notification, so
        // this sentence is the only sign the file exists: it has to be the place the user will go
        // looking for it.
        assertEquals(
            "Download/factura-2026-0042.pdf",
            DownloadPublisher.locationOf("Download/", "factura-2026-0042.pdf"),
        )
    }

    @Test
    fun `a relative path without its trailing slash does not lose the separator`() {
        assertEquals("Download/hub.zip", DownloadPublisher.locationOf("Download", "hub.zip"))
    }

    @Test
    fun `an OEM that reports no relative path still names the Downloads folder`() {
        // `RELATIVE_PATH` is the column just written, but reading it back goes through the
        // manufacturer's own provider. Without it the file is still in Downloads, so Downloads is
        // what gets said — going quiet would be not saying where it landed.
        //
        // This case was first written as `assertEquals(Environment.DIRECTORY_DOWNLOADS,
        // DOWNLOADS_DIRECTORY)` and failed with `expected:<null>`: the mockable `android.jar` these
        // JVM tests run against stubs static fields away, so `Environment` cannot be read here at
        // all. Rather than pin a rule this bench cannot see, the publisher stopped needing it —
        // `RELATIVE_PATH` is WRITTEN from `Environment.DIRECTORY_DOWNLOADS` itself, so the file
        // always lands where Android says, and the constant below is only the fallback WORDING. A
        // wrong word there misnames a folder in a message; it cannot misplace a file.
        for (relativePath in listOf(null, "", "   ")) {
            assertEquals(
                "Download/hub.zip",
                DownloadPublisher.locationOf(relativePath, "hub.zip"),
            )
        }
    }

    // ── The refusal code that travels all the way to the page ────────────────────────────────────

    @Test
    fun `the refusal travels under the word the whole chain reads`() {
        // Kotlin rejects with this code, Tauri renders it as `[code] - message`, Rust recognises it
        // and the page turns it into "open your business in a browser". One word end to end; there
        // is a test in `src/lib.rs` holding the other end.
        assertEquals("downloads_unreachable", DownloadPublisher.DOWNLOADS_UNREACHABLE)
    }
}
