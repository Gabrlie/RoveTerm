package dev.roveterm.mobile

import android.app.Activity
import android.content.ClipData
import android.content.ContentResolver
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import androidx.core.content.FileProvider
import java.io.File
import java.io.IOException
import java.util.Locale
import java.util.concurrent.Executors

internal class FileBridge(private val activity: Activity) {
    private companion object {
        const val BUFFER_SIZE = 32 * 1024
        const val MAX_NAME_SCALARS = 200
        const val MAX_NAME_BYTES = 255
    }

    data class StagedFile(val path: String, val name: String, val size: Long)
    data class Export(val sourceFile: File, val intent: Intent)

    private val executor = Executors.newSingleThreadExecutor()

    fun execute(operation: () -> Unit) {
        executor.execute { operation() }
    }

    fun close() {
        executor.shutdown()
    }

    fun stageFile(uriStr: String, destDirStr: String, maxBytes: Long?): StagedFile {
        val uri = Uri.parse(uriStr).normalizeScheme()
        if ('\u0000' in uriStr || uriStr.substringAfter("://", "").isEmpty() ||
            (uri.scheme != ContentResolver.SCHEME_CONTENT && uri.scheme != ContentResolver.SCHEME_FILE)
        ) {
            throw BridgeFailure("invalid_input", "Staging requires a content or file URI")
        }

        val destDir = canonicalFile(destDirStr)
        if (!isInside(destDir, activity.cacheDir.canonicalFile)) {
            throw BridgeFailure("invalid_input", "Staging directory must be inside the app cache")
        }
        if (!destDir.isDirectory && !destDir.mkdirs() && !destDir.isDirectory) {
            throw IOException("Could not create staging directory")
        }

        val name = sanitizeFileName(displayName(uri) ?: uri.lastPathSegment.orEmpty())
        val output = File(destDir, name)
        val resolver = activity.contentResolver
        var created = false
        var complete = false
        try {
            val size = resolver.openInputStream(uri)?.use { input ->
                if (!output.createNewFile()) {
                    throw BridgeFailure("invalid_input", "A file with this name already exists in the staging directory")
                }
                created = true
                output.outputStream().use { stream ->
                    val buffer = ByteArray(BUFFER_SIZE)
                    var total = 0L
                    while (true) {
                        val count = input.read(buffer)
                        if (count < 0) break
                        if (maxBytes != null && count > maxBytes - total) {
                            throw BridgeFailure("invalid_input", "File exceeds the maximum allowed size")
                        }
                        stream.write(buffer, 0, count)
                        total += count
                    }
                    total
                }
            } ?: throw IOException("Could not open file for staging")
            complete = true
            return StagedFile(output.absolutePath, name, size)
        } finally {
            if (created && !complete) output.delete()
        }
    }

    fun shareFile(path: String, title: String?) {
        val file = requireAppFile(path)
        val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
        val send = Intent(Intent.ACTION_SEND).apply {
            type = mimeType(file)
            putExtra(Intent.EXTRA_STREAM, uri)
            clipData = ClipData.newRawUri(file.name, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        activity.startActivity(Intent.createChooser(send, title ?: file.name))
    }

    fun prepareExportIntent(sourcePath: String, suggestedName: String): Export {
        val source = requireAppFile(sourcePath)
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = mimeType(source)
            putExtra(Intent.EXTRA_TITLE, sanitizeFileName(suggestedName))
        }
        return Export(source, intent)
    }

    fun copyToUri(sourceFile: File, targetUri: Uri) {
        requireAppFile(sourceFile.path).inputStream().use { input ->
            val output = activity.contentResolver.openOutputStream(targetUri, "w")
                ?: throw IOException("Could not open export destination")
            output.use { input.copyTo(it, BUFFER_SIZE) }
        }
    }

    private fun canonicalFile(path: String): File {
        val file = File(path)
        if (!file.isAbsolute || '\u0000' in path) {
            throw BridgeFailure("invalid_input", "File path must be absolute and contain no NUL bytes")
        }
        return file.canonicalFile
    }

    private fun requireAppFile(path: String): File {
        val file = canonicalFile(path)
        val roots = listOfNotNull(
            activity.filesDir, activity.cacheDir, activity.getExternalFilesDir(null), activity.externalCacheDir,
        )
        if (roots.none { isInside(file, it.canonicalFile) } || !file.isFile) {
            throw BridgeFailure("invalid_input", "Expected a regular file inside the app storage")
        }
        return file
    }

    private fun isInside(file: File, root: File): Boolean =
        file == root || file.path.startsWith(root.path + File.separator)

    private fun displayName(uri: Uri): String? {
        if (uri.scheme != ContentResolver.SCHEME_CONTENT) return null
        val column = OpenableColumns.DISPLAY_NAME
        return try {
            activity.contentResolver.query(uri, arrayOf(column), null, null, null)?.use { cursor ->
                val index = cursor.getColumnIndex(column)
                if (index >= 0 && cursor.moveToFirst()) cursor.getString(index) else null
            }?.takeIf { it.isNotBlank() }
        } catch (_: IllegalArgumentException) {
            // Some providers support streaming without exposing file metadata.
            null
        } catch (_: UnsupportedOperationException) {
            // Use the URI component when a provider does not implement queries.
            null
        }
    }

    private fun mimeType(file: File): String =
        MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase(Locale.ROOT))
            ?: "application/octet-stream"

    private fun sanitizeFileName(raw: String): String {
        val name = raw.substringAfterLast('/').substringAfterLast('\\')
            .filterNot { Character.isISOControl(it) }.trim(' ', '.')
        if (name.isEmpty()) return "file"
        val prefix = boundedPrefix(name, MAX_NAME_SCALARS, MAX_NAME_BYTES)
        if (prefix == name) return name

        val dot = name.lastIndexOf('.')
        if (dot >= 0) {
            val extension = name.substring(dot)
            val stem = boundedPrefix(
                name.substring(0, dot),
                MAX_NAME_SCALARS - extension.codePointCount(0, extension.length),
                MAX_NAME_BYTES - extension.toByteArray(Charsets.UTF_8).size,
            )
            if (stem.isNotEmpty()) return stem + extension
        }
        return prefix.trimEnd(' ', '.')
    }

    private fun boundedPrefix(name: String, maxScalars: Int, maxBytes: Int): String {
        var end = 0
        var scalars = 0
        var remainingBytes = maxBytes
        while (end < name.length && scalars < maxScalars) {
            val scalar = name.codePointAt(end)
            val bytes = when {
                scalar <= 0x7F -> 1
                scalar <= 0x7FF -> 2
                scalar <= 0xFFFF -> 3
                else -> 4
            }
            if (bytes > remainingBytes) break
            end += Character.charCount(scalar)
            scalars++
            remainingBytes -= bytes
        }
        return name.substring(0, end)
    }
}
