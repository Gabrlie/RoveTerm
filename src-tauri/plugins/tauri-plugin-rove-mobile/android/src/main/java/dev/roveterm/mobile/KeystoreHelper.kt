package dev.roveterm.mobile

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import org.json.JSONException
import org.json.JSONObject
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

internal class BridgeFailure(val code: String, message: String) : Exception(message)

internal const val MASTER_KEY_LENGTH = 32

internal class KeystoreHelper(context: Context) {
    private val preferences = context.getSharedPreferences("roveterm_vault_keys", Context.MODE_PRIVATE)

    internal data class Envelope(val biometric: Boolean, val iv: ByteArray, val ciphertext: ByteArray)

    fun serviceId(service: String): String {
        if (service.isBlank() || '\u0000' in service) {
            throw BridgeFailure("invalid_input", "Invalid vault service name")
        }

        val digest = MessageDigest.getInstance("SHA-256").digest(service.toByteArray(Charsets.UTF_8))
        return Base64.encodeToString(digest, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
    }

    fun envelope(id: String): Envelope? {
        val stored = preferences.getString(id, null) ?: return null
        try {
            val json = JSONObject(stored)
            if (json.get("version") != 1) throw JSONException("Unknown envelope version")
            val biometric = json.get("biometric") as? Boolean
                ?: throw JSONException("Invalid authentication mode")
            val iv = Base64.decode(json.getString("iv"), Base64.NO_WRAP)
            val ciphertext = Base64.decode(json.getString("ciphertext"), Base64.NO_WRAP)
            if (iv.size != IV_LENGTH || ciphertext.size != MASTER_KEY_LENGTH + TAG_BITS / 8) {
                throw JSONException("Invalid envelope length")
            }

            return Envelope(biometric, iv, ciphertext)
        } catch (_: JSONException) {
            throw BridgeFailure("malformed", "Invalid vault envelope")
        } catch (_: IllegalArgumentException) {
            throw BridgeFailure("malformed", "Invalid vault envelope encoding")
        }
    }

    fun decryptCipher(id: String, envelope: Envelope): Cipher {
        val entry = keyStore().getKey(alias(id, envelope.biometric), null) as? SecretKey
            ?: throw BridgeFailure("key_invalidated", "The wrapping key is missing")
        return Cipher.getInstance(TRANSFORMATION).apply {
            init(Cipher.DECRYPT_MODE, entry, GCMParameterSpec(TAG_BITS, envelope.iv))
        }
    }

    fun encryptCipher(id: String, biometric: Boolean): Cipher {
        val alias = alias(id, biometric)
        val key = keyStore().getKey(alias, null) as? SecretKey ?: generate(alias, biometric)
        return Cipher.getInstance(TRANSFORMATION).apply {
            init(Cipher.ENCRYPT_MODE, key)
        }
    }

    fun save(id: String, biometric: Boolean, cipher: Cipher, key: ByteArray) {
        val envelope = JSONObject()
            .put("version", 1)
            .put("biometric", biometric)
            .put("iv", Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
            .put("ciphertext", Base64.encodeToString(cipher.doFinal(key), Base64.NO_WRAP))
        val previous = preferences.getString(id, null)
        if (!preferences.edit().putString(id, envelope.toString()).commit()) {
            // A failed commit can still update the in-memory preferences.
            preferences.edit().putString(id, previous).apply()
            throw BridgeFailure("unavailable", "Could not save the vault envelope")
        }

        try {
            keyStore().deleteEntry(alias(id, !biometric))
        } catch (_: Exception) {
            // The saved envelope already selects the new wrapping key.
            android.util.Log.w("RoveMobile", "Could not remove the inactive wrapping key")
        }
    }

    private fun keyStore(): KeyStore = KeyStore.getInstance(PROVIDER).apply { load(null) }

    private fun alias(id: String, biometric: Boolean): String =
        "roveterm.vault.$id.${if (biometric) "biometric" else "silent"}"

    @Suppress("DEPRECATION")
    private fun generate(alias: String, biometric: Boolean): SecretKey {
        val spec = KeyGenParameterSpec.Builder(
            alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setKeySize(256)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setUserAuthenticationRequired(biometric)
        if (biometric) {
            spec.setInvalidatedByBiometricEnrollment(false)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                spec.setUserAuthenticationParameters(
                    0,
                    KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL,
                )
            } else {
                spec.setUserAuthenticationValidityDurationSeconds(30)
            }
        }

        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER).run {
            init(spec.build())
            generateKey()
        }
    }

    private companion object {
        const val PROVIDER = "AndroidKeyStore"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_LENGTH = 12
        const val TAG_BITS = 128
    }
}
