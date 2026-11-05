package com.erplora.android

import android.speech.tts.TextToSpeech
import java.util.Locale
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * The system's own voice for the assistant's «Read aloud» (round 2026-11-04a).
 *
 * The Android WebView has no `speechSynthesis`, so without this the assistant is mute in the app.
 * `TextToSpeech` cannot run on the JVM, so what is pinned here is what [SystemVoice] DECIDES over a
 * fake engine: when the engine is made, which language it reads in, how a reading ends (read,
 * stopped, failed) and that every request gets exactly one answer — the Rust side waits on it. The
 * glue ([AndroidSpeechEngine]) is verified on a device.
 */
class SystemVoiceTest {

    private class FakeEngine(
        val events: SpeechEvents,
        var status: Int = TextToSpeech.LANG_COUNTRY_AVAILABLE,
        var takes: Boolean = true,
        override val maxInputLength: Int = 4000,
    ) : SpeechEngine {
        val asked = mutableListOf<Locale>()
        val used = mutableListOf<Locale>()
        val spoken = mutableListOf<Triple<String, Boolean, String>>()
        var stops = 0
        var shut = false

        override fun languageStatus(locale: Locale): Int = status.also { asked += locale }
        override fun useLanguage(locale: Locale): Int = status.also { used += locale }
        override fun speak(text: String, flush: Boolean, utteranceId: String): Boolean {
            spoken += Triple(text, flush, utteranceId)
            return takes
        }
        override fun stop() { stops += 1 }
        override fun shutdown() { shut = true }
    }

    /** What one request was answered with: each list must end up with exactly one entry. */
    private class Answers {
        val done = mutableListOf<Unit>()
        val failed = mutableListOf<String>()
        val total get() = done.size + failed.size
    }

    private val engines = mutableListOf<FakeEngine>()
    private var maxInput = 4000

    /**
     * With no engine it can bind to, `TextToSpeech` answers ERROR from INSIDE its own constructor
     * (`initTts` → `dispatchOnInit`, with no executor in the public constructors), so `ready(false)`
     * reaches the voice before the engine it belongs to has been handed over.
     */
    private var failsWhileBeingMade = false
    private val voice = SystemVoice { events ->
        FakeEngine(events, maxInputLength = maxInput).also {
            engines += it
            if (failsWhileBeingMade) events.ready(false)
        }
    }
    private val engine get() = engines.last()

    private fun speak(text: String, language: String = "es-ES"): Answers {
        val answers = Answers()
        voice.speak(text, language, done = { answers.done += Unit }, failed = { answers.failed += it })
        return answers
    }

    private fun ready(ok: Boolean = true) = engine.events.ready(ok)

    // ── The engine: made on the first use, once ─────────────────────────────────────────────

    @Test
    fun `no engine is made until somebody asks for the voice`() {
        // Binding to the engine's service is not free: an app that never reads aloud never pays it.
        assertTrue(engines.isEmpty())
        speak("Hola")
        assertEquals(1, engines.size)
    }

    @Test
    fun `the engine is made once and kept for every reading after`() {
        speak("Hola")
        ready()
        engine.events.done(engine.spoken.last().third)
        speak("Otra")
        val third = mutableListOf<Boolean>()
        voice.available("en-US") { third += it }
        assertEquals(1, engines.size)
        assertEquals(listOf(true), third)
    }

    @Test
    fun `a request made before the engine is ready waits for it`() {
        val answers = speak("Hola")
        assertTrue(engine.spoken.isEmpty(), "nothing can be read before the engine says it is ready")
        ready()
        assertEquals(listOf(Triple("Hola", true, engine.spoken.single().third)), engine.spoken)
        assertEquals(0, answers.total, "the reading ends when the engine says so, not when it starts")
    }

    @Test
    fun `an engine that cannot start refuses as unavailable and is tried again next time`() {
        val answers = speak("Hola")
        val available = mutableListOf<Boolean>()
        voice.available("es-ES") { available += it }
        ready(ok = false)
        assertEquals(listOf(SystemVoice.TTS_UNAVAILABLE), answers.failed)
        assertEquals(listOf(false), available)
        assertTrue(engines.single().shut, "a dead engine is shut down, not kept")
        // An engine installed later (or a service that came back) gets its chance.
        speak("Hola")
        assertEquals(2, engines.size)
    }

    @Test
    fun `an engine that fails while being made refuses at once and shuts down`() {
        failsWhileBeingMade = true
        val answers = speak("Hola")
        assertEquals(listOf(SystemVoice.TTS_UNAVAILABLE), answers.failed)
        assertEquals(0, answers.done.size)
        assertTrue(engines.single().shut, "the engine that never started is released, not forgotten")
    }

    @Test
    fun `every request after an engine failed while being made is answered too`() {
        // The Rust side blocks on the answer: a request kept waiting for a dead engine never returns.
        failsWhileBeingMade = true
        val first = mutableListOf<Boolean>()
        voice.available("es-ES") { first += it }
        val second = mutableListOf<Boolean>()
        voice.available("en-US") { second += it }
        assertEquals(listOf(false), first)
        assertEquals(listOf(false), second)
        assertEquals(2, engines.size, "each request gets a new try: an engine may have been installed since")
    }

    @Test
    fun `an engine installed after one failed while being made reads`() {
        failsWhileBeingMade = true
        speak("Hola")
        failsWhileBeingMade = false
        val answers = speak("Otra")
        ready()
        assertEquals(listOf("Otra"), engine.spoken.map { it.first })
        engine.events.done(engine.spoken.single().third)
        assertEquals(1, answers.done.size)
    }

    // ── The language ────────────────────────────────────────────────────────────────────────

    @Test
    fun `the text is read in the language asked for, with its region`() {
        speak("Hola")
        ready()
        assertEquals(listOf(Locale.forLanguageTag("es-ES")), engine.used)
        assertEquals("ES", engine.used.single().country)
    }

    @Test
    fun `a language the engine has is available`() {
        val answers = mutableListOf<Boolean>()
        voice.available("en-US") { answers += it }
        ready()
        assertEquals(listOf(true), answers)
        assertEquals(listOf(Locale.forLanguageTag("en-US")), engine.asked)
    }

    @Test
    fun `a language without its voice data or not supported is not available`() {
        for (status in listOf(TextToSpeech.LANG_MISSING_DATA, TextToSpeech.LANG_NOT_SUPPORTED)) {
            val voice = SystemVoice { events -> FakeEngine(events, status = status).also { engines += it } }
            val answers = mutableListOf<Boolean>()
            voice.available("es-ES") { answers += it }
            engines.last().events.ready(true)
            assertEquals(listOf(false), answers, "status=$status")
        }
    }

    @Test
    fun `reading in a language the engine lacks refuses as unavailable and reads nothing`() {
        val voice = SystemVoice { events -> FakeEngine(events, status = TextToSpeech.LANG_MISSING_DATA).also { engines += it } }
        val failed = mutableListOf<String>()
        voice.speak("Hola", "es-ES", done = { error("not read") }, failed = { failed += it })
        engine.events.ready(true)
        assertEquals(listOf(SystemVoice.TTS_UNAVAILABLE), failed)
        assertTrue(engine.spoken.isEmpty())
    }

    @Test
    fun `a language with only its own voice and not the region's still reads`() {
        // A tablet with Spanish of elsewhere reads in it: the Android webview has no other voice to
        // fall back on, and a voice beats a mute assistant.
        assertTrue(SystemVoice.usable(TextToSpeech.LANG_AVAILABLE))
        assertTrue(SystemVoice.usable(TextToSpeech.LANG_COUNTRY_AVAILABLE))
        assertTrue(SystemVoice.usable(TextToSpeech.LANG_COUNTRY_VAR_AVAILABLE))
        assertFalse(SystemVoice.usable(TextToSpeech.LANG_MISSING_DATA))
        assertFalse(SystemVoice.usable(TextToSpeech.LANG_NOT_SUPPORTED))
    }

    @Test
    fun `the language is read from its tag and nothing else is a language`() {
        assertEquals(Locale.forLanguageTag("es-ES"), SystemVoice.localeOf("es-ES"))
        assertEquals(Locale.forLanguageTag("en-US"), SystemVoice.localeOf(" en_US "))
        for (tag in listOf("", "   ", "und", "!!")) assertNull(SystemVoice.localeOf(tag), "tag=$tag")
    }

    @Test
    fun `no language refuses without making an engine`() {
        val answers = speak("Hola", language = "")
        val available = mutableListOf<Boolean>()
        voice.available("") { available += it }
        assertEquals(listOf(SystemVoice.TTS_UNAVAILABLE), answers.failed)
        assertEquals(listOf(false), available)
        assertTrue(engines.isEmpty())
    }

    // ── How a reading ends ──────────────────────────────────────────────────────────────────

    @Test
    fun `a reading ends when the engine has read it`() {
        val answers = speak("Hola")
        ready()
        engine.events.done(engine.spoken.single().third)
        assertEquals(1, answers.done.size)
        assertEquals(0, answers.failed.size)
    }

    @Test
    fun `an engine error fails the reading as failed`() {
        val answers = speak("Hola")
        ready()
        engine.events.failed(engine.spoken.single().third)
        assertEquals(listOf(SystemVoice.TTS_FAILED), answers.failed)
        assertEquals(0, answers.done.size)
    }

    @Test
    fun `an engine that refuses the text fails the reading as failed`() {
        val voice = SystemVoice { events -> FakeEngine(events, takes = false).also { engines += it } }
        val failed = mutableListOf<String>()
        voice.speak("Hola", "es-ES", done = { error("not read") }, failed = { failed += it })
        engine.events.ready(true)
        assertEquals(listOf(SystemVoice.TTS_FAILED), failed)
    }

    @Test
    fun `stopping ends the reading and silences the engine`() {
        val answers = speak("Hola")
        ready()
        voice.stop()
        assertEquals(1, answers.done.size, "a stopped reading ends, as the browser's voice does")
        assertEquals(1, engine.stops)
        // The engine still says how the piece ended: it is not answered twice.
        engine.events.stopped(engine.spoken.single().third)
        engine.events.done(engine.spoken.single().third)
        assertEquals(1, answers.total)
    }

    @Test
    fun `a new reading ends the one before and takes its place`() {
        val first = speak("Primera")
        ready()
        val firstId = engine.spoken.single().third
        val second = speak("Segunda")
        assertEquals(1, first.done.size, "the reading before ends when the new one flushes it")
        assertEquals(Triple("Segunda", true, engine.spoken.last().third), engine.spoken.last())
        // What the engine says late about the first reading is not the second's.
        engine.events.done(firstId)
        engine.events.failed(firstId)
        assertEquals(0, second.total)
        engine.events.done(engine.spoken.last().third)
        assertEquals(1, second.done.size)
        assertEquals(1, first.total)
    }

    @Test
    fun `a reading stopped before the engine is ready is never read`() {
        val answers = speak("Hola")
        voice.stop()
        ready()
        assertTrue(engine.spoken.isEmpty())
        assertEquals(1, answers.done.size)
    }

    @Test
    fun `each reading has its own utterance ids`() {
        speak("Uno")
        ready()
        speak("Dos")
        val ids = engine.spoken.map { it.third }
        assertEquals(ids.distinct(), ids)
    }

    @Test
    fun `shutting down answers what was waiting and releases the engine`() {
        val reading = speak("Hola")
        ready()
        val waiting = speak("Otra")
        voice.shutdown()
        assertEquals(1, reading.total)
        assertEquals(1, waiting.done.size, "the Rust side waits on every request: none is left hanging")
        assertTrue(engine.shut)
        // Asked again after the activity came back: a new engine.
        speak("Hola")
        assertEquals(2, engines.size)
    }

    @Test
    fun `shutting down before the engine is ready answers the requests that waited`() {
        val answers = speak("Hola")
        val available = mutableListOf<Boolean>()
        voice.available("es-ES") { available += it }
        voice.shutdown()
        assertEquals(1, answers.total)
        assertEquals(listOf(false), available)
        // The late «ready» of an engine already shut down reads nothing.
        engines.single().events.ready(true)
        assertTrue(engines.single().spoken.isEmpty())
    }

    // ── Long texts ──────────────────────────────────────────────────────────────────────────

    @Test
    fun `a text longer than the engine takes is read in pieces, one after the other`() {
        maxInput = 20
        val answers = speak("Una frase corta. Y otra frase algo más larga que sigue.")
        ready()
        val pieces = engine.spoken
        assertTrue(pieces.size > 1)
        assertTrue(pieces.all { it.first.length <= 20 }, "every piece fits: ${pieces.map { it.first }}")
        assertEquals("Una frase corta. Y otra frase algo más larga que sigue.", pieces.joinToString(" ") { it.first })
        assertEquals(listOf(true) + List(pieces.size - 1) { false }, pieces.map { it.second }, "the first flushes, the rest queue")
        // Only the last piece ends the reading.
        engine.events.done(pieces.first().third)
        assertEquals(0, answers.total)
        engine.events.done(pieces.last().third)
        assertEquals(1, answers.done.size)
    }

    @Test
    fun `the pieces cut at a space, and a word longer than the limit is cut where it reaches it`() {
        assertEquals(listOf("Hola"), SystemVoice.piecesOf("  Hola  ", 10))
        assertEquals(listOf("uno dos", "tres"), SystemVoice.piecesOf("uno dos tres", 8))
        assertEquals(listOf("abcde", "fghij", "k"), SystemVoice.piecesOf("abcdefghijk", 5))
        assertEquals(emptyList(), SystemVoice.piecesOf("   ", 5))
    }

    @Test
    fun `an engine that reports no limit reads with the platform one`() {
        // `getMaxSpeechInputLength` is 4000 on every engine; a broken one answering 0 must not loop.
        assertEquals(listOf("abc"), SystemVoice.piecesOf("abc", 0))
    }
}
