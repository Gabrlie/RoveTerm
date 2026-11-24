package dev.roveterm.mobile

import android.app.Activity
import android.view.WindowManager
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import app.tauri.plugin.Channel
import app.tauri.plugin.JSObject

internal class InsetsObserver(private val activity: Activity) {
    private data class Insets(
        val top: Double = 0.0,
        val right: Double = 0.0,
        val bottom: Double = 0.0,
        val left: Double = 0.0,
        val imeBottom: Double = 0.0,
    ) {
        fun json() = JSObject()
            .put("top", top)
            .put("right", right)
            .put("bottom", bottom)
            .put("left", left)
            .put("imeBottom", imeBottom)
    }

    private var channel: Channel? = null
    private var latest: Insets? = null

    @Suppress("DEPRECATION")
    fun register(channel: Channel) {
        val window = activity.window
        val decor = window.decorView
        if (this.channel == null) {
            if (android.os.Build.VERSION.SDK_INT < 30) {
                val mode = window.attributes.softInputMode
                window.setSoftInputMode(
                    (mode and WindowManager.LayoutParams.SOFT_INPUT_MASK_ADJUST.inv()) or
                        WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE,
                )
            }
            ViewCompat.setOnApplyWindowInsetsListener(decor) { view, insets ->
                update(read(insets))
                ViewCompat.onApplyWindowInsets(view, insets)
            }
        }

        this.channel = channel
        latest = null
        update(ViewCompat.getRootWindowInsets(decor)?.let(::read) ?: Insets())
        ViewCompat.requestApplyInsets(decor)
    }

    fun snapshot(): JSObject =
        (latest ?: ViewCompat.getRootWindowInsets(activity.window.decorView)?.let(::read) ?: Insets()).json()

    fun close() {
        if (channel != null) {
            ViewCompat.setOnApplyWindowInsetsListener(activity.window.decorView, null)
            channel = null
        }
    }

    private fun update(insets: Insets) {
        if (latest != insets) {
            latest = insets
            channel?.send(insets.json())
        }
    }

    private fun read(insets: WindowInsetsCompat): Insets {
        val density = activity.resources.displayMetrics.density.toDouble()
        val safe = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
        val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
        return Insets(safe.top / density, safe.right / density, safe.bottom / density, safe.left / density, ime.bottom / density)
    }
}
