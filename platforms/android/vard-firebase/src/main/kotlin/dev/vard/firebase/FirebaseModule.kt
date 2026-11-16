package dev.vard.firebase

import android.os.Bundle
import android.util.Base64
import android.util.Log
import com.google.android.gms.tasks.Task
import com.google.firebase.FirebaseApp
import com.google.firebase.Timestamp
import com.google.firebase.analytics.FirebaseAnalytics
import com.google.firebase.auth.FirebaseAuth
import com.google.firebase.auth.FirebaseAuthException
import com.google.firebase.auth.FirebaseUser
import com.google.firebase.firestore.Blob
import com.google.firebase.firestore.DocumentReference
import com.google.firebase.firestore.DocumentSnapshot
import com.google.firebase.firestore.FirebaseFirestore
import com.google.firebase.firestore.FirebaseFirestoreException
import com.google.firebase.firestore.GeoPoint
import com.google.firebase.firestore.ListenerRegistration
import com.google.firebase.firestore.SetOptions
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import dev.vard.host.VardCall
import dev.vard.host.VardHost
import dev.vard.host.VardModule
import dev.vard.host.VardPush
import org.json.JSONArray
import org.json.JSONObject

/**
 * Implements knowledge/modules/firebase.vard. Results cross as JSON; the Vard runtime
 * decodes them to whatever types the app declared, so `firestoreGet*`/`firestoreList*`...
 * work for every document struct (see [resolve]).
 *
 * Without google-services.json Firebase cannot start: every call then answers a
 * `FirebaseError(code: "not-configured")` and the reason is logged once.
 */
class FirebaseModule : VardModule {
    private lateinit var host: VardHost
    private var ready = false
    private val listeners = HashMap<Long, ListenerRegistration>()

    private val auth get() = FirebaseAuth.getInstance()
    private val db get() = FirebaseFirestore.getInstance()

    override fun install(host: VardHost) {
        this.host = host
        ready = try {
            FirebaseApp.getApps(host.context).isNotEmpty() || FirebaseApp.initializeApp(host.context) != null
        } catch (e: Exception) {
            false
        }
        if (!ready) {
            Log.e(TAG, "Firebase is not configured: put google-services.json into platforms/android/app/ and build with -Pvard.firebase=true")
            return
        }
        VardPush.tokenProvider = { done ->
            FirebaseMessaging.getInstance().token.addOnCompleteListener { t -> done(if (t.isSuccessful) t.result else null) }
        }
        auth.addAuthStateListener { a -> host.emit("authChanged", user(a.currentUser)) }
    }

    // ---------------- conversions ----------------
    private fun user(u: FirebaseUser?): Map<String, Any?>? =
        u?.let { mapOf("uid" to it.uid, "email" to it.email, "displayName" to it.displayName, "isAnonymous" to it.isAnonymous) }

    private fun error(e: Exception?): Map<String, Any?>? = e?.let {
        val code = when (it) {
            is FirebaseAuthException -> it.errorCode.lowercase().replace('_', '-')
            is FirebaseFirestoreException -> it.code.name.lowercase().replace('_', '-')
            else -> it.javaClass.simpleName
        }
        mapOf("code" to code, "message" to it.message)
    }

    private val notConfigured = mapOf("code" to "not-configured", "message" to "google-services.json is missing; Firebase did not start")

    private fun authResult(t: Task<*>): Map<String, Any?> =
        if (t.isSuccessful) mapOf("user" to user(auth.currentUser)) else mapOf("error" to error(t.exception))

    /** Firestore value -> JSON-friendly value. */
    private fun out(v: Any?): Any? = when (v) {
        null -> null
        is Timestamp -> v.toDate().time
        is GeoPoint -> mapOf("latitude" to v.latitude, "longitude" to v.longitude)
        is DocumentReference -> v.path
        is Blob -> Base64.encodeToString(v.toBytes(), Base64.NO_WRAP)
        is Map<*, *> -> v.entries.associate { (k, x) -> k.toString() to out(x) }
        is List<*> -> v.map { out(it) }
        else -> v
    }

    private fun doc(s: DocumentSnapshot?): Map<String, Any?>? {
        if (s == null || !s.exists()) return null
        return (s.data ?: emptyMap()).mapValues { out(it.value) } + ("id" to s.id)
    }

    /** JSON from Vard -> Firestore value (the `id` field stays out of the document). */
    private fun into(v: Any?): Any? = when (v) {
        null, JSONObject.NULL -> null
        is JSONObject -> v.keys().asSequence().filter { it != "id" }.associateWith { into(v.opt(it)) }
        is JSONArray -> (0 until v.length()).map { into(v.opt(it)) }
        else -> v
    }

    @Suppress("UNCHECKED_CAST")
    private fun data(c: VardCall, i: Int) = into(c.obj(i)) as Map<String, Any?>

    // ---------------- fixed functions ----------------
    override val functions: Map<String, (VardCall) -> Any?> = mapOf(
        "authCurrentUser" to { _ -> if (ready) user(auth.currentUser) else null },
        "authSignInAnonymously" to { c ->
            val done = c.callback(0)
            if (!ready) done.resolve(mapOf("error" to notConfigured))
            else auth.signInAnonymously().addOnCompleteListener { done.resolve(authResult(it)) }
            null
        },
        "authSignInWithEmail" to { c ->
            val done = c.callback(2)
            if (!ready) done.resolve(mapOf("error" to notConfigured))
            else auth.signInWithEmailAndPassword(c.string(0), c.string(1)).addOnCompleteListener { done.resolve(authResult(it)) }
            null
        },
        "authCreateUser" to { c ->
            val done = c.callback(2)
            if (!ready) done.resolve(mapOf("error" to notConfigured))
            else auth.createUserWithEmailAndPassword(c.string(0), c.string(1)).addOnCompleteListener { done.resolve(authResult(it)) }
            null
        },
        "authSendPasswordReset" to { c ->
            val done = c.callback(1)
            if (!ready) done.resolve(notConfigured)
            else auth.sendPasswordResetEmail(c.string(0)).addOnCompleteListener { done.resolve(error(it.exception)) }
            null
        },
        "authSignOut" to { _ -> if (ready) auth.signOut() },
        "analyticsLog" to { c ->
            if (ready) {
                val b = Bundle()
                c.stringMap(1).forEach { (k, v) -> b.putString(k, v) }
                FirebaseAnalytics.getInstance(host.context).logEvent(c.string(0), b)
            }
        },
        "analyticsSetUserId" to { c -> if (ready) FirebaseAnalytics.getInstance(host.context).setUserId(c.stringOrNull(0)) },
    )

    // ---------------- Firestore, any document type ----------------
    override fun resolve(name: String): ((VardCall) -> Any?)? {
        if (!name.startsWith("firestore")) return null
        val op = listOf("Get", "Set", "Add", "Delete", "List", "Where", "Listen").firstOrNull { name.startsWith("firestore$it") } ?: return null
        return { c -> firestore(op, c); null }
    }

    private fun firestore(op: String, c: VardCall) {
        val last = c.size - 1
        val cb = c.callback(last)
        if (!ready) {
            when (op) {
                "Get" -> cb.resolve(listOf(null, notConfigured))
                "Add" -> cb.resolve(listOf(null, notConfigured))
                "List", "Where" -> cb.resolve(listOf(emptyList<Any>(), notConfigured))
                "Listen" -> cb.release()
                else -> cb.resolve(notConfigured)
            }
            return
        }
        when (op) {
            "Get" -> db.document(c.string(0)).get().addOnCompleteListener { t ->
                cb.resolve(listOf(if (t.isSuccessful) doc(t.result) else null, error(t.exception)))
            }
            "Set" -> db.document(c.string(0)).set(data(c, 1), SetOptions.merge()).addOnCompleteListener { t -> cb.resolve(error(t.exception)) }
            "Add" -> db.collection(c.string(0)).add(data(c, 1)).addOnCompleteListener { t ->
                cb.resolve(listOf(if (t.isSuccessful) t.result.id else null, error(t.exception)))
            }
            "Delete" -> db.document(c.string(0)).delete().addOnCompleteListener { t -> cb.resolve(error(t.exception)) }
            "List" -> db.collection(c.string(0)).get().addOnCompleteListener { t ->
                val docs = if (t.isSuccessful) t.result.documents.mapNotNull { doc(it) } else emptyList()
                cb.resolve(listOf(docs, error(t.exception)))
            }
            "Where" -> {
                val (field, value) = c.string(1) to c.string(3)
                val col = db.collection(c.string(0))
                val q = when (c.string(2)) {
                    "==" -> col.whereEqualTo(field, value)
                    "!=" -> col.whereNotEqualTo(field, value)
                    "<" -> col.whereLessThan(field, value)
                    "<=" -> col.whereLessThanOrEqualTo(field, value)
                    ">" -> col.whereGreaterThan(field, value)
                    ">=" -> col.whereGreaterThanOrEqualTo(field, value)
                    "array-contains" -> col.whereArrayContains(field, value)
                    else -> col.whereEqualTo(field, value)
                }
                q.get().addOnCompleteListener { t ->
                    val docs = if (t.isSuccessful) t.result.documents.mapNotNull { doc(it) } else emptyList()
                    cb.resolve(listOf(docs, error(t.exception)))
                }
            }
            "Listen" -> {
                val reg = db.collection(c.string(0)).addSnapshotListener { snap, e ->
                    if (e != null) Log.w(TAG, "listen ${c.string(0)}: ${e.message}")
                    if (snap != null) cb.send(snap.documents.mapNotNull { doc(it) })
                }
                listeners[cb.id] = reg
            }
        }
    }

    companion object {
        const val TAG = "VardFirebase"
    }
}

/** FCM -> Vard: foreground messages become `pushReceived`, background ones a notification. */
class VardMessagingService : FirebaseMessagingService() {
    override fun onMessageReceived(m: RemoteMessage) {
        val n = m.notification
        VardPush.onMessage(
            this,
            VardPush.Message(m.messageId ?: VardPush.newId(), n?.title ?: m.data["title"] ?: "", n?.body ?: m.data["body"] ?: "", m.data),
        )
    }

    override fun onNewToken(token: String) = VardPush.onNewToken(token)
}
