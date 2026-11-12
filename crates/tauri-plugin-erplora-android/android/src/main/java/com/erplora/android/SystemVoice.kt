package com.erplora.android

import android.content.Context
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import java.util.Locale

/**
 * What [SystemVoice] needs of Android's [TextToSpeech]; [AndroidSpeechEngine] is the real one, a
 * fake one runs the decisions on the JVM.
 */
interface SpeechEngine {
    /** [TextToSpeech.isLanguageAvailable]: one of its `LANG_*` answers. */
    fun languageStatus(locale: Locale): Int

    /** [TextToSpeech.setLanguage]: one of its `LANG_*` answers. */
    fun useLanguage(locale: Locale): Int

    /** [TextToSpeech.speak]: `true` when the engine took the piece; [flush] drops what it was reading. */
    fun speak(text: String, flush: Boolean, utteranceId: String): Boolean

    fun stop()

    fun shutdown()

    /** [TextToSpeech.getMaxSpeechInputLength]. */
    val maxInputLength: Int
}

/** What the engine tells, from any thread: whether it started, and how each piece ended. */
interface SpeechEvents {
    fun ready(ok: Boolean)

    fun done(utteranceId: String)

    fun failed(utteranceId: String)

    fun stopped(utteranceId: String)
}

/**
 * The system's own voice for the assistant's «Read aloud».
 *
 * In a browser and on Windows (WebView2) the page reads with `speechSynthesis`; the Android WebView
 * has none (MDN BCD `webview_android: false`, crbug 40417848), so without this the assistant is mute
 * in the app. This reads through Android's [TextToSpeech] instead, with the voices the person has in
 * the system settings, for the `tts_*` commands of the plugin.
 *
 * - The engine is made on the first request and kept: binding to the engine's service is not free,
 *   and an app that never reads aloud never pays it. One that cannot start is shut down and made
 *   again on the next request (an engine installed later gets its chance).
 * - One reading at a time: a new one, [stop] or [shutdown] ends the one before as read (the page
 *   asked for it), as the browser's `speechSynthesis.cancel()` does.
 * - Every request gets exactly ONE answer, whatever happens: the Rust side blocks on it.
 * - A text longer than the engine takes is read in pieces, the first flushing what was being read
 *   and the rest queued; the reading ends with the last one.
 *
 * The refusals reach the page as [TTS_UNAVAILABLE] (no engine, or no voice for the language: the
 * page reads with the browser's voice if it has one) and [TTS_FAILED] (the engine broke while
 * reading). No words for people here: the page says it, in its language.
 */
class SystemVoice(private val open: (SpeechEvents) -> SpeechEngine) {

    private class Reading(val done: () -> Unit, val failed: (String) -> Unit) {
        /** The utterances it was read in, once the engine took them. */
        var ids: List<String> = emptyList()
        var over = false
    }

    private var engine: SpeechEngine? = null
    private var ready = false

    /** Which engine the events belong to: what an engine already shut down says late is dropped. */
    private var generation = 0
    private val waiting = mutableListOf<(SpeechEngine?) -> Unit>()
    private var reading: Reading? = null
    private var readings = 0L

    /** Whether the engine has a voice for [language] (a BCP 47 tag, `es-ES`). */
    @Synchronized
    fun available(language: String, answer: (Boolean) -> Unit) {
        val locale = localeOf(language) ?: return answer(false)
        withEngine { engine -> answer(engine != null && usable(engine.languageStatus(locale))) }
    }

    /** Reads [text] in [language], ending the reading before; [done] once read or stopped. */
    @Synchronized
    fun speak(text: String, language: String, done: () -> Unit, failed: (String) -> Unit) {
        val locale = localeOf(language) ?: return failed(TTS_UNAVAILABLE)
        finishReading()
        readings += 1
        val mine = Reading(done, failed)
        reading = mine
        val prefix = "erplora-tts-$readings"
        withEngine { engine ->
            // Stopped, or replaced by a newer reading, while the engine was starting.
            if (reading !== mine) return@withEngine
            if (engine == null || !usable(engine.useLanguage(locale))) return@withEngine fail(mine, TTS_UNAVAILABLE)
            val parts = piecesOf(text, engine.maxInputLength)
            mine.ids = parts.indices.map { "$prefix-$it" }
            if (parts.isEmpty()) return@withEngine finish(mine)
            parts.forEachIndexed { index, part ->
                if (!engine.speak(part, index == 0, mine.ids[index])) {
                    engine.stop()
                    return@withEngine fail(mine, TTS_FAILED)
                }
            }
        }
    }

    /** Silences the engine and ends what was being read. */
    @Synchronized
    fun stop() {
        if (ready) engine?.stop()
        finishReading()
    }

    /** The activity is gone: everything waiting is answered and the engine released. */
    @Synchronized
    fun shutdown() {
        finishReading()
        release()
    }

    private fun withEngine(action: (SpeechEngine?) -> Unit) {
        val current = engine
        if (current != null && ready) return action(current)
        waiting += action
        if (current == null) {
            generation += 1
            val mine = generation
            val made = open(Events(mine))
            // With nothing to bind to, `TextToSpeech` answers ERROR from inside its own constructor:
            // [release] already ran, with no engine to release. Kept, this dead one would leave every
            // later request waiting for a `ready` that never comes.
            if (mine == generation) engine = made else made.shutdown()
        }
    }

    private fun release() {
        engine?.shutdown()
        engine = null
        ready = false
        generation += 1
        val asked = waiting.toList()
        waiting.clear()
        asked.forEach { it(null) }
    }

    @Synchronized
    private fun onReady(from: Int, ok: Boolean) {
        if (from != generation) return
        if (!ok) return release()
        ready = true
        val asked = waiting.toList()
        waiting.clear()
        asked.forEach { it(engine) }
    }

    @Synchronized
    private fun onEnded(from: Int, utteranceId: String, failed: Boolean) {
        if (from != generation) return
        val current = reading ?: return
        if (utteranceId !in current.ids) return
        when {
            failed -> {
                engine?.stop()
                fail(current, TTS_FAILED)
            }
            utteranceId == current.ids.last() -> finish(current)
        }
    }

    @Synchronized
    private fun onStopped(from: Int, utteranceId: String) {
        if (from != generation) return
        val current = reading ?: return
        if (utteranceId in current.ids) finish(current)
    }

    private fun finishReading() {
        reading?.let(::finish)
    }

    private fun finish(it: Reading) {
        if (it.over) return
        it.over = true
        if (reading === it) reading = null
        it.done()
    }

    private fun fail(it: Reading, code: String) {
        if (it.over) return
        it.over = true
        if (reading === it) reading = null
        it.failed(code)
    }

    private inner class Events(private val from: Int) : SpeechEvents {
        override fun ready(ok: Boolean) = onReady(from, ok)

        override fun done(utteranceId: String) = onEnded(from, utteranceId, failed = false)

        override fun failed(utteranceId: String) = onEnded(from, utteranceId, failed = true)

        override fun stopped(utteranceId: String) = onStopped(from, utteranceId)
    }

    companion object {
        /** No engine, or no voice for the language: the page reads with another voice if it has one. */
        const val TTS_UNAVAILABLE = "tts_unavailable"

        /** The engine broke while reading. */
        const val TTS_FAILED = "tts_failed"

        /** What every engine takes at once ([TextToSpeech.getMaxSpeechInputLength]), for one that says less than 1. */
        const val DEFAULT_MAX_INPUT = 4000

        /**
         * Whether the engine can read a language it answered [status] for. The language's own voice
         * counts even without the region's (Spanish of elsewhere for `es-ES`): in the app there is no
         * other voice to fall back on, and any voice beats a mute assistant.
         */
        fun usable(status: Int): Boolean = status >= TextToSpeech.LANG_AVAILABLE

        /** The locale of a BCP 47 tag (`es-ES`, also `en_US`), or `null` when it names no language. */
        fun localeOf(tag: String): Locale? {
            val clean = tag.trim().replace('_', '-')
            if (clean.isEmpty()) return null
            val locale = Locale.forLanguageTag(clean)
            return if (locale.language.isEmpty() || locale.language == "und") null else locale
        }

        /** [text] in pieces of at most [max] characters, cut at a space where it can be. */
        fun piecesOf(text: String, max: Int): List<String> {
            val limit = if (max < 1) DEFAULT_MAX_INPUT else max
            val pieces = mutableListOf<String>()
            var rest = text.replace(Regex("\\s+"), " ").trim()
            while (rest.length > limit) {
                val space = rest.lastIndexOf(' ', limit)
                val cut = if (space > 0) space else limit
                pieces += rest.substring(0, cut).trim()
                rest = rest.substring(cut).trim()
            }
            if (rest.isNotEmpty()) pieces += rest
            return pieces
        }
    }
}

/** Android's [TextToSpeech] behind [SpeechEngine]; verified on a device, as the rest of the glue. */
class AndroidSpeechEngine(context: Context, private val events: SpeechEvents) : SpeechEngine {

    private val tts = TextToSpeech(context) { status -> events.ready(status == TextToSpeech.SUCCESS) }

    init {
        tts.setOnUtteranceProgressListener(
            object : UtteranceProgressListener() {
                override fun onStart(utteranceId: String?) = Unit

                override fun onDone(utteranceId: String?) {
                    utteranceId?.let(events::done)
                }

                @Deprecated("Deprecated in Java")
                override fun onError(utteranceId: String?) {
                    utteranceId?.let(events::failed)
                }

                override fun onError(utteranceId: String?, errorCode: Int) {
                    utteranceId?.let(events::failed)
                }

                override fun onStop(utteranceId: String?, interrupted: Boolean) {
                    utteranceId?.let(events::stopped)
                }
            },
        )
    }

    override fun languageStatus(locale: Locale): Int = tts.isLanguageAvailable(locale)

    override fun useLanguage(locale: Locale): Int = tts.setLanguage(locale)

    override fun speak(text: String, flush: Boolean, utteranceId: String): Boolean {
        val mode = if (flush) TextToSpeech.QUEUE_FLUSH else TextToSpeech.QUEUE_ADD
        return tts.speak(text, mode, null, utteranceId) == TextToSpeech.SUCCESS
    }

    override fun stop() {
        tts.stop()
    }

    override fun shutdown() {
        tts.shutdown()
    }

    override val maxInputLength: Int
        get() = TextToSpeech.getMaxSpeechInputLength()
}
