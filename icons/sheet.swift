import AppKit
let names = Array(CommandLine.arguments.dropFirst(2)); let out = CommandLine.arguments[1]
let cell = 360, labelH = 40, cols = 3, rows = (names.count + cols - 1) / cols
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: cols*cell, pixelsHigh: rows*(cell+labelH), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState(); NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
NSColor(white: 0.12, alpha: 1).setFill(); NSRect(x: 0, y: 0, width: cols*cell, height: rows*(cell+labelH)).fill()
for (i, n) in names.enumerated() {
  let c = i % cols, r = rows - 1 - i / cols
  let img = NSImage(contentsOfFile: n + ".png")!
  img.draw(in: NSRect(x: c*cell, y: r*(cell+labelH)+labelH, width: cell, height: cell))
  let s = NSAttributedString(string: n, attributes: [.font: NSFont.boldSystemFont(ofSize: 22), .foregroundColor: NSColor.white])
  s.draw(at: NSPoint(x: c*cell + 16, y: r*(cell+labelH) + 8))
}
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))
