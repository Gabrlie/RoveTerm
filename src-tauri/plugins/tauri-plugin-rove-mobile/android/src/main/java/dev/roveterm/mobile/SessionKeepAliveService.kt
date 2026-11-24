package dev.roveterm.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.annotation.MainThread
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat

class SessionKeepAliveService : Service() {
    companion object {
        internal const val ACTION_UPDATE = "dev.roveterm.mobile.action.UPDATE_KEEPALIVE"
        internal const val ACTION_STOP = "dev.roveterm.mobile.action.STOP_KEEPALIVE"
        internal const val ACTION_DISCONNECT_ALL = "dev.roveterm.mobile.action.DISCONNECT_ALL"
        private const val EXTRA_TITLE = "title"
        private const val EXTRA_BODY = "body"
        private const val EXTRA_DISCONNECT_LABEL = "disconnectLabel"
        private const val CHANNEL_ID = "rove_session_keepalive"
        private const val NOTIFICATION_ID = 1001
        private const val PENDING_INTENT_FLAGS =
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE

        @Volatile
        internal var onDisconnectAll: (() -> Unit)? = null

        private var pendingUpdates = 0
        private var stopAfterUpdates = false

        @MainThread
        internal fun update(context: Context, title: String, body: String, disconnectLabel: String) {
            val intent = Intent(context, SessionKeepAliveService::class.java)
                .setAction(ACTION_UPDATE)
                .putExtra(EXTRA_TITLE, title)
                .putExtra(EXTRA_BODY, body)
                .putExtra(EXTRA_DISCONNECT_LABEL, disconnectLabel)
            checkNotNull(ContextCompat.startForegroundService(context, intent))
            pendingUpdates++
            stopAfterUpdates = false
        }

        @MainThread
        internal fun stop(context: Context) {
            // Each foreground start must be delivered before the service can be stopped.
            if (pendingUpdates > 0) {
                stopAfterUpdates = true
            } else {
                context.stopService(Intent(context, SessionKeepAliveService::class.java))
            }
        }
    }

    private var wakeLock: PowerManager.WakeLock? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_UPDATE -> {
                val notification = buildNotification(intent)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                    startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
                } else {
                    startForeground(NOTIFICATION_ID, notification)
                }
                pendingUpdates = (pendingUpdates - 1).coerceAtLeast(0)
                if (stopAfterUpdates && pendingUpdates == 0) {
                    stopAfterUpdates = false
                    stopKeepAlive(startId)
                } else {
                    acquireWakeLock()
                }
            }
            ACTION_STOP -> stopKeepAlive(startId)
            ACTION_DISCONNECT_ALL -> {
                try {
                    onDisconnectAll?.invoke()
                } finally {
                    stopKeepAlive(startId)
                }
            }
            else -> stopKeepAlive(startId)
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        releaseWakeLock()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun buildNotification(intent: Intent): Notification {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val name = applicationInfo.loadLabel(packageManager)
            val channel = NotificationChannel(CHANNEL_ID, name, NotificationManager.IMPORTANCE_LOW)
            channel.setSound(null, null)
            channel.enableVibration(false)
            getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
        }

        val launchIntent = packageManager.getLaunchIntentForPackage(packageName)
        val contentIntent = launchIntent?.let {
            it.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            PendingIntent.getActivity(this, 0, it, PENDING_INTENT_FLAGS)
        }
        val disconnectIntent = Intent(this, SessionKeepAliveService::class.java).setAction(ACTION_DISCONNECT_ALL)
        val disconnect = PendingIntent.getService(this, 0, disconnectIntent, PENDING_INTENT_FLAGS)

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_popup_sync)
            .setContentTitle(intent.getStringExtra(EXTRA_TITLE))
            .setContentText(intent.getStringExtra(EXTRA_BODY))
            .setContentIntent(contentIntent)
            .setOngoing(true)
            .setSilent(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .addAction(0, intent.getStringExtra(EXTRA_DISCONNECT_LABEL), disconnect)
            .build()
    }

    @Suppress("WakelockTimeout")
    private fun acquireWakeLock() {
        val lock = wakeLock ?: getSystemService(PowerManager::class.java)
            .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "RoveTerm:SessionKeepAlive")
            .also { wakeLock = it }
        if (!lock.isHeld) lock.acquire()
    }

    private fun releaseWakeLock() {
        wakeLock?.takeIf { it.isHeld }?.release()
    }

    private fun stopKeepAlive(startId: Int) {
        releaseWakeLock()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf(startId)
    }
}
