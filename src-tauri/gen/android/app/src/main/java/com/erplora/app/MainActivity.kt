package com.erplora.app

import android.content.Intent
import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import com.erplora.android.NoticeTaps
import com.erplora.android.SystemBarInsets

class MainActivity : TauriActivity() {
  /**
   * Reserve the status bar and the cutout before anything is drawn (hub#1719).
   *
   * `enableEdgeToEdge()` only asks for the window to extend behind the system bars; nothing was
   * reserving the strip they sit on, so the window's content started at the physical top of the
   * screen and every page loaded inside it — the sign-in page the SaaS serves, the till, a module
   * — painted over the clock.
   *
   * The padding goes on the activity's content view, which is the WebView's container: it is
   * already there at this point (the WebView is not — the Rust side creates it later, during
   * `Rust.onActivityCreate`), it survives the container being handed a different child, and its
   * transparent background lets the reserved strip show the `DayNight` window background instead
   * of the WebView's opaque white.
   */
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    SystemBarInsets.applyTopSystemBarPadding(findViewById<View>(android.R.id.content))
  }

  /**
   * A tap on a notice while the task is alive (hub#2360). When the system had killed the process,
   * the activity comes back with its old launcher intent and the tap arrives here — before the
   * WebView, and so before any plugin, exists: nobody else would hear it. The box keeps it until the
   * page claims it; once the page listens, the notification plugin delivers the taps itself.
   */
  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    NoticeTaps.newIntent(this, intent)
  }
}
