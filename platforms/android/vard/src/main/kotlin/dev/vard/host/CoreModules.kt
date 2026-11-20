package dev.vard.host

import android.Manifest
import android.app.Activity
import android.app.NotificationManager
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import android.util.Log
import android.view.HapticFeedbackConstants
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** The std `extern fn`s every host implements (see crates/vard-lang/src/std.vard). */
object CoreModules {
    fun all(): List<VardModule> = listOf(SystemModule(), PrefsModule(), SecureModule(), PermissionsModule(), NotificationsModule())
}

/** now, openUrl, shareText, haptic, openAppSettings. */
class SystemModule : VardModule {
    private lateinit var host: VardHost
    override fun install(host: VardHost) {
        this.host = host
    }

    private fun start(intent: Intent) {
        val a = host.activity
        try {
            if (a != null) a.startActivity(intent) else host.context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        } catch (e: ActivityNotFoundException) {
            Log.w(VardHost.TAG, "no activity for $intent")
        }
    }

    override val functions = mapOf<String, (VardCall) -> Any?>(
        // local wall-clock milliseconds: the zone offset (with DST) is applied here
        "now" to { _ ->
            val t = System.currentTimeMillis()
            t + java.util.TimeZone.getDefault().getOffset(t)
        },
        "openUrl" to { c -> start(Intent(Intent.ACTION_VIEW, Uri.parse(c.string(0)))) },
        "shareText" to { c ->
            val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, c.string(0))
            start(Intent.createChooser(send, null))
        },
        "openAppSettings" to { _ ->
            start(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", host.context.packageName, null)))
        },
        "haptic" to { c ->
            val view = host.activity?.window?.decorView
            val kind = when (c.string(0)) {
                "light" -> HapticFeedbackConstants.CLOCK_TICK
                "heavy" -> HapticFeedbackConstants.LONG_PRESS
                "success" -> if (Build.VERSION.SDK_INT >= 30) HapticFeedbackConstants.CONFIRM else HapticFeedbackConstants.VIRTUAL_KEY
                "warning", "error" -> if (Build.VERSION.SDK_INT >= 30) HapticFeedbackConstants.REJECT else HapticFeedbackConstants.LONG_PRESS
                else -> HapticFeedbackConstants.VIRTUAL_KEY
            }
            view?.performHapticFeedback(kind)
        },
    )
}

/** prefGet/prefSet/prefRemove on SharedPreferences (also backs `pref` members). */
class PrefsModule : VardModule {
    private lateinit var ctx: Context
    private val prefs by lazy { ctx.getSharedPreferences("vard.prefs", Context.MODE_PRIVATE) }
    override fun install(host: VardHost) {
        ctx = host.context
    }

    override val functions = mapOf<String, (VardCall) -> Any?>(
        "prefGet" to { c -> prefs.getString(c.string(0), null) },
        "prefSet" to { c -> prefs.edit().putString(c.string(0), c.string(1)).apply() },
        "prefRemove" to { c -> prefs.edit().remove(c.string(0)).apply() },
    )
}

/**
 * secureGet/secureSet/secureRemove: values encrypted with AES-256-GCM under a key that never
 * leaves the Android Keystore (hardware-backed where available); ciphertext in prefs.
 */
class SecureModule : VardModule {
    private lateinit var ctx: Context
    private val store by lazy { ctx.getSharedPreferences("vard.secure", Context.MODE_PRIVATE) }
    override fun install(host: VardHost) {
        ctx = host.context
    }

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return gen.generateKey()
    }

    fun put(name: String, value: String): Boolean = try {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()) }
        val sealed = cipher.iv + cipher.doFinal(value.toByteArray(Charsets.UTF_8))
        store.edit().putString(name, Base64.encodeToString(sealed, Base64.NO_WRAP)).apply()
        true
    } catch (e: Exception) {
        Log.e(VardHost.TAG, "secureSet failed", e)
        false
    }

    fun get(name: String): String? = try {
        store.getString(name, null)?.let {
            val sealed = Base64.decode(it, Base64.NO_WRAP)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, sealed, 0, 12))
            String(cipher.doFinal(sealed, 12, sealed.size - 12), Charsets.UTF_8)
        }
    } catch (e: Exception) {
        Log.e(VardHost.TAG, "secureGet failed (key reset?)", e)
        null
    }

    override val functions = mapOf<String, (VardCall) -> Any?>(
        "secureGet" to { c -> get(c.string(0)) },
        "secureSet" to { c -> put(c.string(0), c.string(1)) },
        "secureRemove" to { c -> store.edit().remove(c.string(0)).apply() },
    )

    private companion object {
        const val ALIAS = "vard.secure.v1"
    }
}

/**
 * permissionStatus / requestPermission / openAppSettings for the std `Permission` enum.
 * Status: granted | denied (asked, may ask again) | blocked (system will not show the dialog
 * again: open settings) | notDetermined. A permission missing from the manifest is reported
 * in logcat and answers `denied`.
 */
class PermissionsModule : VardModule {
    private lateinit var host: VardHost
    private val waiting = HashMap<Int, Pair<String, VardCallback>>()
    private var nextCode = 0x7A00
    private val asked by lazy { host.context.getSharedPreferences("vard.permissions", Context.MODE_PRIVATE) }

    override fun install(host: VardHost) {
        this.host = host
    }

    private fun manifestNames(p: String): List<String> = when (p) {
        "notifications" -> if (Build.VERSION.SDK_INT >= 33) listOf(Manifest.permission.POST_NOTIFICATIONS) else emptyList()
        "camera" -> listOf(Manifest.permission.CAMERA)
        "microphone" -> listOf(Manifest.permission.RECORD_AUDIO)
        "location" -> listOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION)
        "photos" -> if (Build.VERSION.SDK_INT >= 33) listOf(Manifest.permission.READ_MEDIA_IMAGES) else listOf(Manifest.permission.READ_EXTERNAL_STORAGE)
        "contacts" -> listOf(Manifest.permission.READ_CONTACTS)
        else -> emptyList()
    }

    private fun declared(name: String): Boolean = try {
        val info = host.context.packageManager.getPackageInfo(host.context.packageName, PackageManager.GET_PERMISSIONS)
        info.requestedPermissions?.contains(name) == true
    } catch (e: Exception) {
        false
    }

    fun status(p: String): String {
        val names = manifestNames(p)
        if (names.isEmpty()) {
            // notifications before Android 13: a user switch, no runtime permission
            if (p == "notifications") {
                val nm = host.context.getSystemService(NotificationManager::class.java)
                return if (nm.areNotificationsEnabled()) "granted" else "blocked"
            }
            return "denied"
        }
        val ctx = host.context
        if (names.any { ctx.checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED }) return "granted"
        if (!asked.getBoolean(p, false)) return "notDetermined"
        val act = host.activity ?: return "denied"
        return if (names.any { act.shouldShowRequestPermissionRationale(it) }) "denied" else "blocked"
    }

    override val functions = mapOf<String, (VardCall) -> Any?>(
        "permissionStatus" to { c -> status(c.string(0)) },
        "requestPermission" to { c ->
            val p = c.string(0)
            val done = c.callback(1)
            val names = manifestNames(p)
            val now = status(p)
            val act: Activity? = host.activity
            when {
                now == "granted" || now == "blocked" || names.isEmpty() || act == null -> done.resolve(now)
                names.none { declared(it) } -> {
                    Log.e(VardHost.TAG, "permission `$p` needs <uses-permission android:name=\"${names.first()}\"/> in AndroidManifest.xml")
                    done.resolve("denied")
                }
                else -> {
                    val code = nextCode++
                    waiting[code] = p to done
                    asked.edit().putBoolean(p, true).apply()
                    act.requestPermissions(names.filter { declared(it) }.toTypedArray(), code)
                }
            }
            null
        },
    )

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, results: IntArray): Boolean {
        val (p, done) = waiting.remove(requestCode) ?: return false
        done.resolve(status(p))
        return true
    }
}

/** showNotification and getPushToken; push delivery itself is [VardPush]. */
class NotificationsModule : VardModule {
    private lateinit var host: VardHost
    override fun install(host: VardHost) {
        this.host = host
    }

    override val functions = mapOf<String, (VardCall) -> Any?>(
        "showNotification" to { c ->
            VardPush.notify(host.context, VardPush.Message(VardPush.newId(), c.string(0), c.string(1), c.stringMap(2)))
        },
        "getPushToken" to { c ->
            val done = c.callback(0)
            val provider = VardPush.tokenProvider
            if (provider == null) {
                Log.i(VardHost.TAG, "getPushToken: no push provider installed (add vard-firebase); answering nil")
                done.resolve(null)
            } else {
                provider { token -> done.resolve(token) }
            }
            null
        },
    )
}
