package com.erplora.android

/**
 * The app's window takes no part in Android's autofill (app#15).
 *
 * Every page the window shows is either a business's hub — customers' names, e-mails, phones and
 * notes, health data included — or the SaaS sign-in the owner goes through once. Handed to the
 * system, the hub's forms come back as «Save password to Google Password Manager?» with a
 * customer's e-mail as the user and their notes as the password (QA, 2026-10-30, Android 16 with
 * Google Password Manager). So the window hands out no `AutofillManager` at all.
 *
 * Why the activity, and not the page or the WebView:
 *
 *  - the page cannot: Chromium sends every editable field to the service whatever its
 *    `autocomplete` says, and the service may ignore the hint (it took an `autocomplete="off"`
 *    e-mail as the user name);
 *  - `importantForAutofill` on the WebView cannot either: Chromium's `AutofillProvider` never reads
 *    it, and Android 14+ puts a WebView in the autofill structure whatever its importance
 *    (`always_include_webview_in_assist_structure`, on by default);
 *  - the WebView takes its `AutofillManager` from the context it was created with — wry creates it
 *    with the activity — and Chromium's `AutofillManagerWrapper` treats a missing one as autofill
 *    off: it never tells Android a field was entered or a form was sent. Android's own views and
 *    `Activity` check the manager for `null` too.
 *
 * It is the whole window, the SaaS sign-in included: Chromium takes the manager once per WebView
 * and keeps it, so it cannot be switched on and off by page. The owner types the password once,
 * on the first start (`HUB_APP-F02`); from then on the app opens the business directly.
 */
object AutofillOptOut {

    /**
     * The platform's name for the autofill service: what `getSystemServiceName(AutofillManager)`
     * answers and what `getSystemService(AutofillManager::class.java)` asks for. The SDK hides the
     * constant (`Context.AUTOFILL_MANAGER_SERVICE` is `@hide`); below Android 8 no service has it.
     */
    const val AUTOFILL_SERVICE = "autofill"

    /**
     * The service the window hands out under [name]: nothing for autofill — without asking
     * [platform], so no `AutofillManager` is ever created for the window — and [platform]'s for
     * every other name.
     */
    @JvmStatic
    fun systemService(name: String, platform: (String) -> Any?): Any? =
        if (name == AUTOFILL_SERVICE) null else platform(name)
}
