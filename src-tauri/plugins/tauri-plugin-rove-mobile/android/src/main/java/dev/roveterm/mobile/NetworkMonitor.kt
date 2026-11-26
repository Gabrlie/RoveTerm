package dev.roveterm.mobile

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import app.tauri.plugin.Channel
import app.tauri.plugin.JSObject
import org.json.JSONArray
import java.net.Inet4Address
import java.net.Inet6Address

internal class NetworkMonitor(context: Context) {
    private val manager = context.getSystemService(ConnectivityManager::class.java)
    private val main = Handler(Looper.getMainLooper())

    @Volatile
    private var latest: NetworkInterface? = null

    // Callback state and channel delivery run on the main thread.
    private var activeNetwork: Network? = null
    private var channel: Channel? = null
    private var registered = false
    private var closed = false
    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            main.post {
                if (closed) return@post
                activeNetwork = network
                // Android 7 does not guarantee an initial link properties callback.
                if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
                    safely {
                        if (manager?.activeNetwork == network) {
                            update(manager.getLinkProperties(network)?.let { fromLink(network, it) })
                        }
                    }
                }
            }
        }

        override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) {
            main.post {
                if (!closed && activeNetwork == network) {
                    safely { update(fromLink(network, linkProperties)) }
                }
            }
        }

        override fun onLost(network: Network) {
            main.post {
                if (!closed && activeNetwork == network) {
                    activeNetwork = null
                    update(null)
                }
            }
        }
    }

    init {
        if (manager != null) {
            safely {
                manager.registerDefaultNetworkCallback(callback)
                registered = true
                activeNetwork = manager.activeNetwork
                latest = activeNetwork?.let { network ->
                    manager.getLinkProperties(network)?.let { fromLink(network, it) }
                }
            }
        }
    }

    fun snapshot(): JSObject {
        val interfaces = JSONArray()
        latest?.let { interfaces.put(it.payload()) }
        return JSObject().put("interfaces", interfaces)
    }

    fun register(channel: Channel) {
        if (closed) return
        this.channel = channel
        sendSnapshot()
    }

    fun close() {
        closed = true
        channel = null
        activeNetwork = null
        latest = null
        if (registered) {
            registered = false
            safely { manager?.unregisterNetworkCallback(callback) }
        }
    }

    private fun update(value: NetworkInterface?) {
        if (latest == value) return
        latest = value
        sendSnapshot()
    }

    private fun sendSnapshot() {
        safely { channel?.send(snapshot()) }
    }

    // SecurityException is a RuntimeException and follows the same recovery path.
    private inline fun safely(operation: () -> Unit) {
        try {
            operation()
        } catch (error: RuntimeException) {
            Log.w("RoveMobile", "Network monitor operation failed", error)
        }
    }

    private fun fromLink(network: Network, link: LinkProperties): NetworkInterface {
        val addrs = link.linkAddresses.mapNotNull { address ->
            address.address.hostAddress?.substringBefore('%')?.let { ip ->
                Address(ip, address.prefixLength)
            }
        }.distinct().sortedWith(compareBy(Address::ip, Address::prefixLen))
        return NetworkInterface(
            id = network.networkHandle and Long.MAX_VALUE,
            name = link.interfaceName ?: "net",
            addrs = addrs,
            defaultV4 = link.routes.any { it.isDefaultRoute && it.destination.address is Inet4Address },
            defaultV6 = link.routes.any { it.isDefaultRoute && it.destination.address is Inet6Address },
        )
    }

    private data class Address(val ip: String, val prefixLen: Int)

    private data class NetworkInterface(
        val id: Long,
        val name: String,
        val addrs: List<Address>,
        val defaultV4: Boolean,
        val defaultV6: Boolean,
    ) {
        fun payload(): JSObject {
            val addresses = JSONArray()
            addrs.forEach { addresses.put(JSObject().put("ip", it.ip).put("prefixLen", it.prefixLen)) }
            return JSObject()
                .put("id", id)
                .put("name", name)
                .put("addrs", addresses)
                .put("defaultV4", defaultV4)
                .put("defaultV6", defaultV6)
        }
    }
}
