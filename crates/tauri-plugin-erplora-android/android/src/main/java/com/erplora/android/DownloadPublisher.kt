package com.erplora.android

import android.content.ContentResolver
import android.content.ContentValues
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import androidx.annotation.RequiresApi
import java.io.File
import java.io.IOException

/**
 * Puts a file where the person holding the tablet can find it (hub#499).
 *
 * On Android, saving a file is **not** writing to a path. The folder Tauri calls Downloads is
 * `getExternalFilesDir(DIRECTORY_DOWNLOADS)` — `/storage/emulated/0/Android/data/com.erplora.app/
 * files/Download` — and Android 11 closed `Android/data` to the system Files app and to every
 * third-party file manager. A file written there exists and cannot be reached, which is why the
 * shell used to refuse instead of pretending (hub#480, ADR-0259).
 *
 * The way in is the **public Downloads collection**: an insert into
 * `MediaStore.Downloads.EXTERNAL_CONTENT_URI` gives back a `content://` URI, the bytes go out
 * through its `OutputStream`, and the file lands in the same Downloads every browser writes to. It
 * needs no permission — the app owns the row it just created — and it is the whole reason this
 * class beats `getExternalStoragePublicDirectory`, which needs `WRITE_EXTERNAL_STORAGE` and
 * therefore a dialog.
 *
 * What matters for a till: this is how a business gets its **backup** off the device it keeps its
 * data on, and its invoices out of `/files`.
 *
 * The decisions live in the companion-style functions below, pure and tested on the JVM; only
 * [publish] touches the device.
 */
object DownloadPublisher {

    /**
     * The one refusal the user can act on, spelled the same on all four sides.
     *
     * Kotlin rejects with this code, Tauri renders the rejection as `[code] - message`, the Rust
     * plugin recognises it and the page turns it into *«open your business in a browser»*. Any
     * other failure is a plain one and must NOT wear this word: sending someone to a browser
     * because the disk was full would be advice that fixes nothing.
     */
    const val DOWNLOADS_UNREACHABLE = "downloads_unreachable"

    /** Android 10. Before it there is no `MediaStore.Downloads` collection to insert into. */
    const val SDK_DOWNLOADS_COLLECTION = Build.VERSION_CODES.Q

    /**
     * The public Downloads folder as a WORD to say to the user — never as the place written to.
     *
     * The insert uses `Environment.DIRECTORY_DOWNLOADS`, so the folder the file lands in is always
     * Android's own answer and this literal cannot send it anywhere. It is only the fallback
     * wording for [locationOf] when a provider will not report the path back, and it is spelled out
     * here because `Environment` is stubbed to `null` in JVM unit tests — reading it there would
     * make the rule untestable without a device.
     */
    const val DOWNLOADS_DIRECTORY = "Download"

    /** "Some bytes" — what a file is when its extension says nothing. Never a reason to refuse. */
    const val DEFAULT_MIME_TYPE = "application/octet-stream"

    /**
     * Extension → MIME type for what a till actually saves.
     *
     * Deliberately short: this is not a mime database, it is the list of things that leave a POS —
     * the invoice PDF, the backup zip, an export, a document out of `/files`. Everything else is
     * bytes, which saves just as well.
     */
    private val MIME_TYPES = mapOf(
        "pdf" to "application/pdf",
        "zip" to "application/zip",
        "csv" to "text/csv",
        "json" to "application/json",
        "xml" to "application/xml",
        "txt" to "text/plain",
        "png" to "image/png",
        "jpg" to "image/jpeg",
        "jpeg" to "image/jpeg",
        "webp" to "image/webp",
        "svg" to "image/svg+xml",
        "xlsx" to "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    )

    /** Does this Android have a public Downloads collection to publish into? */
    @JvmStatic
    @JvmOverloads
    fun canPublish(sdkInt: Int = Build.VERSION.SDK_INT): Boolean = sdkInt >= SDK_DOWNLOADS_COLLECTION

    /**
     * The type [fileName] enters the collection under.
     *
     * It is what makes tapping the file in the Files app open a PDF viewer instead of a text
     * editor. Read from the LAST dot, so `2026.08.08-backup.zip` is a zip and not something called
     * `08.08-backup.zip`.
     */
    @JvmStatic
    fun mimeTypeOf(fileName: String): String {
        val dot = fileName.lastIndexOf('.')
        if (dot < 0 || dot == fileName.length - 1) return DEFAULT_MIME_TYPE
        return MIME_TYPES[fileName.substring(dot + 1).lowercase()] ?: DEFAULT_MIME_TYPE
    }

    /**
     * Where the file went, in words the user can act on.
     *
     * The insert answers with `content://media/external/downloads/1234`, which tells nobody
     * anything. What the user needs is the folder they will open and the name they will look for —
     * and inside the installed app it is the only sign the file exists at all: no download shelf,
     * no notification, no Downloads button.
     *
     * A provider that reports no `RELATIVE_PATH` still put the file in Downloads, so Downloads is
     * what gets said. Going quiet there would be saving without saying where.
     */
    @JvmStatic
    fun locationOf(relativePath: String?, displayName: String): String {
        val folder = relativePath?.trim()?.trim('/')?.takeIf { it.isNotEmpty() } ?: DOWNLOADS_DIRECTORY
        return "$folder/$displayName"
    }

    /**
     * Copies [source] into the public Downloads collection under [displayName] and answers with the
     * place the user will find it.
     *
     * Published as **pending** and only flipped visible once every byte is in: a backup half-copied
     * into Downloads is worse than none, because it looks like the copy the business has.
     *
     * The name is what [displayName] asks for unless it is taken — MediaStore appends its own
     * number then, and the row is read BACK so what the user is told is the file that actually
     * exists. Saving a backup twice has to leave two backups, same as on the desktop.
     *
     * The bytes arrive as a file rather than in memory on purpose: an export crosses the JNI
     * boundary as a path, not as a second copy of a multi-megabyte payload on a tablet's heap.
     */
    @RequiresApi(SDK_DOWNLOADS_COLLECTION)
    @JvmStatic
    fun publish(resolver: ContentResolver, source: File, displayName: String): String {
        val pending = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, displayName)
            put(MediaStore.MediaColumns.MIME_TYPE, mimeTypeOf(displayName))
            // Android's own name for its public Downloads folder, not our word for it: the write
            // must land where the platform says, whatever it decides to call it.
            put(MediaStore.MediaColumns.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS)
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, pending)
            ?: throw IOException("MediaStore refused a row in $DOWNLOADS_DIRECTORY")

        try {
            val out = resolver.openOutputStream(uri)
                ?: throw IOException("MediaStore gave no stream for $uri")
            out.use { sink -> source.inputStream().use { it.copyTo(sink) } }

            resolver.update(
                uri,
                ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) },
                null,
                null,
            )
        } catch (e: Exception) {
            // A pending row nobody finished is invisible to the user and never goes away on its
            // own. Failing loudly is the contract; leaving litter behind is not part of it.
            runCatching { resolver.delete(uri, null, null) }
            throw e
        }

        return locationOf(relativePathOf(resolver, uri), displayNameOf(resolver, uri) ?: displayName)
    }

    /** The name the row ended up with — MediaStore renames on a collision, and it is the truth. */
    @RequiresApi(SDK_DOWNLOADS_COLLECTION)
    private fun displayNameOf(resolver: ContentResolver, uri: Uri): String? =
        columnOf(resolver, uri, MediaStore.MediaColumns.DISPLAY_NAME)

    @RequiresApi(SDK_DOWNLOADS_COLLECTION)
    private fun relativePathOf(resolver: ContentResolver, uri: Uri): String? =
        columnOf(resolver, uri, MediaStore.MediaColumns.RELATIVE_PATH)

    /**
     * One column of the row just written, or `null` when the provider will not say.
     *
     * Swallowing the failure is right here and only here: the file is already saved and visible, so
     * a provider that will not answer must not turn a completed save into an error. The fallbacks
     * in [locationOf] and [publish] cover the wording.
     */
    private fun columnOf(resolver: ContentResolver, uri: Uri, column: String): String? =
        runCatching {
            resolver.query(uri, arrayOf(column), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) cursor.getString(0) else null
            }
        }.getOrNull()
}
