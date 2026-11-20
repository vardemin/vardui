import Foundation

/// A Vard project on disk: the output of `vard bundle --out DIR` (what ships inside the
/// app) or a project folder during development. Holds app sources, package sources
/// (`@name/...`), `vard.lock`, `fonts/` and `assets/`.
public struct VardProject {
    public let root: URL
    public let files: [(String, String)]
    public let fonts: [URL]

    /// Read every `.vard` file (recursively; `.vard/` package caches are named `@pkg/` in
    /// bundles), `vard.lock`, and the font files under `fonts/`.
    public init(directory: URL) throws {
        root = directory.standardizedFileURL
        var files: [(String, String)] = []
        var fonts: [URL] = []
        let fm = FileManager.default
        guard let it = fm.enumerator(at: root, includingPropertiesForKeys: [.isDirectoryKey]) else {
            throw CocoaError(.fileReadNoSuchFile)
        }
        for case let url as URL in it {
            let rel = url.standardizedFileURL.path.dropFirst(root.path.count + 1)
            if rel.hasPrefix(".") || rel.contains("/.") || rel.hasPrefix("tests/") || rel.hasPrefix("build/") { continue }
            if url.pathExtension == "vard" || rel == "vard.lock" {
                files.append((String(rel), try String(contentsOf: url, encoding: .utf8)))
            } else if rel.hasPrefix("fonts/"), ["ttf", "otf"].contains(url.pathExtension.lowercased()) {
                fonts.append(url)
            }
        }
        files.sort { $0.0 < $1.0 }
        guard !files.isEmpty else { throw CocoaError(.fileReadNoSuchFile, userInfo: [NSFilePathErrorKey: root.path]) }
        self.files = files
        self.fonts = fonts.sorted { $0.path < $1.path }
    }

    /// The bundled app inside the main bundle (`app/` folder reference by default).
    public static func main(folder: String = "app") throws -> VardProject {
        guard let url = Bundle.main.url(forResource: folder, withExtension: nil) else {
            throw CocoaError(.fileReadNoSuchFile, userInfo: [NSFilePathErrorKey: folder])
        }
        return try VardProject(directory: url)
    }

    /// An engine for this project with its fonts and asset folder installed.
    public func makeEngine(width: Float, height: Float) -> VardEngine {
        let e = VardEngine(files: files, width: width, height: height)
        for f in fonts {
            if let d = try? Data(contentsOf: f) { e.addFont(fileName: f.lastPathComponent, data: d) }
        }
        e.setAssetDirectory(root)
        return e
    }
}
