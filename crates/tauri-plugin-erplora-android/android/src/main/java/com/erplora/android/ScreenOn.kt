package com.erplora.android

import android.view.WindowManager

object ScreenOn {
    fun apply(addFlags: (Int) -> Unit) {
        addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    }
}
