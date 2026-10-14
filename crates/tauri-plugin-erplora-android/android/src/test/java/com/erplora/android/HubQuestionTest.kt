package com.erplora.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

/**
 * The question the app asks before a link links another hub (hub#2644). The app sends the words in
 * English and in Spanish; what is decided here is which ones the device reads, and that a question
 * with missing words is refused instead of shown half empty (a dialog with no «Open» cannot be
 * answered yes, but one with no «Cancel» cannot be answered no).
 */
class HubQuestionTest {

    @Test
    fun `a device in Spanish reads the Spanish words`() {
        assertEquals("es", HubQuestion.languageOf("es"))
        assertEquals("es", HubQuestion.languageOf("ES"))
    }

    @Test
    fun `a device that does not say its language reads Spanish`() {
        // The rule of the bundled offline page and of the Play refusal notice.
        assertEquals("es", HubQuestion.languageOf(null))
        assertEquals("es", HubQuestion.languageOf(""))
    }

    @Test
    fun `any other language reads English, the source`() {
        for (language in listOf("en", "ca", "fr", "eu")) {
            assertEquals("en", HubQuestion.languageOf(language), language)
        }
    }

    @Test
    fun `the four words make a question`() {
        assertEquals(
            HubQuestion.Words("Open b?", "It will use the printer.", "Open", "Cancel"),
            HubQuestion.wordsOf("Open b?", "It will use the printer.", "Open", "Cancel"),
        )
    }

    @Test
    fun `a question with missing words is refused instead of shown`() {
        assertNull(HubQuestion.wordsOf(null, "m", "o", "c"))
        assertNull(HubQuestion.wordsOf("t", " ", "o", "c"))
        assertNull(HubQuestion.wordsOf("t", "m", "", "c"))
        assertNull(HubQuestion.wordsOf("t", "m", "o", null))
        // A blank «Cancel» is a button nobody can read: the no would be lost all the same.
        assertNull(HubQuestion.wordsOf("t", "m", "o", " "))
    }
}
