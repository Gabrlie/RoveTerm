package dev.roveterm.mobile

import android.app.Activity
import android.app.KeyguardManager
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.UserNotAuthenticatedException
import android.util.Base64
import androidx.activity.result.ActivityResult
import androidx.appcompat.app.AppCompatActivity
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.biometric.BiometricPrompt
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
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher

@InvokeArg
internal class RegisterInsetsArgs {
    lateinit var channel: Channel
}

@TauriPlugin
class RoveMobilePlugin(private val activity: Activity) : Plugin(activity) {
    private val insets = InsetsObserver(activity)
    private val keystore = KeystoreHelper(activity)
    private val main = Handler(Looper.getMainLooper())
    private val keyguard = activity.getSystemService(KeyguardManager::class.java)
    private var active: Invoke? = null
    private var prompt: BiometricPrompt? = null
    private var legacyOperation: (() -> Unit)? = null

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

    @Command
    fun getInsets(invoke: Invoke) {
        activity.runOnUiThread { invoke.resolve(insets.snapshot()) }
    }

    @Command
    fun registerInsetsChannel(invoke: Invoke) {
        val channel = try {
            invoke.parseArgs(RegisterInsetsArgs::class.java).channel
        } catch (_: Exception) {
            invoke.reject("Invalid insets channel", "invalid_input")
            return
        }
        // The Rust setup can be waiting for the acknowledgement on the UI thread.
        invoke.resolve()
        main.post { if (!activity.isDestroyed) insets.register(channel) }
    }

    override fun onDestroy(activity: AppCompatActivity) {
        if (this.activity !== activity) return
        insets.close()
        val previousPrompt = prompt
        active?.let { invoke ->
            finish(invoke) { invoke.reject("Authentication was interrupted", "denied") }
        }
        previousPrompt?.cancelAuthentication()
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

    private fun JSObject.requireString(name: String): String =
        opt(name) as? String ?: throw BridgeFailure("invalid_input", "Missing or invalid argument: $name")

    private fun JSObject.requireBoolean(name: String): Boolean =
        opt(name) as? Boolean ?: throw BridgeFailure("invalid_input", "Missing or invalid argument: $name")
}
