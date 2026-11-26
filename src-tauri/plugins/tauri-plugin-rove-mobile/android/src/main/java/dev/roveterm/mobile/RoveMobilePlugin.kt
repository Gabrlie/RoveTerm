package dev.roveterm.mobile

import android.Manifest
import android.app.Activity
import android.app.Application
import android.app.KeyguardManager
import android.content.Intent
import android.content.ActivityNotFoundException
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.UserNotAuthenticatedException
import android.util.Base64
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.ActivityResult
import androidx.appcompat.app.AppCompatActivity
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.biometric.BiometricPrompt
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONException
import java.io.File
import java.math.BigInteger
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher

@InvokeArg
internal class RegisterChannelArgs {
    lateinit var channel: Channel
}

@InvokeArg
internal class StageFileArgs {
    lateinit var uri: String
    lateinit var destDir: String
    var maxBytes: BigInteger? = null
}

@InvokeArg
internal class InstallApkArgs {
    lateinit var path: String
}

@TauriPlugin
class RoveMobilePlugin(private val activity: Activity) : Plugin(activity) {
    private companion object {
        const val NOTIFICATION_PERMISSION_REQUEST = 1001
        var notificationPermissionRequested = false
    }

    private val insets = InsetsObserver(activity)
    private val networkMonitor = NetworkMonitor(activity)
    private val keystore = KeystoreHelper(activity)
    private val fileBridge = FileBridge(activity)
    private val main = Handler(Looper.getMainLooper())
    private val keyguard = activity.getSystemService(KeyguardManager::class.java)
    private var active: Invoke? = null
    private var activeExport: PendingExport? = null
    private var pendingInstall: PendingInstall? = null
    private var prompt: BiometricPrompt? = null
    private var legacyOperation: (() -> Unit)? = null
    private var lifecycleChannel: Channel? = null

    private data class PendingExport(val invoke: Invoke, val sourceFile: File)
    private data class PendingInstall(val invoke: Invoke, val apkFile: File)
    private val lifecycleCallbacks = object : Application.ActivityLifecycleCallbacks {
        override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {}
        override fun onActivityStarted(activity: Activity) {}
        override fun onActivityPaused(activity: Activity) {}
        override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) {}
        override fun onActivityDestroyed(activity: Activity) {}

        override fun onActivityResumed(activity: Activity) {
            if (activity === this@RoveMobilePlugin.activity) sendLifecycle("foreground")
        }

        override fun onActivityStopped(activity: Activity) {
            if (activity === this@RoveMobilePlugin.activity) sendLifecycle("background")
        }
    }
    private val backCallback = object : OnBackPressedCallback(true) {
        override fun handleOnBackPressed() {
            lifecycleChannel?.send(JSObject().put("kind", "back"))
        }
    }
    private val disconnectAll: () -> Unit = {
        lifecycleChannel?.send(
            JSObject().put("kind", "keep_alive_action").put("action", "disconnect_all"),
        )
    }

    @Command
    fun loadMasterKey(invoke: Invoke) = withKeyOperation(invoke) { args ->
        val id = keystore.serviceId(args.requireString("service"))
        val silent = args.requireBoolean("silent")
        val envelope = keystore.envelope(id)
        if (envelope == null) {
            finish(invoke) { invoke.resolve(JSObject()) }
        } else if (envelope.biometric && silent) {
            finish(invoke) { invoke.reject("Interactive authentication required", "denied") }
        } else {
            withCipher(invoke, args, envelope.biometric, { keystore.decryptCipher(id, envelope) }) { cipher ->
                val key = cipher.doFinal(envelope.ciphertext)
                try {
                    invoke.resolve(JSObject().put("key", Base64.encodeToString(key, Base64.NO_WRAP)))
                } finally {
                    key.fill(0)
                }
            }
        }
    }

    @Command
    fun storeMasterKey(invoke: Invoke) = withKeyOperation(invoke) { args ->
        val id = keystore.serviceId(args.requireString("service"))
        val biometric = args.requireBoolean("biometric")
        val encoded = args.requireString("key")
        if (encoded.length != 44) throw BridgeFailure("invalid_input", "Invalid master key length")
        val key = try {
            Base64.decode(encoded, Base64.NO_WRAP)
        } catch (_: IllegalArgumentException) {
            throw BridgeFailure("invalid_input", "Invalid master key encoding")
        }
        try {
            if (key.size != MASTER_KEY_LENGTH) {
                throw BridgeFailure("invalid_input", "Invalid master key length")
            }
        } finally {
            key.fill(0)
        }

        withCipher(invoke, args, biometric, { keystore.encryptCipher(id, biometric) }) { cipher ->
            val plaintext = Base64.decode(encoded, Base64.NO_WRAP)
            try {
                keystore.save(id, biometric, cipher, plaintext)
                invoke.resolve()
            } finally {
                plaintext.fill(0)
            }
        }
    }

    @Command
    fun biometricStatus(invoke: Invoke) {
        val status = authenticationStatus()
        val secureLockSupported = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
            activity.packageManager.hasSystemFeature(PackageManager.FEATURE_SECURE_LOCK_SCREEN)
        val supported = status != BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE &&
            status != BiometricManager.BIOMETRIC_ERROR_UNSUPPORTED || secureLockSupported

        invoke.resolve(
            JSObject()
                .put("supported", supported)
                .put("enrolled", status == BiometricManager.BIOMETRIC_SUCCESS),
        )
    }

    @Command
    fun openExternalUrl(invoke: Invoke) {
        activity.runOnUiThread {
            try {
                val uri = Uri.parse(invoke.getArgs().requireString("url"))
                if (uri.scheme !in listOf("http", "https") || uri.host.isNullOrBlank()) {
                    throw BridgeFailure("invalid_input", "External URL requires HTTP or HTTPS and a host")
                }
                activity.startActivity(Intent(Intent.ACTION_VIEW, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                invoke.resolve()
            } catch (error: BridgeFailure) {
                invoke.reject(error.message, error.code)
            } catch (_: JSONException) {
                invoke.reject("Invalid URL arguments", "invalid_input")
            } catch (_: Exception) {
                invoke.reject("Could not open the external URL", "unavailable")
            }
        }
    }

    @Command
    fun installApk(invoke: Invoke) {
        activity.runOnUiThread {
            withFileOperation(invoke) {
                requireFileWindow()
                if (pendingInstall != null) {
                    throw BridgeFailure("unavailable", "An APK install permission request is in progress")
                }
                val path = try {
                    invoke.parseArgs(InstallApkArgs::class.java).path
                } catch (_: Exception) {
                    throw BridgeFailure("invalid_input", "Invalid APK install arguments")
                }
                val apkFile = fileBridge.requireCachedApk(path)
                if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O || activity.packageManager.canRequestPackageInstalls()) {
                    launchApkInstaller(invoke, apkFile)
                } else {
                    if (activeExport != null || legacyOperation != null) {
                        throw BridgeFailure("unavailable", "Another system activity is in progress")
                    }
                    val intent = Intent(
                        Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                        Uri.parse("package:${activity.packageName}"),
                    )
                    pendingInstall = PendingInstall(invoke, apkFile)
                    try {
                        startActivityForResult(invoke, intent, ::installPermissionResult.name)
                    } catch (_: Exception) {
                        // Some devices cannot open this settings screen.
                        pendingInstall = null
                        resolveInstallPermissionRequired(invoke)
                    }
                }
            }
        }
    }

    @ActivityCallback
    @Suppress("UNUSED_PARAMETER")
    fun installPermissionResult(invoke: Invoke, result: ActivityResult) {
        val pending = pendingInstall ?: return
        if (pending.invoke !== invoke) return
        pendingInstall = null
        withFileOperation(invoke) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && activity.packageManager.canRequestPackageInstalls()) {
                launchApkInstaller(invoke, pending.apkFile)
            } else {
                resolveInstallPermissionRequired(invoke)
            }
        }
    }

    private fun launchApkInstaller(invoke: Invoke, apkFile: File) {
        try {
            requireFileWindow()
            fileBridge.launchApkInstaller(apkFile)
            invoke.resolve(JSObject().put("status", "launched"))
        } catch (_: ActivityNotFoundException) {
            invoke.reject("No Android APK installer is available", "unavailable")
        } catch (error: BridgeFailure) {
            invoke.reject(error.message, error.code)
        } catch (_: Exception) {
            invoke.reject("Could not launch the APK installer", "unavailable")
        }
    }

    private fun resolveInstallPermissionRequired(invoke: Invoke) {
        invoke.resolve(JSObject().put("status", "permission_required"))
    }

    @Command
    fun stageFile(invoke: Invoke) {
        withFileOperation(invoke) {
            val args = try {
                invoke.parseArgs(StageFileArgs::class.java)
            } catch (_: Exception) {
                throw BridgeFailure("invalid_input", "Invalid staging arguments")
            }
            if (args.maxBytes?.signum() == -1) {
                throw BridgeFailure("invalid_input", "Maximum file size must be nonnegative")
            }
            // Android file and stream sizes are bounded by a signed Long.
            val maxBytes = args.maxBytes?.min(BigInteger.valueOf(Long.MAX_VALUE))?.toLong()
            fileBridge.execute {
                withFileOperation(invoke) {
                    val file = fileBridge.stageFile(args.uri, args.destDir, maxBytes)
                    invoke.resolve(JSObject().put("path", file.path).put("name", file.name).put("size", file.size))
                }
            }
        }
    }

    @Command
    fun shareFile(invoke: Invoke) {
        activity.runOnUiThread {
            withFileOperation(invoke) {
                requireFileWindow()
                val args = invoke.getArgs()
                fileBridge.shareFile(args.requireString("path"), args.opt("title") as? String)
                invoke.resolve()
            }
        }
    }

    @Command
    fun exportFile(invoke: Invoke) {
        activity.runOnUiThread {
            withFileOperation(invoke) {
                requireFileWindow()
                if (activeExport != null || legacyOperation != null || pendingInstall != null) {
                    throw BridgeFailure("unavailable", "Another system picker is in progress")
                }
                val args = invoke.getArgs()
                val export = fileBridge.prepareExportIntent(
                    args.requireString("sourcePath"), args.requireString("suggestedName"),
                )
                activeExport = PendingExport(invoke, export.sourceFile)
                try {
                    startActivityForResult(invoke, export.intent, ::exportFileResult.name)
                } catch (error: Exception) {
                    finishExport(invoke, Result.failure(error))
                }
            }
        }
    }

    @ActivityCallback
    fun exportFileResult(invoke: Invoke, result: ActivityResult) {
        val pending = activeExport ?: return
        if (pending.invoke !== invoke) return
        if (result.resultCode == Activity.RESULT_CANCELED) {
            finishExport(invoke, Result.success(false))
            return
        }
        val target = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || target == null) {
            finishExport(invoke, Result.failure(BridgeFailure("unavailable", "The export destination is unavailable")))
            return
        }
        try {
            fileBridge.execute {
                val exported = runCatching {
                    fileBridge.copyToUri(pending.sourceFile, target)
                    true
                }
                main.post { finishExport(invoke, exported) }
            }
        } catch (error: Exception) {
            finishExport(invoke, Result.failure(error))
        }
    }

    @Command
    fun getInsets(invoke: Invoke) {
        activity.runOnUiThread { invoke.resolve(insets.snapshot()) }
    }

    @Command
    fun registerInsetsChannel(invoke: Invoke) {
        val channel = try {
            invoke.parseArgs(RegisterChannelArgs::class.java).channel
        } catch (_: Exception) {
            invoke.reject("Invalid insets channel", "invalid_input")
            return
        }
        // The Rust setup can be waiting for the acknowledgement on the UI thread.
        invoke.resolve()
        main.post { if (!activity.isDestroyed) insets.register(channel) }
    }

    @Command
    fun getNetwork(invoke: Invoke) {
        invoke.resolve(networkMonitor.snapshot())
    }

    @Command
    fun registerNetworkChannel(invoke: Invoke) {
        val channel = try {
            invoke.parseArgs(RegisterChannelArgs::class.java).channel
        } catch (_: Exception) {
            invoke.reject("Invalid network channel", "invalid_input")
            return
        }
        invoke.resolve()
        main.post { if (!activity.isDestroyed) networkMonitor.register(channel) }
    }

    @Command
    fun registerLifecycleChannel(invoke: Invoke) {
        val channel = try {
            invoke.parseArgs(RegisterChannelArgs::class.java).channel
        } catch (_: Exception) {
            invoke.reject("Invalid lifecycle channel", "invalid_input")
            return
        }
        invoke.resolve()
        main.post {
            if (activity.isDestroyed) return@post
            val registered = lifecycleChannel != null
            lifecycleChannel = channel
            if (!registered) {
                activity.application.registerActivityLifecycleCallbacks(lifecycleCallbacks)
                (activity as? ComponentActivity)?.let { owner ->
                    owner.onBackPressedDispatcher.addCallback(owner, backCallback)
                }
            }
            SessionKeepAliveService.onDisconnectAll = disconnectAll
        }
    }

    @Command
    fun updateKeepAlive(invoke: Invoke) {
        activity.runOnUiThread {
            try {
                val args = invoke.getArgs()
                if (args.requireBoolean("active")) {
                    SessionKeepAliveService.update(
                        activity,
                        args.requireString("title"),
                        args.requireString("body"),
                        args.requireString("disconnectLabel"),
                    )
                    main.post { requestNotificationPermission() }
                } else {
                    SessionKeepAliveService.stop(activity)
                }
                invoke.resolve()
            } catch (error: BridgeFailure) {
                invoke.reject(error.message, error.code)
            } catch (_: JSONException) {
                invoke.reject("Invalid keepalive arguments", "invalid_input")
            } catch (_: IllegalStateException) {
                // Includes ForegroundServiceStartNotAllowedException on Android 12 and later.
                invoke.reject("Android did not allow starting the keepalive service", "unavailable")
            } catch (_: SecurityException) {
                invoke.reject("Android denied access to the keepalive service", "unavailable")
            }
        }
    }

    @Command
    fun moveToBackground(invoke: Invoke) {
        activity.runOnUiThread {
            activity.moveTaskToBack(true)
            invoke.resolve()
        }
    }

    override fun onDestroy(activity: AppCompatActivity) {
        if (this.activity !== activity) return
        activity.application.unregisterActivityLifecycleCallbacks(lifecycleCallbacks)
        backCallback.remove()
        if (SessionKeepAliveService.onDisconnectAll === disconnectAll) {
            SessionKeepAliveService.onDisconnectAll = null
        }
        lifecycleChannel = null
        insets.close()
        networkMonitor.close()
        activeExport?.let { pending ->
            finishExport(pending.invoke, Result.failure(BridgeFailure("unavailable", "Export was interrupted")))
        }
        val interruptedInstall = pendingInstall
        pendingInstall = null
        interruptedInstall?.invoke?.reject("APK installation was interrupted", "unavailable")
        fileBridge.close()
        val previousPrompt = prompt
        active?.let { invoke ->
            finish(invoke) { invoke.reject("Authentication was interrupted", "denied") }
        }
        previousPrompt?.cancelAuthentication()
    }

    private fun sendLifecycle(state: String) {
        lifecycleChannel?.send(JSObject().put("kind", "lifecycle").put("state", state))
    }

    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            notificationPermissionRequested || activity.isDestroyed || activity.isFinishing ||
            ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        ) return

        notificationPermissionRequested = true
        ActivityCompat.requestPermissions(
            activity, arrayOf(Manifest.permission.POST_NOTIFICATIONS), NOTIFICATION_PERMISSION_REQUEST,
        )
    }

    @ActivityCallback
    fun credentialResult(invoke: Invoke, result: ActivityResult) {
        if (active !== invoke) return
        val operation = legacyOperation
        if (result.resultCode == Activity.RESULT_OK && operation != null) {
            operation()
        } else {
            finish(invoke) { invoke.reject("Authentication was denied", "denied") }
        }
    }

    private fun authenticationStatus(): Int {
        val manager = BiometricManager.from(activity)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            return manager.canAuthenticate(BIOMETRIC_STRONG or DEVICE_CREDENTIAL)
        }

        // AndroidX cannot query the combined strong biometric and credential policy on API 28–29.
        return if (keyguard?.isDeviceSecure == true) {
            BiometricManager.BIOMETRIC_SUCCESS
        } else {
            manager.canAuthenticate(BIOMETRIC_STRONG)
        }
    }

    private fun withKeyOperation(invoke: Invoke, operation: (JSObject) -> Unit) {
        activity.runOnUiThread {
            if (activity.isDestroyed || activity.isFinishing) {
                invoke.reject("No active window for vault access", "unavailable")
                return@runOnUiThread
            }
            if (active != null) {
                invoke.reject("Another vault operation is in progress", "unavailable")
                return@runOnUiThread
            }

            active = invoke
            try {
                operation(invoke.getArgs())
            } catch (error: Exception) {
                finish(invoke) { rejectKeyError(invoke, error) }
            }
        }
    }

    private fun withCipher(
        invoke: Invoke,
        args: JSObject,
        biometric: Boolean,
        prepare: () -> Cipher,
        operation: (Cipher) -> Unit,
    ) {
        if (!biometric) {
            finish(invoke) { operation(prepare()) }
            return
        }

        val host = activity as? FragmentActivity
            ?: throw BridgeFailure("biometric_unavailable", "Authentication requires a fragment activity")
        val modern = Build.VERSION.SDK_INT >= Build.VERSION_CODES.R
        val title = args.requireString("promptTitle")
        val subtitle = args.requireString("promptSubtitle")
        if (title.isBlank()) throw BridgeFailure("invalid_input", "Authentication requires a prompt title")
        if (authenticationStatus() != BiometricManager.BIOMETRIC_SUCCESS) {
            throw BridgeFailure("biometric_unavailable", "Set up device authentication to unlock this vault")
        }

        val hasCredential = keyguard?.isDeviceSecure == true
        val legacyUnlock = {
            try {
                startCredentialConfirmation(invoke, title, subtitle) {
                    finish(invoke) { operation(prepare()) }
                }
            } catch (error: Exception) {
                finish(invoke) { rejectKeyError(invoke, error) }
            }
        }
        if (!modern && BiometricManager.from(activity).canAuthenticate(BIOMETRIC_STRONG) !=
            BiometricManager.BIOMETRIC_SUCCESS
        ) {
            legacyUnlock()
            return
        }

        val info = BiometricPrompt.PromptInfo.Builder().setTitle(title)
        if (subtitle.isNotBlank()) info.setSubtitle(subtitle)
        if (modern) {
            info.setAllowedAuthenticators(BIOMETRIC_STRONG or DEVICE_CREDENTIAL)
        } else {
            info.setAllowedAuthenticators(BIOMETRIC_STRONG)
            info.setNegativeButtonText(
                activity.getString(if (hasCredential) R.string.rove_mobile_use_screen_lock else android.R.string.cancel),
            )
        }

        val callback = object : BiometricPrompt.AuthenticationCallback() {
            override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                if (active !== invoke) return
                if (modern) {
                    finish(invoke) {
                        val cipher = result.cryptoObject?.cipher
                            ?: throw BridgeFailure("denied", "Authentication did not unlock the wrapping key")
                        operation(cipher)
                    }
                } else {
                    try {
                        val cipher = prepare()
                        operation(cipher)
                        clearOperation()
                    } catch (error: UserNotAuthenticatedException) {
                        if (hasCredential) legacyUnlock() else finish(invoke) { rejectKeyError(invoke, error) }
                    } catch (error: Exception) {
                        finish(invoke) { rejectKeyError(invoke, error) }
                    }
                }
            }

            override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                if (active !== invoke) return
                if (!modern && hasCredential && errorCode == BiometricPrompt.ERROR_NEGATIVE_BUTTON) {
                    legacyUnlock()
                    return
                }
                val unavailable = errorCode == BiometricPrompt.ERROR_HW_NOT_PRESENT ||
                    errorCode == BiometricPrompt.ERROR_HW_UNAVAILABLE ||
                    errorCode == BiometricPrompt.ERROR_NO_BIOMETRICS ||
                    errorCode == BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL
                finish(invoke) {
                    val code = if (unavailable) "biometric_unavailable" else "denied"
                    invoke.reject("Device authentication did not complete", code)
                }
            }

            override fun onAuthenticationFailed() {
                if (active !== invoke) return
                val previousPrompt = prompt
                finish(invoke) { invoke.reject("Authentication was denied", "denied") }
                previousPrompt?.cancelAuthentication()
            }
        }

        val prepared = if (modern) prepare() else null
        val instance = BiometricPrompt(host, ContextCompat.getMainExecutor(activity), callback)
        prompt = instance
        if (prepared != null) {
            instance.authenticate(info.build(), BiometricPrompt.CryptoObject(prepared))
        } else {
            instance.authenticate(info.build())
        }
    }

    @Suppress("DEPRECATION")
    private fun startCredentialConfirmation(
        invoke: Invoke, title: String, subtitle: String, operation: () -> Unit,
    ) {
        if (activeExport != null || pendingInstall != null) {
            throw BridgeFailure("unavailable", "Another system activity is in progress")
        }
        val intent = keyguard?.createConfirmDeviceCredentialIntent(title, subtitle)
            ?: throw BridgeFailure("biometric_unavailable", "No device credential is enrolled")
        legacyOperation = operation
        startActivityForResult(invoke, intent, ::credentialResult.name)
    }

    private fun finish(invoke: Invoke, operation: () -> Unit) {
        if (active !== invoke) return
        try {
            operation()
        } catch (error: Exception) {
            rejectKeyError(invoke, error)
        } finally {
            clearOperation()
        }
    }

    private fun clearOperation() {
        active = null
        prompt = null
        legacyOperation = null
    }

    private fun rejectKeyError(invoke: Invoke, error: Exception) {
        when (error) {
            is BridgeFailure -> invoke.reject(error.message, error.code)
            is JSONException -> invoke.reject("Invalid vault arguments", "invalid_input")
            is KeyPermanentlyInvalidatedException -> invoke.reject("Device key invalidated", "key_invalidated")
            is UserNotAuthenticatedException -> invoke.reject("Device key is locked", "denied")
            is AEADBadTagException -> invoke.reject("Invalid vault ciphertext", "malformed")
            else -> invoke.reject("Could not access Android Keystore", "unavailable")
        }
    }

    private fun requireFileWindow() {
        if (activity.isDestroyed || activity.isFinishing) {
            throw BridgeFailure("unavailable", "No active window for file operations")
        }
    }

    private fun finishExport(invoke: Invoke, result: Result<Boolean>) {
        if (activeExport?.invoke !== invoke) return
        activeExport = null
        result.fold(
            { exported -> invoke.resolve(JSObject().put("exported", exported)) },
            { error -> rejectFileError(invoke, error) },
        )
    }

    private fun withFileOperation(invoke: Invoke, operation: () -> Unit) {
        try {
            operation()
        } catch (error: Exception) {
            rejectFileError(invoke, error)
        }
    }

    private fun rejectFileError(invoke: Invoke, error: Throwable) {
        when (error) {
            is BridgeFailure -> invoke.reject(error.message, error.code)
            is JSONException -> invoke.reject("Invalid file arguments", "invalid_input")
            else -> invoke.reject("Could not complete the file operation", "unavailable")
        }
    }

    private fun JSObject.requireString(name: String): String =
        opt(name) as? String ?: throw BridgeFailure("invalid_input", "Missing or invalid argument: $name")

    private fun JSObject.requireBoolean(name: String): Boolean =
        opt(name) as? Boolean ?: throw BridgeFailure("invalid_input", "Missing or invalid argument: $name")
}
