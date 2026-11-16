package dev.vard.host

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import org.json.JSONObject

/**
 * Push and local notifications, provider-agnostic. A push provider (vard-firebase's
 * messaging service, or any other) calls [onMessage] / [onNewToken] and sets
 * [tokenProvider]. Routing:
 *  - app in the foreground  -> `pushReceived(message)` event, nothing is shown
 *  - app in the background  -> a system notification; tapping it starts the activity and
 *    the app gets `pushOpened(message)` (also on a cold start)
 */
object VardPush {
    class Message(val id: String, val title: String, val body: String, val data: Map<String, String>) : VardJson {
        override fun toJson() = mapOf("id" to id, "title" to title, "body" to body, "data" to data)
        fun encode(): String = JSONObject(toJson()).toString()
    }

    /** Returns the current device token asynchronously (set by the push provider). */
    @Volatile var tokenProvider: ((done: (String?) -> Unit) -> Unit)? = null
    /** The host of the visible activity, if any. */
    @Volatile var foreground: VardHost? = null

    const val EXTRA = "dev.vard.push"
    const val CHANNEL = "vard.default"
    private val main = Handler(Looper.getMainLooper())
    private var counter = 0

    fun newId(): String = "local-${System.currentTimeMillis()}-${counter++}"

    /** Entry point for push providers; any thread. */
    @JvmStatic
    fun onMessage(context: Context, message: Message) {
        val ctx = context.applicationContext
        main.post {
            val host = foreground
            if (host != null) host.emit("pushReceived", message) else notify(ctx, message)
        }
    }

    @JvmStatic
    fun onNewToken(token: String) {
        main.post { foreground?.emit("pushTokenChanged", token) }
    }

    /** Post a system notification that opens the app with `pushOpened(message)`. */
    fun notify(context: Context, message: Message) {
        val nm = context.getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26 && nm.getNotificationChannel(CHANNEL) == null) {
            nm.createNotificationChannel(NotificationChannel(CHANNEL, "Notifications", NotificationManager.IMPORTANCE_HIGH))
        }
        if (Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            Log.w(VardHost.TAG, "notification \"${message.title}\" not shown: POST_NOTIFICATIONS is not granted (requestPermission(.notifications))")
            return
        }
        val launch = context.packageManager.getLaunchIntentForPackage(context.packageName) ?: return
        launch.putExtra(EXTRA, message.encode()).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val pi = PendingIntent.getActivity(context, message.id.hashCode(), launch, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val icon = context.applicationInfo.icon.takeIf { it != 0 } ?: android.R.drawable.ic_dialog_info
        val n = Notification.Builder(context, CHANNEL)
            .setSmallIcon(icon)
            .setContentTitle(message.title)
            .setContentText(message.body)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        nm.notify(message.id.hashCode(), n)
    }

    /** The message an activity was started with: our notifications, or FCM's own tray ones. */
    fun fromIntent(intent: Intent?): Message? {
        if (intent == null) return null
        intent.getStringExtra(EXTRA)?.let { raw ->
            intent.removeExtra(EXTRA)
            val o = JSONObject(raw)
            val d = o.optJSONObject("data") ?: JSONObject()
            return Message(o.optString("id"), o.optString("title"), o.optString("body"), d.keys().asSequence().associateWith { d.optString(it) })
        }
        val extras = intent.extras ?: return null
        val id = extras.getString("google.message_id") ?: return null
        intent.removeExtra("google.message_id")
        val skip = setOf("from", "collapse_key")
        val data = extras.keySet()
            .filter { !it.startsWith("google.") && !it.startsWith("gcm.") && it !in skip }
            .associateWith { extras.getString(it) ?: "" }
        return Message(id, data["title"] ?: "", data["body"] ?: "", data)
    }
}

/**
 * Debug-only push injector (registered by the debug manifest of the app):
 * `adb shell am broadcast -a dev.vard.DEBUG_PUSH -p <package> --es title T --es body B --es data '{"k":"v"}'`
 * goes through exactly the path a real FCM message takes.
 */
class DebugPushReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val data = intent.getStringExtra("data")?.let { raw ->
            val o = JSONObject(raw)
            o.keys().asSequence().associateWith { o.optString(it) }
        } ?: emptyMap()
        VardPush.onMessage(context, VardPush.Message(VardPush.newId(), intent.getStringExtra("title") ?: "", intent.getStringExtra("body") ?: "", data))
    }
}
