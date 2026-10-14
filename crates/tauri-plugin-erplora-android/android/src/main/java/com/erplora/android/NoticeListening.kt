package com.erplora.android

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat

/**
 * hub#2307 — the pure decisions behind keeping the app listening for notices with the screen off.
 *
 * Every notice of the shell is born in the page — the bell polls its counters, the kitchen and the
 * diary listen to the event socket — so they only arrive while Android keeps the app running. With
 * the screen off, in the background or under Doze it did not: a foreground service is what keeps
 * the process (and the network, which Doze cuts for everything else) alive, the way order-taking
 * apps for merchants do. Android shows it with an ongoing notification whose words come from the
 * page, in the app's language.
 */
object NoticeListening {
    /** The channel of the ongoing notification. Silent: it says the app listens, it is not a notice. */
    const val CHANNEL_ID = "erplora_notice_listening"

    /**
     * Negative on purpose: the shell's own notices take their ids from the clock (hub#2305), always
     * positive, and one of them landing on this id would replace the ongoing notification.
     */
    const val NOTIFICATION_ID = -2307

    /**
     * Not restarted by the system after it kills the app: the service keeps the PROCESS, the page is
     * what listens, and a service brought back without it would promise «listening» with nothing
     * behind the words. The page asks again when the app is opened.
     */
    const val START_MODE = Service.START_NOT_STICKY

    const val EXTRA_TITLE = "title"
    const val EXTRA_BODY = "body"
    const val EXTRA_CHANNEL = "channel"

    /** The refusal the page hears when the request carries no words to show. */
    const val TEXT_MISSING = "notice_listening_text_missing"

    /** The refusal the page hears when Android does not let the service start (e.g. from the background). */
    const val START_REFUSED = "notice_listening_start_refused"

    /**
     * Android 14 (API 34) refuses `startForeground` without a type. `specialUse` because none of the
     * named ones fits a till waiting for orders: `dataSync` is capped at 6 h a day from Android 15,
     * `remoteMessaging` is for moving messages between devices.
     */
    fun serviceType(sdk: Int): Int =
        if (sdk >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE else 0

    /**
     * Whether the page has to be shown again to Chromium: only while listening, and only once Android
     * has hidden it. A hidden page is frozen by Chromium after a minute of network silence — measured
     * on API 37: the `freeze` event at 60 s with the foreground service running and the process alive,
     * and the socket, the bell's polling and the diary's timers frozen with it. Keeping the process is
     * not enough; the page has to stay "on screen" for its listeners to keep running.
     */
    fun keepsPageShown(listening: Boolean, windowVisibility: Int): Boolean =
        listening && windowVisibility != View.VISIBLE

    data class Texts(val title: String, val body: String, val channel: String)

    /** The words of the notification, or `null` when any is missing: never an empty notification. */
    fun textsOf(title: String?, body: String?, channel: String?): Texts? {
        if (title.isNullOrBlank() || body.isNullOrBlank() || channel.isNullOrBlank()) return null
        return Texts(title, body, channel)
    }
}

/**
 * Android glue around [NoticeListening.keepsPageShown]: while listening, undoes the two ways the app
 * hides its page when it leaves the screen — the activity pausing the WebView (`WryActivity.onPause`)
 * and the window going away (`onWindowVisibilityChanged`). Nothing is drawn: the window has no
 * surface, only the page's timers and sockets keep running.
 */
class PageKeeper(private val webView: WebView) : DefaultLifecycleObserver {
    @Volatile
    var listening = false
        private set

    /**
     * A view that is never laid out, only there to hear the window's visibility: the WebView is built
     * by the runtime and cannot be subclassed, and `OnWindowVisibilityChangeListener` needs API 34.
     */
    private val sentinel = object : View(webView.context) {
        override fun onWindowVisibilityChanged(visibility: Int) {
            super.onWindowVisibilityChanged(visibility)
            onWindowVisibility(visibility)
        }
    }.apply { visibility = View.GONE }

    /** What the sentinel heard from the window. Apart from it so a test can say it too. */
    internal fun onWindowVisibility(visibility: Int) {
        if (NoticeListening.keepsPageShown(listening, visibility)) {
            // Posted: the WebView hears the same change in this pass, after or before this view.
            webView.post { showPage() }
        }
    }

    fun install(root: ViewGroup, owner: LifecycleOwner) {
        if (sentinel.parent == null) root.addView(sentinel, 0, 0)
        owner.lifecycle.addObserver(this)
    }

    fun setListening(on: Boolean) {
        listening = on
    }

    override fun onPause(owner: LifecycleOwner) = keepShownAfterLifecycle()

    override fun onStop(owner: LifecycleOwner) = keepShownAfterLifecycle()

    /** The observer runs before the activity's own `onPause`, which is what pauses the WebView. */
    private fun keepShownAfterLifecycle() {
        if (listening) webView.post { if (listening) showPage() }
    }

    private fun showPage() {
        if (!listening) return
        webView.onResume()
        webView.dispatchWindowVisibilityChanged(View.VISIBLE)
    }
}

/**
 * The foreground service itself: Android glue around [NoticeListening]. It does no work of its own —
 * being there is what keeps the process, its WebView and the page's socket running.
 */
class NoticeListeningService : Service() {

    companion object {
        /** Starts listening, or updates the words of a service already running. */
        fun start(context: Context, texts: NoticeListening.Texts) {
            val intent = Intent(context, NoticeListeningService::class.java)
                .putExtra(NoticeListening.EXTRA_TITLE, texts.title)
                .putExtra(NoticeListening.EXTRA_BODY, texts.body)
                .putExtra(NoticeListening.EXTRA_CHANNEL, texts.channel)
            ContextCompat.startForegroundService(context, intent)
        }

        /** Lets Android reclaim the app again. Stopping a service that is not running is a no-op. */
        fun stop(context: Context) {
            context.stopService(Intent(context, NoticeListeningService::class.java))
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val texts = NoticeListening.textsOf(
            intent?.getStringExtra(NoticeListening.EXTRA_TITLE),
            intent?.getStringExtra(NoticeListening.EXTRA_BODY),
            intent?.getStringExtra(NoticeListening.EXTRA_CHANNEL),
        )
        if (texts == null) {
            // `startForegroundService` obliges a `startForeground`; with nothing to show, stop instead.
            stopSelf()
            return NoticeListening.START_MODE
        }
        ServiceCompat.startForeground(
            this,
            NoticeListening.NOTIFICATION_ID,
            notificationOf(texts),
            NoticeListening.serviceType(Build.VERSION.SDK_INT),
        )
        return NoticeListening.START_MODE
    }

    /** Swiped away from the recent apps: the page is gone, so is the reason to keep running. */
    override fun onTaskRemoved(rootIntent: Intent?) {
        stopSelf()
    }

    private fun notificationOf(texts: NoticeListening.Texts): android.app.Notification {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                NoticeListening.CHANNEL_ID,
                texts.channel,
                NotificationManager.IMPORTANCE_LOW,
            ).apply { setShowBadge(false) }
            getSystemService(NotificationManager::class.java)?.createNotificationChannel(channel)
        }
        val open = packageManager.getLaunchIntentForPackage(packageName)?.let {
            PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }
        return NotificationCompat.Builder(this, NoticeListening.CHANNEL_ID)
            // The same icon as the shell's own notices (the notification plugin's default).
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle(texts.title)
            .setContentText(texts.body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(texts.body))
            .setOngoing(true)
            .setShowWhen(false)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .setContentIntent(open)
            .build()
    }
}
