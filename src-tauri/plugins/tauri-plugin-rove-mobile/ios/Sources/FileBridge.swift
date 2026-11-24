import Darwin
import Foundation
import UIKit

final class FileBridge {
    struct StagedFile: Encodable {
        let path: String
        let name: String
        let size: UInt64
    }

    private struct ExportFile {
        let url: URL
        let temporaryDirectory: URL?
    }

    private static let bufferSize = 32 * 1024
    private static let maxNameScalars = 200
    private static let maxNameBytes = 255

    private let queue = DispatchQueue(label: "dev.roveterm.mobile.files")
    @MainActor private var activeExport: ExportDelegate?

    func stageFile(
        uri: String, destDir: String, maxBytes: UInt64?,
        completion: @escaping (Result<StagedFile, Error>) -> Void
    ) {
        queue.async {
            completion(Result {
                guard !uri.utf8.contains(0),
                      let separator = uri.range(of: "://"), separator.upperBound != uri.endIndex,
                      let source = URL(string: uri), source.isFileURL
                else {
                    throw BridgeFailure("invalid_input", "Staging requires a file URL")
                }
                let directory = try Self.stagingDirectory(destDir)
                let accessed = source.startAccessingSecurityScopedResource()
                defer { if accessed { source.stopAccessingSecurityScopedResource() } }

                let name = Self.sanitizeFileName(source.lastPathComponent)
                let output = directory.appendingPathComponent(name, isDirectory: false)
                let size = try Self.copyForStaging(source: source, output: output, maxBytes: maxBytes)
                return StagedFile(path: output.path, name: name, size: size)
            })
        }
    }

    @MainActor
    func shareFile(
        path: String, title: String?, from viewController: UIViewController?,
        completion: @escaping (Result<Void, Error>) -> Void
    ) {
        queue.async {
            let file = Result { try Self.appFile(path) }
            DispatchQueue.main.async {
                completion(Result {
                    let url = try file.get()
                    let presenter = try Self.presenter(from: viewController)
                    let controller = UIActivityViewController(activityItems: [url], applicationActivities: nil)
                    controller.title = title
                    if let popover = controller.popoverPresentationController, let view = presenter.viewIfLoaded {
                        popover.sourceView = view
                        popover.sourceRect = CGRect(x: view.bounds.midX, y: view.bounds.midY, width: 1, height: 1)
                        popover.permittedArrowDirections = []
                    }
                    presenter.present(controller, animated: true)
                })
            }
        }
    }

    @MainActor
    func exportFile(
        sourcePath: String, suggestedName: String, from viewController: UIViewController?,
        completion: @escaping (Result<Bool, Error>) -> Void
    ) {
        guard activeExport == nil else {
            completion(.failure(BridgeFailure("unavailable", "Another export is in progress")))
            return
        }

        let delegate = ExportDelegate { [weak self, queue] directory, result in
            self?.activeExport = nil
            if let directory {
                queue.async { try? FileManager.default.removeItem(at: directory) }
            }
            completion(result)
        }
        activeExport = delegate

        queue.async {
            let file = Result { try Self.prepareExport(sourcePath: sourcePath, suggestedName: suggestedName) }
            DispatchQueue.main.async {
                do {
                    let export = try file.get()
                    delegate.temporaryDirectory = export.temporaryDirectory
                    let presenter = try Self.presenter(from: viewController)
                    let picker = UIDocumentPickerViewController(forExporting: [export.url], asCopy: true)
                    picker.delegate = delegate
                    presenter.present(picker, animated: true)
                } catch {
                    delegate.finish(.failure(error))
                }
            }
        }
    }

    private static func copyForStaging(source: URL, output: URL, maxBytes: UInt64?) throws -> UInt64 {
        let input = try FileHandle(forReadingFrom: source)
        defer { try? input.close() }

        let descriptor = open(output.path, O_WRONLY | O_CREAT | O_EXCL, 0o600)
        guard descriptor >= 0 else {
            if errno == EEXIST {
                throw BridgeFailure("invalid_input", "A file with this name already exists in the staging directory")
            }
            throw BridgeFailure("unavailable", "Could not create the staged file")
        }
        let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        do {
            var size: UInt64 = 0
            while let chunk = try input.read(upToCount: bufferSize), !chunk.isEmpty {
                size += UInt64(chunk.count)
                if let maxBytes, size > maxBytes {
                    throw BridgeFailure("invalid_input", "File exceeds the maximum allowed size")
                }
                try handle.write(contentsOf: chunk)
            }
            try handle.close()
            return size
        } catch {
            try? handle.close()
            try? FileManager.default.removeItem(at: output)
            throw error
        }
    }

    private static func prepareExport(sourcePath: String, suggestedName: String) throws -> ExportFile {
        let source = try appFile(sourcePath)
        let name = sanitizeFileName(suggestedName)
        guard source.lastPathComponent != name else {
            return ExportFile(url: source, temporaryDirectory: nil)
        }
        let files = FileManager.default
        let directory = files.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try files.createDirectory(at: directory, withIntermediateDirectories: false)
        let target = directory.appendingPathComponent(name, isDirectory: false)
        do {
            try files.copyItem(at: source, to: target)
        } catch {
            try? files.removeItem(at: directory)
            throw error
        }
        return ExportFile(url: target, temporaryDirectory: directory)
    }

    private static func stagingDirectory(_ path: String) throws -> URL {
        let files = FileManager.default
        let requested = try absoluteURL(path)
        let roots: [URL?] = [
            files.urls(for: .cachesDirectory, in: .userDomainMask).first, files.temporaryDirectory,
        ]
        guard let directory = resolvedDirectoryForCreation(requested),
              isInside(directory, roots: roots.compactMap { $0.flatMap(resolvedDirectoryForCreation) })
        else {
            throw BridgeFailure("invalid_input", "Staging directory must be inside the app cache or temporary storage")
        }
        try files.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }

    private static func resolvedDirectoryForCreation(_ requested: URL) -> URL? {
        var ancestor = requested.standardizedFileURL
        var missingComponents = [String]()
        // Resolve the existing ancestor before creating any missing components.
        while !FileManager.default.fileExists(atPath: ancestor.path) {
            if ancestor.path == "/" { return nil }
            missingComponents.append(ancestor.lastPathComponent)
            ancestor.deleteLastPathComponent()
        }
        guard var directory = resolvedURL(ancestor) else { return nil }
        for component in missingComponents.reversed() {
            directory.appendPathComponent(component, isDirectory: true)
        }
        return directory
    }

    private static func appFile(_ path: String) throws -> URL {
        let requested = try absoluteURL(path)
        let roots = [URL(fileURLWithPath: NSHomeDirectory()), FileManager.default.temporaryDirectory]
        guard let file = resolvedURL(requested),
              isInside(file, roots: roots.compactMap(resolvedURL)),
              try file.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile == true
        else {
            throw BridgeFailure("invalid_input", "Expected a regular file inside the app storage")
        }
        return file
    }

    private static func absoluteURL(_ path: String) throws -> URL {
        guard path.hasPrefix("/"), !path.utf8.contains(0) else {
            throw BridgeFailure("invalid_input", "File path must be absolute and contain no NUL bytes")
        }
        return URL(fileURLWithPath: path)
    }

    private static func resolvedURL(_ url: URL) -> URL? {
        guard let path = realpath(url.path, nil) else { return nil }
        defer { free(path) }
        return URL(fileURLWithPath: String(cString: path))
    }

    private static func isInside(_ file: URL, roots: [URL]) -> Bool {
        roots.contains { root in
            file.path == root.path || file.path.hasPrefix(root.path + "/")
        }
    }

    @MainActor
    private static func presenter(from viewController: UIViewController?) throws -> UIViewController {
        guard var top = viewController else {
            throw BridgeFailure("unavailable", "No active window for file operations")
        }
        while let presented = top.presentedViewController {
            top = presented
        }
        guard top.viewIfLoaded?.window != nil, !top.isBeingDismissed, !top.isBeingPresented else {
            throw BridgeFailure("unavailable", "No active window for file operations")
        }
        return top
    }

    private static func sanitizeFileName(_ raw: String) -> String {
        let component = raw.unicodeScalars.split(omittingEmptySubsequences: false) { $0 == "/" || $0 == "\\" }.last
        let scalars = component?.filter { $0.properties.generalCategory != .control } ?? []
        let trim = CharacterSet(charactersIn: " .")
        let name = String(String.UnicodeScalarView(scalars)).trimmingCharacters(in: trim)
        guard !name.isEmpty else { return "file" }
        let prefix = boundedPrefix(name[...], maxScalars: maxNameScalars, maxBytes: maxNameBytes)
        if prefix == name { return name }

        if let dot = name.unicodeScalars.lastIndex(of: ".") {
            let ext = String(name.unicodeScalars[dot...])
            let stem = boundedPrefix(
                name[..<dot], maxScalars: maxNameScalars - ext.unicodeScalars.count,
                maxBytes: maxNameBytes - ext.utf8.count
            )
            if !stem.isEmpty { return stem + ext }
        }
        return prefix.trimmingCharacters(in: trim)
    }

    private static func boundedPrefix(_ value: Substring, maxScalars: Int, maxBytes: Int) -> String {
        var prefix = String.UnicodeScalarView()
        var bytes = 0
        for scalar in value.unicodeScalars.prefix(max(0, maxScalars)) {
            bytes += scalar.utf8.count
            if bytes > maxBytes { break }
            prefix.append(scalar)
        }
        return String(prefix)
    }
}

@MainActor
private final class ExportDelegate: NSObject, UIDocumentPickerDelegate {
    var temporaryDirectory: URL?
    private var completion: ((URL?, Result<Bool, Error>) -> Void)?

    init(completion: @escaping (URL?, Result<Bool, Error>) -> Void) {
        self.completion = completion
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        finish(.success(true))
    }

    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        finish(.success(false))
    }

    func finish(_ result: Result<Bool, Error>) {
        let callback = completion
        completion = nil
        callback?(temporaryDirectory, result)
    }
}
