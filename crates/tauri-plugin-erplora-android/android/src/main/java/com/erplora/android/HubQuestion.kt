package com.erplora.android

/**
 * The question the app asks before a link links another hub (hub#2644): «Open <hub> on this
 * device?», answered in an `AlertDialog` by the person, never by the page. The app writes the words
 * in English and in Spanish ([ErploraAndroidPlugin.askToOpenHub]); this picks and checks them, pure,
 * so it is decided without a device.
 */
object HubQuestion {
    /** The words of the dialog in one language. */
    data class Words(val title: String, val message: String, val open: String, val cancel: String)

    /** Which words a device reads: Spanish unless it says otherwise (the rule of the offline page). */
    fun languageOf(language: String?): String =
        if (language.isNullOrBlank() || language.equals("es", ignoreCase = true)) "es" else "en"

    /** The words, or `null` when any is missing: a dialog with no «Cancel» could not be answered no. */
    fun wordsOf(title: String?, message: String?, open: String?, cancel: String?): Words? {
        if (title.isNullOrBlank() || message.isNullOrBlank() || open.isNullOrBlank() || cancel.isNullOrBlank()) {
            return null
        }
        return Words(title, message, open, cancel)
    }
}
