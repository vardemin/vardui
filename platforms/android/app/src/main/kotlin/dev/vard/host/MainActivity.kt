package dev.vard.host

import android.app.Activity
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.widget.FrameLayout

/** Loads the `.vard` files under `assets/app` (bundled by Gradle from `vard.app`) and shows it full screen. */
class MainActivity : Activity() {

    private lateinit var vard: VardView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val files = (assets.list(APP_DIR) ?: emptyArray())
            .filter { it.endsWith(".vard") }
            .sorted()
            .map { name -> name to assets.open("$APP_DIR/$name").bufferedReader().use { it.readText() } }

        vard = VardView(this, files)
        // The container takes the system bar / IME insets as padding, so the Vard screen is
        // exactly the visible area (and shrinks when the keyboard opens).
        val root = FrameLayout(this)
        root.addView(vard, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            // Draw edge-to-edge (API 35+ always does) with transparent bars over the app background.
            @Suppress("DEPRECATION")
            window.setDecorFitsSystemWindows(false)
            @Suppress("DEPRECATION")
            window.statusBarColor = Color.TRANSPARENT
            @Suppress("DEPRECATION")
            window.navigationBarColor = Color.TRANSPARENT
            root.setOnApplyWindowInsetsListener { v, insets ->
                val i = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout() or WindowInsets.Type.ime())
                v.setPadding(i.left, i.top, i.right, i.bottom)
                WindowInsets.CONSUMED
            }
            var lastDark: Boolean? = null
            vard.onTheme = { dark ->
                if (dark != lastDark) {
                    lastDark = dark
                    val light = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS
                    window.insetsController?.setSystemBarsAppearance(if (dark) 0 else light, light)
                }
            }
        }
        setContentView(root)
    }

    override fun onDestroy() {
        vard.destroy()
        super.onDestroy()
    }

    private companion object {
        const val APP_DIR = "app"
    }
}
