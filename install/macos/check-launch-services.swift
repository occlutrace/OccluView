import AppKit
import Foundation
import UniformTypeIdentifiers

guard CommandLine.arguments.count == 2 else {
    fatalError("usage: check-launch-services.swift /path/to/App.app")
}

let appURL = URL(fileURLWithPath: CommandLine.arguments[1]).resolvingSymlinksInPath()
guard let appBundle = Bundle(url: appURL),
      let bundleIdentifier = appBundle.bundleIdentifier,
      let documentTypes = appBundle.infoDictionary?["CFBundleDocumentTypes"] as? [[String: Any]] else {
    fatalError("the installed application has no document type declarations")
}

func path(_ url: URL?) -> String? {
    url?.resolvingSymlinksInPath().standardizedFileURL.path
}

let expectedAppPath = path(appURL)
let workspace = NSWorkspace.shared
guard path(workspace.urlForApplication(withBundleIdentifier: bundleIdentifier)) == expectedAppPath else {
    fatalError("LaunchServices did not register \(bundleIdentifier) at \(appURL.path)")
}

var checkedExtensions = Set<String>()
for documentType in documentTypes {
    guard let rank = documentType["LSHandlerRank"] as? String,
          rank == "Default" || rank == "Alternate",
          let identifiers = documentType["LSItemContentTypes"] as? [String],
          !identifiers.isEmpty else {
        fatalError("a document type has no supported handler rank or content type")
    }

    for identifier in identifiers {
        guard let declaredType = UTType(identifier),
              let extensions = declaredType.tags[.filenameExtension],
              !extensions.isEmpty else {
            fatalError("LaunchServices cannot resolve \(identifier) to a filename extension")
        }

        for fileExtension in extensions {
            guard checkedExtensions.insert(fileExtension).inserted else {
                fatalError("the bundle declares .\(fileExtension) more than once")
            }
            guard let filenameType = UTType(filenameExtension: fileExtension),
                  filenameType.identifier == identifier else {
                fatalError(".\(fileExtension) does not resolve to \(identifier)")
            }

            let handlerURLs = workspace.urlsForApplications(toOpen: filenameType)
            let isRegistered = handlerURLs.contains { path($0) == expectedAppPath }
            let defaultURL = workspace.urlForApplication(toOpen: filenameType)
            if rank == "Default" {
                guard path(defaultURL) == expectedAppPath else {
                    fatalError(".\(fileExtension) does not select OccluView as its default handler")
                }
            } else {
                guard isRegistered else {
                    fatalError(".\(fileExtension) does not list OccluView as an alternate handler")
                }
            }

            print(".\(fileExtension): \(rank); default=\(path(defaultURL) ?? "none"); registered=\(isRegistered)")
        }
    }
}

precondition(!checkedExtensions.isEmpty, "the installed application must register file extensions")
print("LaunchServices verified \(checkedExtensions.count) file extension handler(s)")
