// Locate displays and capture windows; no Accessibility permission needed.
import AppKit
import CoreGraphics
import Foundation

if CommandLine.arguments[1] == "display" {
    let internalDisplay = CommandLine.arguments[2] == "internal"
    let screen = NSScreen.screens.first {
        let id = $0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as! CGDirectDisplayID
        return internalDisplay ? CGDisplayIsBuiltin(id) != 0 : id == CGMainDisplayID()
    }
    if let screen {
        let id = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as! CGDirectDisplayID
        let bounds = CGDisplayBounds(id)
        let info: [String: Any] = [
            "id": id, "name": screen.localizedName, "scale": screen.backingScaleFactor,
            "x": bounds.minX, "y": bounds.minY,
            "width": bounds.width, "height": bounds.height
        ]
        let data = try JSONSerialization.data(withJSONObject: info, options: [.sortedKeys])
        print(String(decoding: data, as: UTF8.self))
    } else {
        print("null")
    }
    exit(0)
}
if CommandLine.arguments[1] == "pid" {
    let path = URL(fileURLWithPath: CommandLine.arguments[2]).resolvingSymlinksInPath().path
    if let app = NSWorkspace.shared.runningApplications.first(where: {
        $0.bundleURL?.resolvingSymlinksInPath().path == path
    }) {
        print(app.processIdentifier)
    } else {
        print("null")
    }
    exit(0)
}
if CommandLine.arguments[1] == "colours" {
    let data = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[2]))
    let bitmap = NSBitmapImageRep(data: data)!
    var colours = Set<[Int]>()
    // Ignore the title bar: an unpainted Metal surface can otherwise look settled.
    for y in stride(from: bitmap.pixelsHigh / 8, to: bitmap.pixelsHigh, by: 8) {
        for x in stride(from: 8, to: bitmap.pixelsWide, by: 8) {
            if let colour = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) {
                colours.insert([colour.redComponent, colour.greenComponent, colour.blueComponent]
                    .map { Int($0 * 255) / 16 })
            }
        }
    }
    print(colours.count)
    exit(0)
}
let pid = Int(CommandLine.arguments[2])!
let windows = CGWindowListCopyWindowInfo(
    [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID
) as? [[String: Any]] ?? []
let window = windows.first {
    ($0[kCGWindowOwnerPID as String] as? Int) == pid &&
    ($0[kCGWindowLayer as String] as? Int) == 0
}
if let window, let bounds = window[kCGWindowBounds as String] as? [String: Any] {
    let info: [String: Any] = [
        "id": window[kCGWindowNumber as String]!,
        "width": bounds["Width"]!,
        "height": bounds["Height"]!,
        "x": bounds["X"]!, "y": bounds["Y"]!
    ]
    let data = try JSONSerialization.data(withJSONObject: info, options: [.sortedKeys])
    print(String(decoding: data, as: UTF8.self))
} else {
    print("null")
}
