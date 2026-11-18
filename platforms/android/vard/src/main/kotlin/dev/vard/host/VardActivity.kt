package dev.vard.host

import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.WindowManager
import android.widget.FrameLayout
import android.window.BackEvent
import android.window.OnBackAnimationCallback
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher

/**
 * A full-screen Vard app: loads the project from `assets/app` (bundled by Gradle), installs
 * the core modules plus every module library on the classpath, and wires the platform:
 *
 *  - edge-to-edge drawing on every API level; bar/cutout/keyboard insets go to the core
 *    (`safeArea`), status/navigation bar icons follow the app's theme
 *  - predictive back: while an `on back` handler is enabled the app owns the gesture and
 *    receives its progress (`backProgress`); otherwise the system animates back-to-home
 *  - lifecycle (`appResume`/`appPause`), notification taps (`pushOpened`), deep links
 *  - runtime permission results for [PermissionsModule]
 *
 * Module libraries register themselves in their manifest:
 * `<meta-data android:name="dev.vard.module.<name>" android:value="<class implementing VardModule>"/>`.
 */
open class VardActivity : Activity() {

    lateinit var vard: VardView
        private set
    lateinit var host: VardHost
        private set

    /** Asset folder that holds the Vard project. */
    open val appDir = "app"

    /** Extra modules to install, in addition to the core ones and discovered libraries. */
    open fun modules(): List<VardModule> = emptyList()

    private var backCallback: Any? = null
    private var lastDark: Boolean? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // app sources, package sources (`@name/...`) and vard.lock (the package graph)
        val files = listAssets(appDir).filter { it.endsWith(".vard") || it == "$appDir/vard.lock" }.sorted()
            .map { path -> path.removePrefix("$appDir/") to assets.open(path).bufferedReader().use { it.readText() } }
        val fonts = listAssets("$appDir/fonts").filter { it.endsWith(".ttf") || it.endsWith(".otf") }

        host = VardHost(this).also { it.activity = this }
        CoreModules.all().forEach { host.register(it) }
        discoverModules().forEach { host.register(it) }
        modules().forEach { host.register(it) }

        vard = VardView(this, files, host, fonts, appDir)
        val root = FrameLayout(this)
        root.addView(vard, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        setContentView(root)
        edgeToEdge(root)
        vard.onFrame = { dark, back ->
            barIcons(dark)
            updateBack(back)
        }
        handleIntent(intent)
    }

    // ---------------- edge-to-edge ----------------
    @Suppress("DEPRECATION")
    private fun edgeToEdge(root: View) {
        window.statusBarColor = Color.TRANSPARENT
        window.navigationBarColor = Color.TRANSPARENT
        if (Build.VERSION.SDK_INT >= 29) {
            window.isNavigationBarContrastEnforced = false
            window.isStatusBarContrastEnforced = false
        }
        if (Build.VERSION.SDK_INT >= 28) {
            window.attributes = window.attributes.apply {
                layoutInDisplayCutoutMode = if (Build.VERSION.SDK_INT >= 30) WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
                else WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
            }
        }
        if (Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(false)
            root.setOnApplyWindowInsetsListener { _, insets ->
                val bars = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
                val ime = insets.getInsets(WindowInsets.Type.ime()).bottom
                vard.setInsets(bars.top, bars.bottom, bars.left, bars.right, ime)
                WindowInsets.CONSUMED
            }
        } else {
            root.systemUiVisibility = View.SYSTEM_UI_FLAG_LAYOUT_STABLE or View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN or View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
            root.setOnApplyWindowInsetsListener { _, insets ->
                val stableBottom = insets.stableInsetBottom
                val keyboard = if (insets.systemWindowInsetBottom > stableBottom) insets.systemWindowInsetBottom else 0
                vard.setInsets(insets.systemWindowInsetTop, stableBottom, insets.systemWindowInsetLeft, insets.systemWindowInsetRight, keyboard)
                insets.consumeSystemWindowInsets()
            }
        }
    }

    @Suppress("DEPRECATION")
    private fun barIcons(dark: Boolean) {
        if (dark == lastDark) return
        lastDark = dark
        if (Build.VERSION.SDK_INT >= 30) {
            val light = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS
            window.insetsController?.setSystemBarsAppearance(if (dark) 0 else light, light)
        } else {
            val v = window.decorView
            val flags = View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR or View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR
            v.systemUiVisibility = if (dark) v.systemUiVisibility and flags.inv() else v.systemUiVisibility or flags
        }
    }

    // ---------------- predictive back ----------------
    private fun updateBack(enabled: Boolean) {
        if (Build.VERSION.SDK_INT < 33) return // onBackPressed asks the app each time
        val registered = backCallback != null
        if (enabled == registered) return
        val dispatcher = onBackInvokedDispatcher
        if (enabled) {
            val cb: OnBackInvokedCallback = if (Build.VERSION.SDK_INT >= 34) {
                object : OnBackAnimationCallback {
                    override fun onBackStarted(e: BackEvent) = vard.backProgress(0f)
                    override fun onBackProgressed(e: BackEvent) = vard.backProgress(e.progress)
                    override fun onBackCancelled() = vard.backCancel()
                    override fun onBackInvoked() {
                        if (!vard.back()) finishAfterTransition()
                    }
                }
            } else {
                OnBackInvokedCallback { if (!vard.back()) finishAfterTransition() }
            }
            dispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT, cb)
            backCallback = cb
        } else {
            dispatcher.unregisterOnBackInvokedCallback(backCallback as OnBackInvokedCallback)
            backCallback = null
        }
    }

    @Deprecated("API < 33 path; 33+ uses OnBackInvokedCallback")
    override fun onBackPressed() {
        @Suppress("DEPRECATION")
        if (!vard.back()) super.onBackPressed()
    }

    // ---------------- lifecycle, intents, results ----------------
    override fun onResume() {
        super.onResume()
        VardPush.foreground = host
        host.emit("appResume")
    }

    override fun onPause() {
        if (VardPush.foreground === host) VardPush.foreground = null
        host.emit("appPause")
        super.onPause()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIntent(intent)
    }

    private fun handleIntent(intent: Intent?) {
        VardPush.fromIntent(intent)?.let { host.emit("pushOpened", it) }
        if (intent?.action == Intent.ACTION_VIEW) intent.dataString?.let { host.emit("deepLink", it) }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        host.onRequestPermissionsResult(requestCode, permissions, grantResults)
    }

    @Deprecated("Activity results for modules")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        host.onActivityResult(requestCode, resultCode, data)
    }

    override fun onDestroy() {
        if (VardPush.foreground === host) VardPush.foreground = null
        vard.destroy()
        super.onDestroy()
    }

    // ---------------- helpers ----------------
    private fun listAssets(dir: String): List<String> {
        val names = assets.list(dir) ?: return emptyList()
        return names.flatMap { n ->
            val p = "$dir/$n"
            val sub = assets.list(p)
            if (sub.isNullOrEmpty()) listOf(p) else listAssets(p)
        }
    }

    private fun discoverModules(): List<VardModule> {
        val meta = try {
            packageManager.getApplicationInfo(packageName, PackageManager.GET_META_DATA).metaData
        } catch (e: Exception) {
            null
        } ?: return emptyList()
        return meta.keySet().filter { it.startsWith("dev.vard.module.") }.sorted().mapNotNull { key ->
            val cls = meta.getString(key) ?: return@mapNotNull null
            try {
                Class.forName(cls).getDeclaredConstructor().newInstance() as VardModule
            } catch (e: Exception) {
                Log.e(VardHost.TAG, "module $key ($cls) could not be created", e)
                null
            }
        }
    }

}
